//! MCP authentication providers.
//!
//! Three auth modes, matching gemini-cli's MCP auth architecture:
//!
//! - `google_credentials` — Google ADC access token as Bearer
//! - `service_account_impersonation` — ADC token → IAM generateIdToken → ID token
//! - `dynamic_discovery` — OAuth PKCE via rmcp's AuthorizationManager
//!
//! Auth injection is dynamic (per-request) via `AuthInjectingClient`, a
//! `StreamableHttpClient` wrapper that resolves fresh tokens on every call
//! and delegates to an inner transport.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use http::HeaderName;
use http::HeaderValue;
use rmcp::model::ClientJsonRpcMessage;
use rmcp::transport::streamable_http_client::{
    SseError, StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
};
use sse_stream::Sse;
use tokio::sync::Mutex;

use super::config::McpServerConfig;

// ---------------------------------------------------------------------------
// Auth provider
// ---------------------------------------------------------------------------

/// Resolves `Authorization: Bearer <token>` on demand.
///
/// Tokens are cached with a 5-minute expiry buffer. The cache is per-provider
/// instance, so each MCP server gets its own token lifecycle.
pub enum McpAuthProvider {
    /// Google Application Default Credentials (ADC).
    ///
    /// Reads ADC from `~/.config/gcloud/application_default_credentials.json`
    /// (or `GOOGLE_APPLICATION_CREDENTIALS` env var) and refreshes expired
    /// access tokens via `oauth2.googleapis.com/token`.
    GoogleAdc { cached: Option<(String, Instant)> },
    /// Service account impersonation via IAM Credentials API.
    ///
    /// Exchanges an ADC access token for an ID token targeting
    /// `target_service_account`. Optionally scoped to `target_audience`
    /// (e.g. a CLIENT_ID.apps.googleusercontent.com for IAP-protected
    /// MCP servers behind Identity-Aware Proxy).
    ServiceAccountImpersonation {
        target_service_account: String,
        target_audience: Option<String>,
        cached: Option<(String, Instant)>,
    },
    /// OAuth PKCE dynamic discovery via rmcp's AuthorizationManager.
    ///
    /// Discovers OAuth endpoints from the MCP server's well-known URIs
    /// (RFC 9728), performs PKCE in a browser, and caches/refreshes tokens.
    DynamicDiscovery {
        auth_manager: Arc<Mutex<rmcp::transport::auth::AuthorizationManager>>,
    },
}

/// How long before expiry we proactively refresh.
const TOKEN_REFRESH_BUFFER: Duration = Duration::from_secs(300);

impl McpAuthProvider {
    /// Resolve the `Authorization: Bearer <token>` value.
    ///
    /// Returns `None` if no token is available (e.g. ADC not configured).
    /// Tokens are cached — call this on every request; it only hits the
    /// network when the cached token is within 5 minutes of expiry.
    pub async fn resolve_header(&mut self) -> anyhow::Result<Option<String>> {
        match self {
            Self::GoogleAdc { cached } => {
                if let Some((token, expiry)) = cached {
                    if expiry.saturating_duration_since(Instant::now()) > TOKEN_REFRESH_BUFFER {
                        return Ok(Some(token.clone()));
                    }
                }
                let adc = crate::provider::google_adc::resolve_adc_token()
                    .await?
                    .map(|t| t.access_token);
                if let Some(ref token) = adc {
                    *cached = Some((token.clone(), Instant::now() + Duration::from_secs(3600)));
                }
                Ok(adc)
            }
            Self::ServiceAccountImpersonation {
                target_service_account,
                target_audience,
                cached,
            } => {
                if let Some((token, expiry)) = cached {
                    if expiry.saturating_duration_since(Instant::now()) > TOKEN_REFRESH_BUFFER {
                        return Ok(Some(token.clone()));
                    }
                }
                let adc_token = crate::provider::google_adc::resolve_adc_token()
                    .await?
                    .map(|t| t.access_token);
                let Some(adc_token) = adc_token else {
                    return Ok(None);
                };
                let id_token = generate_id_token(
                    &adc_token,
                    target_service_account,
                    target_audience.as_deref(),
                )
                .await?;
                *cached = Some((id_token.clone(), Instant::now() + Duration::from_secs(3600)));
                Ok(Some(id_token))
            }
            Self::DynamicDiscovery { auth_manager } => {
                let token = auth_manager
                    .lock()
                    .await
                    .get_access_token()
                    .await
                    .map_err(|e| anyhow::anyhow!("OAuth token resolution failed: {e}"))?;
                Ok(Some(token))
            }
        }
    }
}

