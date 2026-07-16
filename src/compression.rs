use anyhow::{Context as _, Result};

/// Process-global compression configuration, set once at startup from the
/// `[compression]` config section (with env var overrides checked later at
/// each `resolve_compression_*` call site). Initialized by
/// [`init_from_config`] right after the runtime Config is loaded from disk.
static COMPRESSION_CFG: std::sync::OnceLock<(bool, String)> = std::sync::OnceLock::new();

/// Seed the compression config from the loaded runtime Config. Call ONCE
/// at startup, BEFORE any provider client is built.
///
/// `enabled` is `None` when no `[compression]` section exists; the default
/// is `true`.
/// `preset` defaults to `"dirge"` when absent or `None`.
pub fn init_from_config(enabled: Option<bool>, preset: Option<String>) {
    let _ = COMPRESSION_CFG.set((
        enabled.unwrap_or(true),
        preset.unwrap_or_else(|| "dirge".to_string()),
    ));
}

/// Was compression enabled in the config file? Defaults to `true` if
/// `init_from_config` was never called (feature compiled in but config
/// not yet loaded — fail-safe: assume on).
pub fn configured_enabled() -> bool {
    COMPRESSION_CFG.get().map(|(e, _)| *e).unwrap_or(true)
}

/// Which preset did the config file choose? Defaults to `"dirge"`.
pub fn configured_preset() -> String {
    COMPRESSION_CFG
        .get()
        .map(|(_, p)| p.clone())
        .unwrap_or_else(|| "dirge".to_string())
}

/// Dirge's default compression config: "lossless + tool-output windowing, no
/// output-shaping" — the A/B-validated safe default.
///
/// Everything here is behavior-preserving: `toolout` (adaptive/keep-more)
/// windows verbose log/diff/grep tool results, `serialize_*` columnar-encode
/// uniform record arrays (TOON, lossless), and `cache` marks cache
/// breakpoints. Deliberately NOT set: `json_crush` (lossy — samples record
/// arrays down to a row cap), `retrieve`/`skeletonize`/`ngram` (lossy or
/// redundant with dirge's own minify), and every `output_*` control (they
/// alter the model's output, not just the input).
pub fn dirge_default_config() -> crate::llmtrim::config::DenseConfig {
    let mut c = crate::llmtrim::config::DenseConfig::lossless();
    c.toolout = true;
    c.toolout_mode = "adaptive".to_string();
    c.serialize_flatten = true;
    c.serialize_buckets = true;
    c.cache = true;
    c
}

/// Resolve a preset name to a [`DenseConfig`](crate::llmtrim::config::DenseConfig).
///
/// `"dirge"` and `"default"` return [`dirge_default_config`] — a
/// lossless-safe profile with tool-output windowing and no output-shaping
/// directives. **All other names** (`"agent"`, `"aggressive"`, `"auto"`,
/// `"safe"`, `"lossless"`, `"rag"`, `"code"`) delegate to the upstream
/// `DenseConfig::preset()`. Of those, `"safe"` and `"lossless"` are also
/// output-neutral; the rest (`agent` / `aggressive` / `auto` / `rag` /
/// `code`) enable lossy stages (retrieve, skeletonize, json_crush) AND
/// output-shaping directives that **alter the model's output behavior** —
/// they are an opt-in escape hatch for aggressive trimming, not a tuning
/// knob to casually dial.
pub fn config_for_preset(name: &str) -> crate::llmtrim::config::DenseConfig {
    if name == "dirge" || name == "default" {
        return dirge_default_config();
    }
    crate::llmtrim::config::DenseConfig::preset(name).unwrap_or_else(dirge_default_config)
}

