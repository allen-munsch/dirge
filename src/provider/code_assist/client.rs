// CodeAssist HTTP transport — wraps reqwest::Client and implements
// rig-core's HttpClientExt so it can be injected as the HTTP transport
// for rig-core's Gemini client builder.
//
// The wrapper intercepts Gemini API requests, rewrites URLs from
// /v1beta/models/{model}:method → /v1internal:method on
// cloudcode-pa.googleapis.com, and wraps/unwraps the CodeAssist
// envelope (CaGenerateContentRequest / CaGenerateContentResponse).

use std::io;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use futures::StreamExt;
use http::HeaderValue;
use rig::http_client::{
    Error, HttpClientExt, LazyBody, MultipartForm, Response, Result as HttpClientResult,
    StreamingResponse,
};
use rig_core::wasm_compat::{WasmCompatSend, WasmCompatSendStream};
use serde_json::Value;
use tokio::sync::Mutex;

use super::auth::{CodeAssistAuth, CodeAssistAuthError};
use super::types::CaGenerateContentRequest;

static CODE_ASSIST_BASE: &str = "https://cloudcode-pa.googleapis.com";
static CODE_ASSIST_VERSION: &str = "v1internal";

pub struct CodeAssistHttpClient {
    inner: reqwest::Client,
    auth: Arc<Mutex<CodeAssistAuth>>,
    model: String,
}

impl CodeAssistHttpClient {
    pub fn new(inner: reqwest::Client, auth: CodeAssistAuth, model: String) -> Self {
        Self {
            inner,
            auth: Arc::new(Mutex::new(auth)),
            model,
        }
    }

    pub async fn setup(&self) -> Result<Option<String>, CodeAssistAuthError> {
        let mut auth = self.auth.lock().await;
        let token = auth.access_token().await?;
        let client_metadata = auth.client_metadata.clone();
        drop(auth);

        let req_body = serde_json::json!({
            "metadata": client_metadata,
            "mode": "FULL_ELIGIBILITY_CHECK",
        });

        let url = format!("{CODE_ASSIST_BASE}/{CODE_ASSIST_VERSION}:loadCodeAssist");
        let response = self
            .inner
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {token}"))
            .json(&req_body)
            .send()
            .await
            .map_err(|e| CodeAssistAuthError::Adc(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(CodeAssistAuthError::Adc(format!(
                "loadCodeAssist failed with status {status}"
            )));
        }

        let body: Value = response
            .json()
            .await
            .map_err(|e| CodeAssistAuthError::Adc(e.to_string()))?;
        Ok(body
            .get("cloudaicompanionProject")
            .and_then(|v| v.as_str())
            .map(str::to_string))
    }

    async fn get_token(&self) -> Result<String, Error> {
        let mut guard = self.auth.lock().await;
        guard
            .access_token()
            .await
            .map_err(|e| Error::Instance(Box::new(io::Error::new(io::ErrorKind::Other, e.to_string()))))
    }

    fn rewrite_request<T: Into<Bytes>>(
        &self,
        req: http::Request<T>,
    ) -> Result<http::Request<Vec<u8>>, Error> {
        let (parts, body) = req.into_parts();

        let method = parts
            .uri
            .path()
            .split(':')
            .last()
            .ok_or_else(|| {
                Error::Instance(Box::new(io::Error::new(
                    io::ErrorKind::Other,
                    "Cannot extract CodeAssist method from URI path",
                )))
            })?;

        let is_stream = parts.uri.query().map_or(false, |q| q.contains("alt=sse"));

        let mut url = format!("{CODE_ASSIST_BASE}/{CODE_ASSIST_VERSION}:{method}");
        if is_stream {
            url.push_str("?alt=sse");
        }

        let body_bytes: Bytes = body.into();
        let inner_request: Value = serde_json::from_slice(&body_bytes)
            .map_err(|e| Error::Instance(Box::new(e)))?;

        let ca_request = CaGenerateContentRequest {
            model: self.model.clone(),
            project: None,
            user_prompt_id: None,
            request: inner_request,
            enabled_credit_types: None,
        };
        let ca_body = serde_json::to_vec(&ca_request)
            .map_err(|e| Error::Instance(Box::new(e)))?;

        let mut request = http::Request::builder()
            .method(parts.method)
            .uri(url)
            .body(ca_body)
            .map_err(Error::Protocol)?;

        request.headers_mut().insert(
            http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );

        Ok(request)
    }

    fn unwrap_response(body: Bytes) -> Result<Bytes, Error> {
        let ca_response: Value = serde_json::from_slice(&body)
            .map_err(|e| Error::Instance(Box::new(e)))?;

        if let Some(inner) = ca_response.get("response") {
            serde_json::to_vec(inner)
                .map(Bytes::from)
                .map_err(|e| Error::Instance(Box::new(e)))
        } else {
            Ok(body)
        }
    }
}

// ---------------------------------------------------------------------------
// HttpClientExt — delegates to inner reqwest::Client after URL/body rewrite.
// ---------------------------------------------------------------------------

impl HttpClientExt for CodeAssistHttpClient {
    fn send<T, U>(
        &self,
        req: http::Request<T>,
    ) -> impl Future<Output = HttpClientResult<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        T: Into<Bytes>,
        T: WasmCompatSend,
        U: From<Bytes>,
        U: WasmCompatSend + 'static,
    {
        let client = self.inner.clone();
        let this = self.clone();
        let modified = self.rewrite_request(req);

        async move {
            let modified = modified?;
            let token = this.get_token().await?;
            let (parts, body) = modified.into_parts();
            let response = client
                .request(parts.method, parts.uri.to_string())
                .headers(parts.headers)
                .header("Authorization", format!("Bearer {token}"))
                .body(body)
                .send()
                .await
                .map_err(|e| Error::Instance(Box::new(e)))?;

            if !response.status().is_success() {
                let status = response.status();
                let body_text = response.text().await.unwrap_or_default();
                return Err(Error::Instance(Box::new(io::Error::new(
                    io::ErrorKind::Other,
                    format!("CodeAssist returned {status}: {body_text}"),
                ))));
            }

            let mut res = Response::builder().status(response.status());
            if let Some(hs) = res.headers_mut() {
                *hs = response.headers().clone();
            }

            let body: LazyBody<U> = Box::pin(async move {
                let bytes = response
                    .bytes()
                    .await
                    .map_err(|e| Error::Instance(Box::new(e)))?;
                let unwrapped = Self::unwrap_response(bytes)?;
                Ok(U::from(unwrapped))
            });

            res.body(body).map_err(Error::Protocol)
        }
    }