/// Build an auth provider from the server config, or `None` if no auth
/// is configured.
pub async fn build_auth_provider(
    config: &McpServerConfig,
) -> anyhow::Result<Option<McpAuthProvider>> {
    let url_config = match config {
        McpServerConfig::Url {
            auth_provider_type,
            url,
            oauth,
            target_audience,
            target_service_account,
            ..
        } => (
            auth_provider_type.as_deref(),
            url,
            oauth.as_deref(),
            target_audience.as_deref(),
            target_service_account.as_deref(),
        ),
        _ => return Ok(None),
    };

    let (auth_type, base_url, oauth_config, target_audience, target_sa) = url_config;

    match auth_type {
        Some("google_credentials") => Ok(Some(McpAuthProvider::GoogleAdc { cached: None })),
        Some("service_account_impersonation") => {
            let target_sa = target_sa
                .ok_or_else(|| {
                    anyhow::anyhow!("service_account_impersonation requires target_service_account")
                })?
                .to_string();
            Ok(Some(McpAuthProvider::ServiceAccountImpersonation {
                target_service_account: target_sa,
                target_audience: target_audience.map(|s| s.to_string()),
                cached: None,
            }))
        }
        Some("dynamic_discovery") => {
            let mut auth_manager =
                rmcp::transport::auth::AuthorizationManager::new(base_url.as_str())
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to create OAuth auth manager: {e}"))?;

            if let Some(cfg) = oauth_config {
                let parsed: OAuthClientConfig = serde_json::from_str(cfg)
                    .map_err(|e| anyhow::anyhow!("Invalid oauth config JSON: {e}"))?;
                let metadata = auth_manager
                    .discover_metadata()
                    .await
                    .map_err(|e| anyhow::anyhow!("OAuth metadata discovery failed: {e}"))?;
                auth_manager.set_metadata(metadata);
                auth_manager
                    .configure_client(parsed.into_rmcp())
                    .map_err(|e| anyhow::anyhow!("OAuth client configuration failed: {e}"))?;
            }

            let _ = auth_manager.initialize_from_store().await;
            Ok(Some(McpAuthProvider::DynamicDiscovery {
                auth_manager: Arc::new(Mutex::new(auth_manager)),
            }))
        }
        Some(other) => Err(anyhow::anyhow!("Unknown auth_provider_type: '{other}'")),
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Auth-injecting HTTP client wrapper
// ---------------------------------------------------------------------------

/// Wraps any `StreamableHttpClient` and injects a fresh `Authorization`
/// header on every request.
///
/// The inner client handles the actual HTTP transport; this layer only
/// resolves the auth token and passes it through as the `auth_header`
/// parameter that every `StreamableHttpClient` method already accepts.
#[derive(Clone)]
pub struct AuthInjectingClient<C> {
    inner: C,
    auth: Arc<Mutex<McpAuthProvider>>,
}

impl<C> AuthInjectingClient<C> {
    pub fn new(inner: C, auth: McpAuthProvider) -> Self {
        Self {
            inner,
            auth: Arc::new(Mutex::new(auth)),
        }
    }
}

impl<C: StreamableHttpClient + Sync> StreamableHttpClient for AuthInjectingClient<C> {
    type Error = C::Error;

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        let fresh = self
            .auth
            .lock()
            .await
            .resolve_header()
            .await
            .unwrap_or_else(|e| {
                tracing::warn!("MCP auth token resolution failed: {e}");
                None
            });
        let auth = fresh.or(auth_header);
        self.inner
            .post_message(uri, message, session_id, auth, custom_headers)
            .await
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        let fresh = self
            .auth
            .lock()
            .await
            .resolve_header()
            .await
            .unwrap_or_else(|e| {
                tracing::warn!("MCP auth token resolution failed: {e}");
                None
            });
        let auth = fresh.or(auth_header);
        self.inner
            .delete_session(uri, session_id, auth, custom_headers)
            .await
    }

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<
        futures::stream::BoxStream<'static, Result<Sse, SseError>>,
        StreamableHttpError<Self::Error>,
    > {
        let fresh = self
            .auth
            .lock()
            .await
            .resolve_header()
            .await
            .unwrap_or_else(|e| {
                tracing::warn!("MCP auth token resolution failed: {e}");
                None
            });
        let auth = fresh.or(auth_header);
        self.inner
            .get_stream(uri, session_id, last_event_id, auth, custom_headers)
            .await
    }
}

// ---------------------------------------------------------------------------
// SA impersonation: IAM generateIdToken
// ---------------------------------------------------------------------------

const IAM_API_URL: &str = "https://iamcredentials.googleapis.com/v1";

async fn generate_id_token(
    access_token: &str,
    service_account: &str,
    audience: Option<&str>,
) -> anyhow::Result<String> {
    let url = format!("{IAM_API_URL}/projects/-/serviceAccounts/{service_account}:generateIdToken");
    let mut body = serde_json::json!({});
    if let Some(aud) = audience {
        body["audience"] = serde_json::Value::String(aud.to_string());
        body["includeEmail"] = serde_json::Value::Bool(true);
    }
    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .bearer_auth(access_token)
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("generateIdToken failed ({status}): {text}"));
    }
    let parsed: serde_json::Value = resp.json().await?;
    parsed["token"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| {
            anyhow::anyhow!("generateIdToken response missing 'token' field: {}", parsed)
        })
}

// ---------------------------------------------------------------------------
// OAuth client config (for dynamic_discovery JSON)
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct OAuthClientConfig {
    client_id: String,
    redirect_uri: String,
    #[serde(default)]
    client_secret: Option<String>,
}

impl OAuthClientConfig {
    fn into_rmcp(self) -> rmcp::transport::auth::OAuthClientConfig {
        let mut cfg =
            rmcp::transport::auth::OAuthClientConfig::new(self.client_id, self.redirect_uri);
        cfg.client_secret = self.client_secret;
        cfg
    }
}