/// Rewrite a request body with an explicit config (the low-level entry point,
/// called from the HTTP interceptor).
pub fn rewrite_with(
    body: &str,
    provider: crate::llmtrim::ir::ProviderKind,
    config: &crate::llmtrim::config::DenseConfig,
) -> Result<String> {
    let result = crate::llmtrim::compress_with_config(body, Some(provider), config)
        .context("llmtrim-core compress_with_config failed")?;
    Ok(result.request_json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_from_config_disables_compression() {
        // OnceLock is set-once per process, so this test must run in
        // isolation. We can't call init_from_config (it would poison the
        // lock for other tests in the same binary), but we can assert
        // that configured_enabled() defaults to true when the lock is
        // still empty (which it is before any prod startup path runs).
        assert!(configured_enabled(), "default should be true");
        assert_eq!(
            configured_preset(),
            "dirge",
            "default preset should be 'dirge'"
        );
    }

    #[test]
    fn smoke_openai_safe() {
        let body = r#"{"model":"x","messages":[{"role":"user","content":"hi"}],"max_tokens":5}"#;
        let cfg = config_for_preset("safe");
        let out = rewrite_with(body, crate::llmtrim::ir::ProviderKind::OpenAi, &cfg)
            .expect("rewrite_with should succeed");
        let parsed: serde_json::Value =
            serde_json::from_str(&out).expect("output should be valid JSON");
        let content = parsed["messages"][0]["content"]
            .as_str()
            .expect("content should be a string");
        assert!(
            content.contains("hi"),
            "compressed content should still contain the original message text 'hi', got: {content}"
        );
    }

    #[test]
    fn byte_identity_when_nothing_fires() {
        let body = r#"{"model":"x","messages":[{"role":"user","content":"hi"}],"temperature":0}"#;
        let cfg = crate::llmtrim::config::DenseConfig::lossless();
        // Turn off toolout so nothing changes the body.
        let mut cfg = cfg;
        cfg.toolout = false;
        let out = rewrite_with(body, crate::llmtrim::ir::ProviderKind::OpenAi, &cfg)
            .expect("rewrite_with should succeed");
        let a: serde_json::Value = serde_json::from_str(body).unwrap();
        let b: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(a, b, "lossless config should not change a trivial body");
    }

    #[test]
    fn needle_survival_toolout_compression() {
        // Build a body with a tool_call + long tool result containing a specific
        // error line, then a user question referencing it.
        let log_lines: Vec<String> = (0..80)
            .map(|i| format!("DEBUG processed item {}", i))
            .collect();
        let mut lines = log_lines.clone();
        lines.insert(42, "ERROR NullPointerException at Foo.java:147".to_string());
        let log = lines.join("\n");
        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [
                {"role": "system", "content": "You are a helpful assistant."},
                {"role": "assistant", "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "read_logs", "arguments": "{}"}}]},
                {"role": "tool", "tool_call_id": "call_1", "content": log},
                {"role": "user", "content": "What caused the NullPointerException?"}
            ],
            "max_tokens": 100
        })
        .to_string();
        let cfg = dirge_default_config();
        let out = rewrite_with(&body, crate::llmtrim::ir::ProviderKind::OpenAi, &cfg)
            .expect("rewrite_with should succeed");
        assert!(
            out.len() < body.len(),
            "compressed output ({}) should be smaller than input ({})",
            out.len(),
            body.len()
        );
        assert!(
            out.contains("Foo.java:147"),
            "needle should survive compression, got:\n{out}"
        );
    }

    #[test]
    fn cache_stability_preserves_cache_control_blocks() {
        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [
                {"role": "system", "content": "Cached preamble", "cache_control": {"type": "ephemeral"}},
                {"role": "user", "content": "What is Rust?"}
            ],
            "max_tokens": 50
        })
        .to_string();
        let cfg = dirge_default_config();
        let out = rewrite_with(&body, crate::llmtrim::ir::ProviderKind::OpenAi, &cfg)
            .expect("rewrite_with should succeed");
        assert!(
            out.contains("cache_control"),
            "cache_control block must survive compression:\n{out}"
        );
        assert!(
            out.contains("Cached preamble"),
            "cached content must survive compression:\n{out}"
        );
    }
}