    fn send_multipart<U>(
        &self,
        _req: http::Request<MultipartForm>,
    ) -> impl Future<Output = HttpClientResult<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        U: From<Bytes>,
        U: WasmCompatSend + 'static,
    {
        async move {
            Err(Error::Instance(Box::new(io::Error::new(
                io::ErrorKind::Other,
                "CodeAssist does not support multipart requests",
            ))))
        }
    }

    fn send_streaming<T>(
        &self,
        req: http::Request<T>,
    ) -> impl Future<Output = HttpClientResult<StreamingResponse>> + WasmCompatSend
    where
        T: Into<Bytes> + WasmCompatSend,
    {
        let client = self.inner.clone();
        let this = self.clone();
        let modified = self.rewrite_request(req);

        async move {
            let modified = modified?;
            let token = this.get_token().await?;
            let (parts, body) = modified.into_parts();
            let req = client
                .request(parts.method, parts.uri.to_string())
                .headers(parts.headers)
                .header("Authorization", format!("Bearer {token}"))
                .body(body)
                .build()
                .map_err(|e| Error::Instance(Box::new(e)))?;

            let response: reqwest::Response = client
                .execute(req)
                .await
                .map_err(|e| Error::Instance(Box::new(e)))?;

            if !response.status().is_success() {
                let status = response.status();
                let body_text = response.text().await.unwrap_or_default();
                return Err(Error::Instance(Box::new(io::Error::new(
                    io::ErrorKind::Other,
                    format!("CodeAssist returned {status}: {body_text}"),
                ))));
            }

            let mut res = http::Response::builder()
                .status(response.status())
                .version(response.version());
            if let Some(hs) = res.headers_mut() {
                *hs = response.headers().clone();
            }

            let mapped_stream: Pin<Box<dyn WasmCompatSendStream<InnerItem = Result<Bytes, Error>>>> =
                Box::pin(
                    response
                        .bytes_stream()
                        .map(|chunk| chunk.map_err(|e| Error::Instance(Box::new(e)))),
                );

            res.body(mapped_stream).map_err(Error::Protocol)
        }
    }
}

impl Clone for CodeAssistHttpClient {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            auth: self.auth.clone(),
            model: self.model.clone(),
        }
    }
}
