use actix_http::{encoding::Decoder, Error, Payload, PayloadStream};
use actix_web::{
    body::Body,
    client::{ClientRequest, ClientResponse, SendRequestError},
    http::{HeaderName, HeaderValue},
    web::Bytes,
};
use futures::{future::TryFutureExt, Future, Stream};
use opentelemetry::api::{
    Context, FutureExt, Injector, KeyValue, SpanKind, StatusCode, TraceContextExt, Tracer, Value,
};
use opentelemetry::global;
use serde::Serialize;
use std::fmt;
use std::str::FromStr;

static HTTP_METHOD_ATTRIBUTE: &str = "http.method";
static HTTP_URL_ATTRIBUTE: &str = "http.url";
static HTTP_STATUS_CODE_ATTRIBUTE: &str = "http.status_code";
static HTTP_STATUS_TEXT_ATTRIBUTE: &str = "http.status_text";
static HTTP_FLAVOR_ATTRIBUTE: &str = "http.flavor";

/// Trace an `actix_web::client::Client` request.
///
/// Example:
/// ```no_run
/// use actix_web::client;
/// use futures::Future;
///
/// async fn execute_request(client: &client::Client) -> Result<(), client::SendRequestError> {
///     let mut res = actix_web_opentelemetry::with_tracing(
///         client.get("http://localhost:8080"),
///         |request| request.send()
///     )
///     .await;
///
///     res.and_then(|res| {
///         println!("Response: {:?}", res);
///         Ok(())
///     })
/// }
/// ```
#[deprecated(since = "0.5.0", note = "Please use ClientExt::trace_request instead")]
pub async fn with_tracing<F, R, RE, S>(
    mut request: ClientRequest,
    f: F,
) -> Result<ClientResponse<S>, RE>
where
    F: FnOnce(ClientRequest) -> R,
    R: Future<Output = Result<ClientResponse<S>, RE>>,
    RE: fmt::Debug,
{
    let tracer = global::tracer("actix-client");
    let span = tracer
        .span_builder(
            format!(
                "{} {}{}{}",
                request.get_method(),
                request
                    .get_uri()
                    .scheme()
                    .map(|s| format!("{}://", s.as_str()))
                    .unwrap_or_else(String::new),
                request
                    .get_uri()
                    .authority()
                    .map(|s| s.as_str())
                    .unwrap_or(""),
                request.get_uri().path()
            )
            .as_str(),
        )
        .with_kind(SpanKind::Client)
        .with_attributes(vec![
            KeyValue::new(HTTP_METHOD_ATTRIBUTE, request.get_method().as_str()),
            KeyValue::new(HTTP_URL_ATTRIBUTE, request.get_uri().to_string().as_str()),
            KeyValue::new(
                HTTP_FLAVOR_ATTRIBUTE,
                format!("{:?}", request.get_version()).replace("HTTP/", ""),
            ),
        ])
        .start(&tracer);
    let cx = Context::current_with_span(span);

    global::get_http_text_propagator(|injector| {
        injector.inject_context(&cx, &mut ActixClientCarrier::new(&mut request))
    });

    f(request)
        .with_context(cx.clone())
        .inspect_ok(|ok_result| {
            let span = cx.span();
            span.set_attribute(KeyValue::new(
                HTTP_STATUS_CODE_ATTRIBUTE,
                Value::U64(ok_result.status().as_u16() as u64),
            ));
            if let Some(reason) = ok_result.status().canonical_reason() {
                span.set_attribute(KeyValue::new(HTTP_STATUS_TEXT_ATTRIBUTE, reason));
            }
            span.end();
        })
        .inspect_err(|err| {
            let span = cx.span();
            span.set_status(StatusCode::Internal, format!("{:?}", err));
            span.end();
        })
        .await
}

/// A wrapper for the actix-web [`ClientRequest`].
///
/// [`ClientRequest`]: https://docs.rs/actix-web/2.0.0/actix_web/client/struct.ClientRequest.html
#[derive(Debug)]
pub struct InstrumentedClientRequest {
    cx: Context,
    request: ClientRequest,
}

/// OpenTelemetry extensions for actix-web's [`Client`].
///
/// [`Client`]: https://docs.rs/actix-web/2.0.0/actix_web/client/struct.Client.html
pub trait ClientExt {
    /// Trace an `actix_web::client::Client` request using the current context.
    ///
    /// Example:
    /// ```no_run
    /// use actix_web::client;
    /// use actix_web_opentelemetry::ClientExt;
    ///
    /// async fn execute_request(client: &client::Client) -> Result<(), client::SendRequestError> {
    ///     let res = client.get("http://localhost:8080")
    ///         .trace_request()
    ///         .send()
    ///         .await?;
    ///
    ///     println!("Response: {:?}", res);
    ///     Ok(())
    /// }
    /// ```
    fn trace_request(self) -> InstrumentedClientRequest
    where
        Self: Sized,
    {
        self.trace_request_with_context(Context::current())
    }

    /// Trace an `actix_web::client::Client` request using the given span context.
    ///
    /// Example:
    /// ```no_run
    /// use actix_web::client;
    /// use actix_web_opentelemetry::ClientExt;
    /// use opentelemetry::api::Context;
    ///
    /// async fn execute_request(client: &client::Client) -> Result<(), client::SendRequestError> {
    ///     let res = client.get("http://localhost:8080")
    ///         .trace_request_with_context(Context::current())
    ///         .send()
    ///         .await?;
    ///
    ///     println!("Response: {:?}", res);
    ///     Ok(())
    /// }
    /// ```
    fn trace_request_with_context(self, cx: Context) -> InstrumentedClientRequest;
}

