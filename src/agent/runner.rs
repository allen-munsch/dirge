use futures::StreamExt;
use rig::agent::{Agent, MultiTurnStreamItem};
use rig::completion::{CompletionModel, Message};
use rig::streaming::{StreamedAssistantContent, StreamingChat};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::agent::recovery::{self, RecoveryPolicy};
use crate::event::AgentEvent;
use crate::session::{MessageRole, Session};
use crate::ui::ansi::{self, StripPolicy};

/// Per-chunk read deadline for streaming provider responses. Applied
/// to every `stream.next().await` in both the interactive and
/// `run_print` paths. The reason a finite timeout exists at all:
/// `reqwest`'s default streaming behaviour doesn't detect silently-
/// dropped TCP connections (no RST, no FIN — the socket reads block
/// forever). A finite timeout converts that into a retryable
/// `Network` error so the retry loop in `spawn_agent` can re-issue.
///
/// Original value (120s) was too aggressive for reasoning-heavy
/// models. Claude 3.7 / GPT-5 extended thinking, large tool outputs
/// being processed, and provider load spikes routinely produce
/// 2-4 minute chunk gaps that are NOT failures — the model is
/// thinking. The default is now 5 minutes; users with even longer
/// reasoning budgets can bump it via `stream_chunk_timeout_secs`
/// in config.json.
pub const DEFAULT_STREAM_CHUNK_TIMEOUT_SECS: u64 = 300;

pub struct AgentRunner {
    pub event_rx: mpsc::Receiver<AgentEvent>,
    /// Handle to the spawned tokio task. The UI calls `abort()` on interrupt
    /// so in-flight LLM calls and tool execution actually stop, rather than
    /// running to completion in the background and emitting permission
    /// prompts after the user thought they cancelled.
    pub task: JoinHandle<()>,
    /// Send a unit signal to ask the runner to stop the stream at the next
    /// safe boundary (after the current tool call's result). The runner
    /// emits `AgentEvent::Interjected` with whatever assistant text had
    /// streamed so far, and the UI is responsible for queueing the next
    /// user turn. Unbounded because the signal payload is just `()`.
    /// F20: bounded so a user who hammers the interject keybind
    /// can't fill an unbounded queue while the runner is in a long
    /// LLM call. Only the FIRST signal needs to be received — all
    /// subsequent ones are noise (the runner drains via
    /// `try_recv()` after the first wakeup). 64 is generous; if
    /// the channel is full, `try_send` silently no-ops (we already
    /// have one queued).
    pub interject_tx: mpsc::Sender<()>,
}

