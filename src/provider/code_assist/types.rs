// CodeAssist API types — mirrors gemini-cli's code_assist/types.ts and converter.ts.
//
// The CodeAssist protocol wraps standard Gemini/Vertex AI request/response payloads
// in an envelope that adds project routing, user-prompt identification, and credit
// tracking. The inner payloads are transparently forwarded — we use serde_json::Value
// rather than duplicating rig-core's Gemini types.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Envelope types for content generation
// ---------------------------------------------------------------------------

/// Request envelope for cloudcode-pa.googleapis.com/streamGenerateContent and /generateContent.
#[derive(Debug, Clone, Serialize)]
pub struct CaGenerateContentRequest {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_prompt_id: Option<String>,
    /// Inner Gemini/Vertex AI request — forwarded as-is.
    pub request: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled_credit_types: Option<Vec<String>>,
}

/// Response envelope from cloudcode-pa.googleapis.com.
///
/// On a successful generation the `response` field carries the standard Gemini
/// `GenerateContentResponse` JSON.  `traceId` maps to `responseId` downstream.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct CaGenerateContentResponse {
    pub response: Option<serde_json::Value>,
    #[serde(rename = "traceId")]
    pub trace_id: Option<String>,
    #[serde(rename = "consumedCredits")]
    pub consumed_credits: Option<Vec<Credit>>,
    #[serde(rename = "remainingCredits")]
    pub remaining_credits: Option<Vec<Credit>>,
}

/// A credit balance line item (same shape in LoadCodeAssist and GenerateContent responses).
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct Credit {
    #[serde(rename = "creditType")]
    pub credit_type: String,
    #[serde(rename = "creditAmount")]
    /// String-encoded int64 to avoid precision loss.
    pub credit_amount: String,
}

// ---------------------------------------------------------------------------
// LoadCodeAssist — project discovery and eligibility check
// ---------------------------------------------------------------------------

/// Sent on first use to discover the user's project and tier.
#[derive(Debug, Clone, Serialize)]
#[allow(dead_code, non_snake_case)]
pub struct LoadCodeAssistRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cloudaicompanionProject: Option<String>,
    pub metadata: ClientMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>, // "FULL_ELIGIBILITY_CHECK" | "HEALTH_CHECK"
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct LoadCodeAssistResponse {
    /// The Cloud AI Companion project number (numeric string, e.g. "123456789").
    #[serde(rename = "cloudaicompanionProject")]
    pub cloud_ai_companion_project: Option<String>,
    /// Human-readable project name.
    #[serde(rename = "cloudaicompanionProjectName")]
    pub cloud_ai_companion_project_name: Option<String>,
    /// Whether the user is eligible for CodeAssist.
    #[serde(rename = "eligible")]
    pub eligible: Option<bool>,
    /// Tier name — "free-tier", "standard-tier", "legacy-tier", etc.
    #[serde(rename = "tierId")]
    pub tier_id: Option<String>,
    /// Credits available (by credit type).
    #[serde(rename = "availableCredits")]
    pub available_credits: Option<Vec<Credit>>,
}

// ---------------------------------------------------------------------------
// Client metadata — sent as part of every LoadCodeAssist request.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ClientMetadata {
    #[serde(rename = "ideType")]
    pub ide_type: String,
    #[serde(rename = "ideVersion")]
    pub ide_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(rename = "updateChannel")]
    pub update_channel: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "duetProject")]
    pub duet_project: Option<String>,
    #[serde(rename = "pluginType")]
    pub plugin_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "ideName")]
    pub ide_name: Option<String>,
}

impl Default for ClientMetadata {
    fn default() -> Self {
        Self {
            ide_type: "GEMINI_CLI".into(),
            ide_version: env!("CARGO_PKG_VERSION").into(),
            platform: Some(current_platform()),
            update_channel: "stable".into(),
            duet_project: None,
            plugin_type: "GEMINI".into(),
            ide_name: Some("dirge".into()),
        }
    }
}

fn current_platform() -> String {
    let arch = std::env::consts::ARCH;
    match (std::env::consts::OS, arch) {
        ("linux", "x86_64") => "LINUX_AMD64",
        ("linux", "aarch64") => "LINUX_ARM64",
        ("macos", "x86_64") => "DARWIN_AMD64",
        ("macos", "aarch64") => "DARWIN_ARM64",
        ("windows", "x86_64") => "WINDOWS_AMD64",
        _ => "PLATFORM_UNSPECIFIED",
    }
    .into()
}

// ---------------------------------------------------------------------------
// Count-tokens envelope (optional, for parity with gemini-cli)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub struct CaCountTokenRequest {
    pub request: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct CaCountTokenResponse {
    #[serde(rename = "totalTokens")]
    pub total_tokens: Option<u64>,
}
