use bytes::Bytes;
use rig::http_client::{
    self, HttpClientExt, LazyBody, MultipartForm, Request, Response, StreamingResponse,
};

/// Wraps an inner HTTP client and optionally compresses request bodies before
/// delegating — fail-open: any compression error passes the original body
/// through unchanged, so a compression bug can never break a request.
///
/// The `enabled` field gates compression at runtime; set to `false` for a
/// pass-through. Use `DIRGE_COMPRESSION=0` to disable via env.
#[derive(Clone)]
pub(crate) struct CompressingHttpClient<Inner> {
    inner: Inner,
    enabled: bool,
    provider: crate::llmtrim::ir::ProviderKind,
    config: std::sync::Arc<crate::llmtrim::config::DenseConfig>,
}

impl<Inner: Default> Default for CompressingHttpClient<Inner> {
    fn default() -> Self {
        Self {
            inner: Inner::default(),
            enabled: true,
            provider: crate::llmtrim::ir::ProviderKind::OpenAi,
            config: std::sync::Arc::new(crate::compression::dirge_default_config()),
        }
    }
}

impl<Inner: std::fmt::Debug> std::fmt::Debug for CompressingHttpClient<Inner> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompressingHttpClient")
            .field("inner", &self.inner)
            .field("enabled", &self.enabled)
            .finish()
    }
}

impl<Inner> CompressingHttpClient<Inner> {
    /// Construct a compressing HTTP client wrapper. Runtime compression is
    /// controlled by the `enabled` field; set to `false` for a pass-through.
    pub fn new(
        inner: Inner,
        provider: crate::llmtrim::ir::ProviderKind,
        config: std::sync::Arc<crate::llmtrim::config::DenseConfig>,
        enabled: bool,
    ) -> Self {
        Self {
            inner,
            enabled,
            provider,
            config,
        }
    }
}

impl<Inner> CompressingHttpClient<Inner> {
    /// Try to compress the body. On any failure, return the original bytes
    /// unchanged — this is the fail-open guard.
    fn maybe_compress(&self, body: Bytes) -> Bytes {
        if self.enabled {
            let body_str = match std::str::from_utf8(&body) {
                Ok(s) => s,
                Err(_) => return body,
            };
            match crate::compression::rewrite_with(body_str, self.provider, &self.config) {
                Ok(compressed) => {
                    tracing::debug!(
                        target: "dirge::compression",
                        before = body.len(),
                        after = compressed.len(),
                        "compressed request body"
                    );
                    return Bytes::from(compressed);
                }
                Err(e) => {
                    tracing::warn!(
                        target: "dirge::compression",
                        error = %e,
                        "compression failed; sending original body"
                    );
                }
            }
        }
        body
    }

    fn normalized_request<T>(&self, req: Request<T>) -> http_client::Result<Request<Bytes>>
    where
        T: Into<Bytes>,
    {
        let (parts, body) = req.into_parts();
        let body: Bytes = body.into();
        let body = self.maybe_compress(body);
        let mut builder = Request::builder()
            .method(parts.method)
            .uri(parts.uri)
            .version(parts.version);
        if let Some(headers) = builder.headers_mut() {
            *headers = parts.headers;
        }
        builder.body(body).map_err(http_client::Error::Protocol)
    }
}

impl<Inner> HttpClientExt for CompressingHttpClient<Inner>
where
    Inner: HttpClientExt + Clone + Send + Sync + 'static,
{
    fn send<T, U>(
        &self,
        req: Request<T>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + Send + 'static
    where
        T: Into<Bytes>,
        T: Send,
        U: From<Bytes>,
        U: Send + 'static,
    {
        let inner = self.inner.clone();
        let req = self.normalized_request(req);
        async move {
            let req = req?;
            inner.send(req).await
        }
    }

    fn send_multipart<U>(
        &self,
        req: Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + Send + 'static
    where
        U: From<Bytes> + Send + 'static,
    {
        self.inner.send_multipart(req)
    }

    fn send_streaming<T>(
        &self,
        req: Request<T>,
    ) -> impl Future<Output = http_client::Result<StreamingResponse>> + Send
    where
        T: Into<Bytes> + Send,
    {
        let inner = self.inner.clone();
        let req = self.normalized_request(req);
        async move {
            let req = req?;
            inner.send_streaming(req).await
        }
    }
}