pub fn convert_history(session: &Session) -> Vec<Message> {
    use rig::OneOrMany;
    use rig::completion::message::AssistantContent;
    let (summary, first_kept) = session.compacted_context();
    let mut messages = Vec::new();

    if let Some(summary) = summary {
        messages.push(Message::system(format!(
            "[Previous conversation summary]\n{}",
            summary
        )));
    }

    for msg in &session.messages[first_kept..] {
        match msg.role {
            MessageRole::User => messages.push(Message::user(msg.content.to_string())),
            MessageRole::System => messages.push(Message::system(msg.content.to_string())),
            MessageRole::Assistant => {
                // Phase 3: if this assistant message has structured
                // tool calls, emit a single Assistant message with
                // text + tool_use content parts, followed by ONE
                // tool_result User message per call. The pairing
                // matches opencode's `toModelMessagesEffect`
                // (`message-v2.ts:630-899`); Anthropic + OpenAI
                // reject orphan tool_use blocks so we always emit a
                // result, marking Interrupted/Failed as error text
                // rather than skipping. Bare assistant messages
                // (no tool_calls) keep the prior simple shape.
                if msg.tool_calls.is_empty() {
                    messages.push(Message::assistant(msg.content.to_string()));
                    continue;
                }

                // Build the Assistant message's content blocks: text
                // first (if any) then each ToolCall.
                let mut parts: Vec<AssistantContent> = Vec::new();
                if !msg.content.is_empty() {
                    parts.push(AssistantContent::text(msg.content.to_string()));
                }
                for tc in &msg.tool_calls {
                    parts.push(AssistantContent::tool_call(
                        tc.id.clone(),
                        tc.name.clone(),
                        tc.args.clone(),
                    ));
                }
                // OneOrMany::many requires at least one element; we
                // always have at least one ToolCall here since
                // tool_calls is non-empty.
                let content = if parts.len() == 1 {
                    OneOrMany::one(parts.pop().unwrap())
                } else {
                    OneOrMany::many(parts).expect("non-empty parts vec")
                };
                messages.push(Message::Assistant { id: None, content });

                // One User tool_result per call. State maps to:
                //  Completed  → result text verbatim
                //  Interrupted → "[Tool execution was interrupted]"
                //  Failed     → "[Tool error: <message>]"
                for tc in &msg.tool_calls {
                    let body = match &tc.state {
                        crate::session::ToolCallState::Completed { result } => result.clone(),
                        crate::session::ToolCallState::Interrupted => {
                            "[Tool execution was interrupted]".to_string()
                        }
                        crate::session::ToolCallState::Failed { error } => {
                            format!("[Tool error: {}]", error)
                        }
                    };
                    messages.push(Message::tool_result(tc.id.clone(), body));
                }
            }
        }
    }

    messages
}
/// dirge-rmk: emit one stream-json event line to stdout. NDJSON shape
/// matches Claude Code so tooling written against `claude --print
/// --output-format stream-json` works against dirge unchanged.
fn emit_stream_json_event(value: serde_json::Value) {
    if let Ok(s) = serde_json::to_string(&value) {
        println!("{}", s);
        let _ = std::io::Write::flush(&mut std::io::stdout());
    }
}

