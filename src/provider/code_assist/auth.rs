/// Authentication for cloudcode-pa.googleapis.com (Google CodeAssist).
///
/// Two modes are supported:
///
/// **ADC** (default) — uses Application Default Credentials to obtain an
/// access token. This is the same flow as `google_adc::resolve_adc_token()`
/// and produces a short-lived Bearer token that CodeAssist accepts.
///
/// **OAuth PKCE** (not yet implemented) — interactive browser-based login
/// using the gcloud installed-app OAuth client. This is the flow used by
/// gemini-cli's `LOGIN_WITH_GOOGLE` auth type.
///
/// Token refresh is transparent: every call to `access_token()` re-resolves
/// ADC credentials when the cached token is near expiry.

use super::types::ClientMetadata;

pub struct CodeAssistAuth {
    /// Cached ADC access token + expiry.
    cached_token: Option<CachedToken>,
    /// Client metadata sent with loadCodeAssist calls.
    pub client_metadata: ClientMetadata,
    /// Project ID resolved from ADC or environment.
    project_id: Option<String>,
}

struct CachedToken {
    token: String,
    expires_at: std::time::Instant,
}

impl CodeAssistAuth {
    pub fn new(project_id: Option<String>) -> Self {
        Self {
            cached_token: None,
            client_metadata: ClientMetadata::default(),
            project_id,
        }
    }

    /// Returns a Bearer access token, refreshing via ADC if needed.
    pub async fn access_token(&mut self) -> Result<String, CodeAssistAuthError> {
        if let Some(cached) = &self.cached_token {
            // Refresh when within 5 minutes of expiry.
            if cached.expires_at
                > std::time::Instant::now() + std::time::Duration::from_secs(300)
            {
                return Ok(cached.token.clone());
            }
        }

        let adc = crate::provider::google_adc::resolve_adc_token()
            .await
            .map_err(|e| CodeAssistAuthError::Adc(e.to_string()))?
            .ok_or_else(|| {
                CodeAssistAuthError::Adc(
                    "ADC token not available (no metadata server or credentials?)".into(),
                )
            })?;

        // Store the resolved project_id for later use.
        if let Some(ref pid) = adc.project_id {
            self.project_id = Some(pid.clone());
        }

        self.cached_token = Some(CachedToken {
            token: adc.access_token.clone(),
            // ADC access tokens from the metadata server typically have a
            // 1-hour lifetime. We don't know the exact expiry, so assume
            // 55 minutes for a safe refresh buffer.
            expires_at: std::time::Instant::now() + std::time::Duration::from_secs(3300),
        });

        Ok(adc.access_token)
    }

    /// The project ID — either the one passed at construction or the one
    /// resolved during the most recent `access_token()` call.
    #[allow(dead_code)]
    pub fn project_id(&self) -> Option<&str> {
        self.project_id.as_deref()
    }
}

#[derive(Debug)]
pub enum CodeAssistAuthError {
    Adc(String),
}

impl std::fmt::Display for CodeAssistAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Adc(msg) => write!(f, "ADC error: {msg}"),
        }
    }
}
