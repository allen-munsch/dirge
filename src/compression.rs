#[cfg(feature = "compression")]
use anyhow::{Context as _, Result};

/// Dirge's default compression config: "lossless + tool-output windowing, no
/// output-shaping" — the A/B-validated safe default.
#[cfg(feature = "compression")]
pub fn dirge_default_config() -> llmtrim_core::config::DenseConfig {
    let mut c = llmtrim_core::config::DenseConfig::lossless();
    c.toolout = true;
    c.toolout_mode = "adaptive".to_string();
    c.serialize_flatten = true;
    c.serialize_buckets = true;
    c.json_crush = true;
    c.cache = true;
    c
}

/// Resolve a preset name to a DenseConfig. "dirge" / "default" → our
/// `dirge_default_config()`; anything else tries the upstream `DenseConfig::preset()`,
/// falling back to the dirge default on an unknown name.
#[cfg(feature = "compression")]
pub fn config_for_preset(name: &str) -> llmtrim_core::config::DenseConfig {
    if name == "dirge" || name == "default" {
        return dirge_default_config();
    }
    llmtrim_core::config::DenseConfig::preset(name).unwrap_or_else(|| dirge_default_config())
}

/// Rewrite a request body with an explicit config (the low-level entry point,
/// called from the HTTP interceptor).
#[cfg(feature = "compression")]
pub fn rewrite_with(
    body: &str,
    provider: llmtrim_core::ir::ProviderKind,
    config: &llmtrim_core::config::DenseConfig,
) -> Result<String> {
    let result = llmtrim_core::compress_with_config(body, Some(provider), config)
        .context("llmtrim-core compress_with_config failed")?;
    Ok(result.request_json)
}

#[cfg(test)]
#[cfg(feature = "compression")]
mod tests {
    use super::*;

    #[test]
    fn smoke_openai_safe() {
        let body = r#"{"model":"x","messages":[{"role":"user","content":"hi"}],"max_tokens":5}"#;
        let cfg = config_for_preset("safe");
        let out = rewrite_with(body, llmtrim_core::ir::ProviderKind::OpenAi, &cfg)
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
        let cfg = llmtrim_core::config::DenseConfig::lossless();
        // Turn off toolout so nothing changes the body.
        let mut cfg = cfg;
        cfg.toolout = false;
        let out = rewrite_with(body, llmtrim_core::ir::ProviderKind::OpenAi, &cfg)
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
        let out = rewrite_with(&body, llmtrim_core::ir::ProviderKind::OpenAi, &cfg)
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
        let out = rewrite_with(&body, llmtrim_core::ir::ProviderKind::OpenAi, &cfg)
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