pub async fn run_print<M, P>(
    agent: &Agent<M, P>,
    prompt: &str,
    max_turns: usize,
    chunk_timeout: std::time::Duration,
    output_format: crate::cli::OutputFormat,
) -> anyhow::Result<String>
where
    M: CompletionModel + 'static,
    M::StreamingResponse: Send + Sync + Unpin + Clone + 'static,
    P: rig::agent::PromptHook<M> + 'static,
{
    let start_instant = std::time::Instant::now();
    let session_id = uuid_v4_simple();
    let mut num_turns: u32 = 0;

    // Plugin `on-prompt` dispatch. Headless modes (--print, --loop)
    // previously skipped this — plugins that mutate the user prompt
    // or block it never fired in CI/script contexts.
    let effective_prompt: String = {
        #[cfg(feature = "plugin")]
        {
            if let Some(pm_arc) = crate::plugin::hook::global() {
                let mut mgr = pm_arc.lock().unwrap_or_else(|e| e.into_inner());
                resolve_prompt_with_hooks(prompt, &mut mgr)
            } else {
                prompt.to_string()
            }
        }
        #[cfg(not(feature = "plugin"))]
        {
            prompt.to_string()
        }
    };
    // For Json / StreamJson modes the assistant text is BUFFERED
    // (never streamed inline to stdout) so the JSON envelope is the
    // only thing the user sees on stdout. Text mode keeps the prior
    // streaming behavior.
    let suppress_inline = !matches!(output_format, crate::cli::OutputFormat::Text);

    // StreamJson init event — fires once at startup so downstream
    // tools can pick up cwd/session/model before any turns stream.
    // Ported from maki print.rs:67-75 (InitEvent shape).
    if matches!(output_format, crate::cli::OutputFormat::StreamJson) {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        emit_stream_json_event(serde_json::json!({
            "type": "system",
            "subtype": "init",
            "cwd": cwd,
            "session_id": session_id,
            "tools": Vec::<String>::new(),
            "model": "",
        }));
    }
    // Retry loop. Print mode (`dirge --print "..."`) is commonly used
    // in scripts and CI where a single transient 502 or rate-limit
    // would otherwise turn a 5-line shell snippet into a flaky one.
    // Use the same RecoveryPolicy as the interactive path.
    //
    // Caveat: we only retry when NO bytes of the response have been
    // emitted to stdout yet. Once a byte is out, retrying would
    // duplicate visible output — better to surface the error and let
    // the script decide whether to re-run. This matches what
    // opencode does for its non-interactive path.
    let policy = RecoveryPolicy::default();
    let mut attempts: usize = 0;
    loop {
        let mut stream = agent
            .stream_chat(effective_prompt.clone(), Vec::<Message>::new())
            .multi_turn(max_turns)
            .await;

        let mut full_response = String::new();
        let mut had_output = false;
        let mut stream_error: Option<String> = None;

        loop {
            let item = match tokio::time::timeout(chunk_timeout, stream.next()).await {
                Ok(Some(item)) => item,
                Ok(None) => break,
                Err(_) => {
                    stream_error = Some(format!(
                        "stream chunk timed out after {}s (provider stalled or connection silently dropped) — bump `stream_chunk_timeout_secs` in config.json if your model has long reasoning gaps",
                        chunk_timeout.as_secs(),
                    ));
                    break;
                }
            };
            match item {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    text,
                ))) => {
                    full_response.push_str(&text.text);
                    if !suppress_inline {
                        let safe = ansi::strip_controls(&text.text, StripPolicy::KEEP_NEWLINE);
                        print!("{safe}");
                        let _ = std::io::Write::flush(&mut std::io::stdout());
                    }
                    had_output = true;
                }
                Ok(MultiTurnStreamItem::StreamAssistantItem(
                    StreamedAssistantContent::Reasoning(r),
                )) => {
                    if !suppress_inline {
                        // Json / StreamJson modes: reasoning is the
                        // model's internal thinking — not part of the
                        // user-visible result. Suppressing keeps the
                        // JSON output clean of chain-of-thought
                        // noise.
                        let display = r.display_text();
                        let safe = ansi::strip_controls(&display, StripPolicy::KEEP_NEWLINE);
                        eprint!("{safe}");
                        let _ = std::io::Write::flush(&mut std::io::stderr());
                    }
                }
                Ok(MultiTurnStreamItem::FinalResponse(_)) => break,
                Ok(_) => {}
                Err(e) => {
                    stream_error = Some(e.to_string());
                    break;
                }
            }
        }

        if let Some(msg) = stream_error {
            let kind = recovery::classify_error(&msg);
            if !had_output && policy.should_retry(attempts, kind) {
                let delay = policy.backoff_duration_for_msg(attempts, &msg);
                eprintln!(
                    "(retry {}/{} in {:.1}s — {:?})",
                    attempts + 1,
                    policy.max_retries(),
                    delay.as_secs_f64(),
                    kind,
                );
                tokio::time::sleep(delay).await;
                attempts += 1;
                continue;
            }
            // Either we already wrote bytes to stdout (can't safely
            // retry without duplicating) or the retry policy says
            // give up. Newline-terminate any in-flight output before
            // the error so the diagnostic doesn't share a line with
            // half a response.
            if had_output {
                println!();
            }
            eprintln!("Error: {}", msg);
            return Err(anyhow::anyhow!("{}", msg));
        }

        // dirge-rmk: turn complete. Bump turn counter; emit per-format
        // closing payload. Ported from maki print.rs:51-64
        // (`PrintResult`) and the StreamJson assistant event shape.
        num_turns += 1;

        // Plugin `on-response` + `on-complete` + `prepare-next-run`
        // dispatch. Headless modes previously skipped these. The
        // on-response hook can replace the final response via
        // harness/replace-result — used (for example) by a
        // formatting plugin that wraps the agent's output in a
        // structured envelope. Errors go to stderr; we keep the
        // run going so the user still sees the unmodified response.
        //
        // NOTE: text mode has already streamed full_response to
        // stdout BYTE BY BYTE during the loop above — a
        // replace-result mutation can't roll those bytes back, so
        // text mode prints the mutated tail after a separator
        // rather than silently dropping the upstream content. JSON
        // / StreamJson modes buffer (`suppress_inline`), so the
        // mutation cleanly replaces.
        #[cfg(feature = "plugin")]
        if let Some(pm_arc) = crate::plugin::hook::global() {
            let mut mgr = pm_arc.lock().unwrap_or_else(|e| e.into_inner());
            let result = apply_response_hooks(&full_response, &mut mgr);
            if let Some(replacement) = result.replacement {
                if suppress_inline {
                    full_response = replacement;
                } else {
                    // Text mode already streamed the original — show
                    // the replacement on a new line with a marker so
                    // the user understands what happened.
                    println!();
                    println!("[plugin replace-result]");
                    let safe = ansi::strip_controls(&replacement, StripPolicy::KEEP_NEWLINE);
                    println!("{safe}");
                    full_response = replacement;
                }
            }
            // `prepare-next-run`'s `set-next-model` value (if any) is
            // left in `PluginManager` for the caller to drain. `--loop`
            // rebuilds the agent on it; `--print` warns and ignores.
        }

        match output_format {
            crate::cli::OutputFormat::Text => {
                println!();
            }
            crate::cli::OutputFormat::Json => {
                // Single Claude-shaped result object. `total_cost_usd`
                // is 0.0 until provider cost plumbing lands.
                let result = serde_json::json!({
                    "type": "result",
                    "subtype": "success",
                    "is_error": false,
                    "duration_ms": start_instant.elapsed().as_millis() as u64,
                    "num_turns": num_turns,
                    "result": full_response.clone(),
                    "session_id": session_id,
                    "total_cost_usd": 0.0,
                });
                if let Ok(s) = serde_json::to_string(&result) {
                    println!("{}", s);
                }
            }
            crate::cli::OutputFormat::StreamJson => {
                // Per-turn assistant event + closing result event.
                emit_stream_json_event(serde_json::json!({
                    "type": "assistant",
                    "message": {
                        "role": "assistant",
                        "content": [{"type": "text", "text": full_response.clone()}],
                    },
                    "session_id": session_id,
                }));
                emit_stream_json_event(serde_json::json!({
                    "type": "result",
                    "subtype": "success",
                    "is_error": false,
                    "duration_ms": start_instant.elapsed().as_millis() as u64,
                    "num_turns": num_turns,
                    "result": full_response.clone(),
                    "session_id": session_id,
                    "total_cost_usd": 0.0,
                }));
            }
        }
        return Ok(full_response);
    }
}