impl ClientExt for ClientRequest {
    fn trace_request_with_context(self, cx: Context) -> InstrumentedClientRequest {
        InstrumentedClientRequest { cx, request: self }
    }
}

type AwcResult = Result<ClientResponse<Decoder<Payload<PayloadStream>>>, SendRequestError>;

impl InstrumentedClientRequest {
    /// Generate an awc `ClientResponse` from a traced request with an empty body.
    ///
    /// [`ClientResponse`]: https://docs.rs/actix-web/2.0.0/actix_web/client/struct.ClientResponse.html
    pub async fn send(self) -> AwcResult {
        self.trace_request(|request| request.send()).await
    }

    /// Generate an awc `ClientResponse` from a traced request with the given body.
    ///
    /// [`ClientResponse`]: https://docs.rs/actix-web/2.0.0/actix_web/client/struct.ClientResponse.html
    pub async fn send_body<B>(self, body: B) -> AwcResult
    where
        B: Into<Body>,
    {
        self.trace_request(|request| request.send_body(body)).await
    }

    /// Generate an awc `ClientResponse` from a traced request with the given form
    /// body.
    ///
    /// [`ClientResponse`]: https://docs.rs/actix-web/2.0.0/actix_web/client/struct.ClientResponse.html
    pub async fn send_form<T: Serialize>(self, value: &T) -> AwcResult {
        self.trace_request(|request| request.send_form(value)).await
    }

    /// Generate an awc `ClientResponse` from a traced request with the given JSON
    /// body.
    ///
    /// [`ClientResponse`]: https://docs.rs/actix-web/2.0.0/actix_web/client/struct.ClientResponse.html
    pub async fn send_json<T: Serialize>(self, value: &T) -> AwcResult {
        self.trace_request(|request| request.send_json(value)).await
    }

    /// Generate an awc `ClientResponse` from a traced request with the given stream
    /// body.
    ///
    /// [`ClientResponse`]: https://docs.rs/actix-web/2.0.0/actix_web/client/struct.ClientResponse.html
    pub async fn send_stream<S, E>(self, stream: S) -> AwcResult
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin + 'static,
        E: Into<Error> + 'static,
    {
        self.trace_request(|request| request.send_stream(stream))
            .await
    }

    async fn trace_request<F, R>(mut self, f: F) -> AwcResult
    where
        F: FnOnce(ClientRequest) -> R,
        R: Future<Output = AwcResult>,
    {
        let tracer = global::tracer("actix-client");
        let span = tracer
            .span_builder(
                format!(
                    "{} {}{}{}",
                    self.request.get_method(),
                    self.request
                        .get_uri()
                        .scheme()
                        .map(|s| format!("{}://", s.as_str()))
                        .unwrap_or_else(String::new),
                    self.request
                        .get_uri()
                        .authority()
                        .map(|s| s.as_str())
                        .unwrap_or(""),
                    self.request.get_uri().path()
                )
                .as_str(),
            )
            .with_kind(SpanKind::Client)
            .with_attributes(vec![
                KeyValue::new(HTTP_METHOD_ATTRIBUTE, self.request.get_method().as_str()),
                KeyValue::new(
                    HTTP_URL_ATTRIBUTE,
                    self.request.get_uri().to_string().as_str(),
                ),
                KeyValue::new(
                    HTTP_FLAVOR_ATTRIBUTE,
                    format!("{:?}", self.request.get_version()).replace("HTTP/", ""),
                ),
            ])
            .start(&tracer);
        let cx = self.cx.with_span(span);

        global::get_http_text_propagator(|injector| {
            injector.inject_context(&cx, &mut ActixClientCarrier::new(&mut self.request));
        });

        f(self.request)
            .inspect_ok(|res| record_response(&res, &cx))
            .inspect_err(|err| record_err(err, &cx))
            .await
    }
}

fn record_response<T>(response: &ClientResponse<T>, cx: &Context) {
    let span = cx.span();
    span.set_attribute(KeyValue::new(
        HTTP_STATUS_CODE_ATTRIBUTE,
        Value::U64(response.status().as_u16() as u64),
    ));
    if let Some(reason) = response.status().canonical_reason() {
        span.set_attribute(KeyValue::new(HTTP_STATUS_TEXT_ATTRIBUTE, reason));
    }
    span.end();
}

fn record_err<T: fmt::Debug>(err: T, cx: &Context) {
    let span = cx.span();
    span.set_status(StatusCode::Internal, format!("{:?}", err));
    span.end();
}

struct ActixClientCarrier<'a> {
    request: &'a mut ClientRequest,
}

impl<'a> ActixClientCarrier<'a> {
    fn new(request: &'a mut ClientRequest) -> Self {
        ActixClientCarrier { request }
    }
}

impl<'a> Injector for ActixClientCarrier<'a> {
    fn set(&mut self, key: &str, value: String) {
        let header_name = HeaderName::from_str(key).expect("Must be header name");
        let header_value = HeaderValue::from_str(&value).expect("Must be a header value");
        self.request.headers_mut().insert(header_name, header_value);
    }
}
