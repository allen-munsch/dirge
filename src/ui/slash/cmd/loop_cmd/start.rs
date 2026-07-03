//! /loop start <prompt> — start a loop.

#[cfg(feature = "loop")]
use crate::ui::slash::c_error;
use crate::ui::slash::{SlashCtx, c_agent};

/// Parse the post-`/loop` body of `/loop start <prompt>` into
/// `(max_iterations, prompt)`.
///
/// `after` is the command text with the `/loop` prefix already removed, e.g.
/// `start fix the tests` (it still carries the `start` dispatch verb). The
/// leading `start` verb token is dropped; `--max N` may appear in any
/// position; everything else becomes the loop prompt.
#[cfg(feature = "loop")]
fn parse_loop_start(after: &str) -> Result<(Option<u32>, String), String> {
    let tokens: Vec<&str> = after.split_whitespace().collect();
    // Drop a single leading "start" verb token (the dispatch verb).
    let body: &[&str] = if tokens.first() == Some(&"start") {
        &tokens[1..]
    } else {
        &tokens[..]
    };
    let mut max_iterations: Option<u32> = Some(20);
    let mut prompt_tokens: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < body.len() {
        if body[i] == "--max" && i + 1 < body.len() {
            match body[i + 1].parse::<u32>() {
                Ok(0) => max_iterations = None,
                Ok(n) => max_iterations = Some(n),
                Err(_) => {
                    return Err(format!(
                        "invalid --max value: {} (use a positive integer, or 0 for unbounded)",
                        body[i + 1]
                    ));
                }
            }
            i += 2;
        } else {
            prompt_tokens.push(body[i]);
            i += 1;
        }
    }
    Ok((max_iterations, prompt_tokens.join(" ")))
}

pub(crate) async fn cmd_loop_start(
    ctx: &mut SlashCtx<'_>,
    _parts: &[&str],
    #[cfg(feature = "loop")] text: &str,
    #[cfg(not(feature = "loop"))] _text: &str,
) -> anyhow::Result<()> {
    #[cfg(feature = "loop")]
    {
        let after = text.trim().strip_prefix("/loop").unwrap_or("").trim_start();
        let (max_iterations, prompt) = match parse_loop_start(after) {
            Ok(v) => v,
            Err(msg) => {
                ctx.renderer.write_line(&msg, c_error())?;
                return Ok(());
            }
        };
        if prompt.is_empty() {
            ctx.renderer.write_line(
                "usage: /loop [--max N] <prompt>  (default cap: 20 iterations; --max 0 = unbounded)",
                c_error(),
            )?;
            return Ok(());
        }
        let plan_file = std::path::PathBuf::from("LOOP_PLAN.md");
        let ls = crate::extras::r#loop::LoopState::new(prompt, plan_file, max_iterations, None);
        *ctx.loop_state = Some(ls);
        let cap_msg = match max_iterations {
            Some(n) => format!(
                "loop started (max {n} iterations) — iteration 1 will run after this message"
            ),
            None => "loop started (unbounded — use /loop stop to cancel) — iteration 1 will run after this message".to_string(),
        };
        ctx.renderer.write_line(&cap_msg, c_agent())?;
    }
    #[cfg(not(feature = "loop"))]
    ctx.renderer.write_line(
        "/loop requires the 'loop' feature: cargo build --features loop",
        c_agent(),
    )?;
    Ok(())
}

#[cfg(all(test, feature = "loop"))]
mod tests {
    use super::parse_loop_start;

    #[test]
    fn strips_leading_start_verb() {
        // /loop start fix the tests  ->  prompt "fix the tests", default max 20
        let (max, prompt) = parse_loop_start("start fix the tests").unwrap();
        assert_eq!(prompt, "fix the tests");
        assert_eq!(max, Some(20));
        assert!(
            !prompt.starts_with("start"),
            "start verb must not leak into the loop prompt"
        );
    }

    #[test]
    fn keeps_second_start_in_prompt() {
        // /loop start start the server  ->  only the dispatch verb is dropped
        let (_max, prompt) = parse_loop_start("start start the server").unwrap();
        assert_eq!(prompt, "start the server");
    }

    #[test]
    fn max_before_prompt() {
        // /loop start --max 5 fix tests
        let (max, prompt) = parse_loop_start("start --max 5 fix tests").unwrap();
        assert_eq!(prompt, "fix tests");
        assert_eq!(max, Some(5));
    }

    #[test]
    fn max_zero_is_unbounded() {
        let (max, prompt) = parse_loop_start("start --max 0 fix tests").unwrap();
        assert_eq!(prompt, "fix tests");
        assert_eq!(max, None);
    }

    #[test]
    fn invalid_max_is_error() {
        assert!(parse_loop_start("start --max abc fix tests").is_err());
    }

    #[test]
    fn empty_prompt() {
        let (_max, prompt) = parse_loop_start("start").unwrap();
        assert_eq!(prompt, "");
    }
}