/// Generate a UUIDv4-shaped session id without pulling the `uuid`
/// crate (dirge already has enough deps). Random bytes via system
/// time + thread id seeded into a small xorshift.
fn uuid_v4_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let pid = std::process::id() as u64;
    let mut state = nanos.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(pid);
    let mut bytes = [0u8; 16];
    for chunk in bytes.chunks_mut(8) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let words = state.to_le_bytes();
        chunk.copy_from_slice(&words[..chunk.len()]);
    }
    // Set version (4) + variant (10) bits per RFC 4122.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    )
}

/// Outcome of the post-response plugin dispatch sequence
/// (`on-response` → `on-complete` → `prepare-next-run`).
///
/// Note: `next_model` is intentionally NOT included here. The
/// `prepare-next-run` hook stores its value in [`PluginManager`]; the
/// caller of `run_print` (e.g. `main.rs`'s `--loop` driver) drains it
/// via `take_pending_next_model()` AFTER `run_print` returns. That
/// keeps the choice of how to react (warn-and-ignore in `--print`,
/// rebuild agent in `--loop`) in the caller's hands and out of the
/// runner.
#[cfg(feature = "plugin")]
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ResponseHookResult {
    /// `Some(text)` when a plugin called `harness/replace-result` to
    /// substitute the agent's response. Caller decides how to surface
    /// it (text mode prints with a marker since the original already
    /// streamed; JSON modes substitute cleanly).
    pub replacement: Option<String>,
}

/// Resolve `prompt` through the `on-prompt` hook chain:
///
///   1. Dispatch `on-prompt` → join results as a "hint" prepended
///      to the prompt
///   2. `harness/request-prompt` → if set, replaces the hint
///   3. `harness/replace-prompt` → if set, fully replaces the prompt
///
/// Errors from plugin code are surfaced to stderr so the user's
/// stdout (the structured `--print` result) stays clean.
#[cfg(feature = "plugin")]
pub(crate) fn resolve_prompt_with_hooks(
    prompt: &str,
    mgr: &mut crate::plugin::PluginManager,
) -> String {
    let janet_ctx = format!(
        "@{{:prompt \"{}\"}}",
        crate::plugin::escape_janet_string(prompt)
    );
    let mut hint: Option<String> = match mgr.dispatch("on-prompt", &janet_ctx) {
        Ok(results) if !results.is_empty() => Some(results.join("\n")),
        Ok(_) => None,
        Err(e) => {
            eprintln!("[plugin] on-prompt error: {e}");
            None
        }
    };
    if let Some(pending) = mgr.take_pending_prompt() {
        hint = Some(pending);
    }
    let replace = mgr.take_pending_prompt_replace();
    if let Some(rep) = replace {
        rep
    } else if let Some(h) = hint {
        format!("{}\n\n{}", h, prompt)
    } else {
        prompt.to_string()
    }
}

/// Run the post-response hook chain: `on-response` → record store →
/// `on-complete` → `prepare-next-run`. Returns the replacement (if
/// any). The `set-next-model` value, if any, is left in
/// [`PluginManager`] for the caller to drain via
/// `take_pending_next_model()`.
#[cfg(feature = "plugin")]
pub(crate) fn apply_response_hooks(
    response: &str,
    mgr: &mut crate::plugin::PluginManager,
) -> ResponseHookResult {
    let janet_ctx = format!(
        "@{{:response \"{}\"}}",
        crate::plugin::escape_janet_string(response)
    );
    if let Err(e) = mgr.dispatch("on-response", &janet_ctx) {
        eprintln!("[plugin] on-response error: {e}");
    }
    mgr.store_response(response);
    let replacement = mgr.take_pending_replace_result();
    if let Err(e) = mgr.dispatch("on-complete", "@{}") {
        eprintln!("[plugin] on-complete error: {e}");
    }
    if let Err(e) = mgr.dispatch("prepare-next-run", "@{}") {
        eprintln!("[plugin] prepare-next-run error: {e}");
    }
    ResponseHookResult { replacement }
}

#[cfg(all(test, feature = "plugin"))]
mod plugin_hook_tests {
    use super::*;
    use crate::plugin::PluginManager;

    /// on-prompt result is joined with the original prompt as a
    /// "hint" prefix. Demonstrates the simplest plugin-mutates-input
    /// flow: a code-style hint that always precedes the user prompt.
    #[test]
    fn resolve_prompt_prepends_on_prompt_hint() {
        let mut mgr = PluginManager::try_new().unwrap();
        mgr.eval(r#"(defn style-hint [ctx] "ALWAYS USE TYPESCRIPT")"#)
            .unwrap();
        mgr.register("on-prompt", "style-hint");
        let out = resolve_prompt_with_hooks("write a function", &mut mgr);
        assert!(out.contains("ALWAYS USE TYPESCRIPT"));
        assert!(out.contains("write a function"));
        assert!(
            out.find("ALWAYS USE TYPESCRIPT").unwrap() < out.find("write a function").unwrap(),
            "hint must come before the prompt"
        );
    }

    /// harness/request-prompt overrides the dispatch result. Used by
    /// plugins that want full control: they may run logic in the
    /// hook AND emit a queue-style replacement.
    #[test]
    fn resolve_prompt_request_prompt_overrides_hint() {
        let mut mgr = PluginManager::try_new().unwrap();
        mgr.eval(
            r#"(defn override [ctx]
                 (harness/request-prompt "from-request-prompt")
                 "from-dispatch")"#,
        )
        .unwrap();
        mgr.register("on-prompt", "override");
        let out = resolve_prompt_with_hooks("original", &mut mgr);
        // The "from-dispatch" hint is discarded once
        // request-prompt was set — same precedence as the UI path.
        assert!(out.contains("from-request-prompt"));
        assert!(out.contains("original"));
        assert!(!out.contains("from-dispatch"));
    }

    /// harness/replace-prompt fully substitutes the prompt — the
    /// original text is not seen by the LLM at all.
    #[test]
    fn resolve_prompt_replace_prompt_substitutes_entirely() {
        let mut mgr = PluginManager::try_new().unwrap();
        mgr.eval(
            r#"(defn replace [ctx]
                 (harness/replace-prompt "ENTIRELY NEW PROMPT")
                 nil)"#,
        )
        .unwrap();
        mgr.register("on-prompt", "replace");
        let out = resolve_prompt_with_hooks("user typed this", &mut mgr);
        assert_eq!(out, "ENTIRELY NEW PROMPT");
        assert!(!out.contains("user typed this"));
    }

    /// No plugins / nil result: prompt passes through untouched.
    #[test]
    fn resolve_prompt_no_hook_passthrough() {
        let mut mgr = PluginManager::try_new().unwrap();
        let out = resolve_prompt_with_hooks("just this", &mut mgr);
        assert_eq!(out, "just this");
    }

    /// on-response can mutate the final response via
    /// harness/replace-result. Used by formatting / wrapping
    /// plugins that produce structured output around the agent's
    /// text.
    #[test]
    fn apply_response_hooks_replace_result() {
        let mut mgr = PluginManager::try_new().unwrap();
        mgr.eval(
            r#"(defn wrap [ctx]
                 (harness/replace-result "WRAPPED")
                 nil)"#,
        )
        .unwrap();
        mgr.register("on-response", "wrap");
        let result = apply_response_hooks("raw response", &mut mgr);
        assert_eq!(result.replacement.as_deref(), Some("WRAPPED"));
        // next_model is not part of ResponseHookResult; it's left in
        // the manager. Verify it wasn't set as a side-effect of the
        // wrap hook.
        assert_eq!(mgr.take_pending_next_model(), None);
    }

    /// prepare-next-run can set the next model. The runner does NOT
    /// drain it — the caller (e.g. `run_headless_loop`) is responsible
    /// for `take_pending_next_model()`.
    #[test]
    fn apply_response_hooks_set_next_model_left_in_manager() {
        let mut mgr = PluginManager::try_new().unwrap();
        mgr.eval(
            r#"(defn pick-model [ctx]
                 (harness/set-next-model "claude-opus-4-7")
                 nil)"#,
        )
        .unwrap();
        mgr.register("prepare-next-run", "pick-model");
        let _ = apply_response_hooks("ok", &mut mgr);
        assert_eq!(
            mgr.take_pending_next_model().as_deref(),
            Some("claude-opus-4-7")
        );
    }

    /// No plugins / no hooks fired: response passes through with
    /// no replacement and no next-model.
    #[test]
    fn apply_response_hooks_no_hooks_passthrough() {
        let mut mgr = PluginManager::try_new().unwrap();
        let result = apply_response_hooks("ok", &mut mgr);
        assert_eq!(result, ResponseHookResult::default());
    }
}
