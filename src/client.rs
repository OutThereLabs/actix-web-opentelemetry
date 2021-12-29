use crate::util::http_method_str;
use actix_http::{encoding::Decoder, BoxedPayloadStream, Error, Payload};
use actix_web::{
    body::MessageBody,
    http::{
        self,
        header::{HeaderName, HeaderValue},
    },
    web::Bytes,
};
use awc::{error::SendRequestError, ClientRequest, ClientResponse};
use futures_util::{future::TryFutureExt as _, Future, Stream};
use opentelemetry::{
    global,
    propagation::Injector,
    trace::{SpanKind, StatusCode, TraceContextExt, Tracer},
    Context, KeyValue,
};
use opentelemetry_semantic_conventions::trace::{
    HTTP_FLAVOR, HTTP_METHOD, HTTP_STATUS_CODE, HTTP_URL, NET_PEER_IP,
};
use serde::Serialize;
use std::array::IntoIter;
use std::fmt;
use std::mem;
use std::str::FromStr;

/// A wrapper for the actix-web [awc::ClientRequest].
#[derive(Debug)]
pub struct InstrumentedClientRequest {
    cx: Context,
    attrs: Vec<KeyValue>,
    request: ClientRequest,
}

/// OpenTelemetry extensions for actix-web's [awc::Client].
pub trait ClientExt {
    /// Trace an [awc::Client] request using the current context.
    ///
    /// Example:
    /// ```no_run
    /// use actix_web_opentelemetry::ClientExt;
    /// use awc::{Client, error::SendRequestError};
    ///
    /// async fn execute_request(client: &Client) -> Result<(), SendRequestError> {
    ///     let res = client.get("http://localhost:8080")
    ///         // Add `trace_request` before `send` to any awc request to add instrumentation
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

    /// Trace an [awc::Client] request using the given span context.
    ///
    /// Example:
    /// ```no_run
    /// use actix_web_opentelemetry::ClientExt;
    /// use awc::{Client, error::SendRequestError};
    /// use opentelemetry::Context;
    ///
    /// async fn execute_request(client: &Client) -> Result<(), SendRequestError> {
    ///     let res = client.get("http://localhost:8080")
    ///         // Add `trace_request_with_context` before `send` to any awc request to
    ///         // add instrumentation
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
        InstrumentedClientRequest {
            cx,
            attrs: Vec::new(),
            request: self,
        }
    }
}

type AwcResult = Result<ClientResponse<Decoder<Payload<BoxedPayloadStream>>>, SendRequestError>;

impl InstrumentedClientRequest {
    /// Generate an [`awc::ClientResponse`] from a traced request with an empty body.
    pub async fn send(self) -> AwcResult {
        self.trace_request(|request| request.send()).await
    }

    /// Generate an [awc::ClientResponse] from a traced request with the given body.
    pub async fn send_body<B>(self, body: B) -> AwcResult
    where
        B: MessageBody + 'static,
    {
        self.trace_request(|request| request.send_body(body)).await
    }

    /// Generate an [awc::ClientResponse] from a traced request with the given form
    /// body.
    pub async fn send_form<T: Serialize>(self, value: &T) -> AwcResult {
        self.trace_request(|request| request.send_form(value)).await
    }

    /// Generate an [awc::ClientResponse] from a traced request with the given JSON
    /// body.
    pub async fn send_json<T: Serialize>(self, value: &T) -> AwcResult {
        self.trace_request(|request| request.send_json(value)).await
    }

    /// Generate an [awc::ClientResponse] from a traced request with the given stream
    /// body.
    pub async fn send_stream<S, E>(self, stream: S) -> AwcResult
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin + 'static,
        E: std::error::Error + Into<Error> + 'static,
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
        self.attrs.extend(&mut IntoIter::new([
            KeyValue::new(HTTP_METHOD, http_method_str(self.request.get_method())),
            KeyValue::new(HTTP_URL, self.request.get_uri().to_string()),
            KeyValue::new(
                HTTP_FLAVOR,
                format!("{:?}", self.request.get_version()).replace("HTTP/", ""),
            ),
        ]));

        if let Some(peer_addr) = self.request.get_peer_addr() {
            self.attrs.push(NET_PEER_IP.string(peer_addr.to_string()));
        }

        let span = tracer
            .span_builder(format!(
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
            ))
            .with_kind(SpanKind::Client)
            .with_attributes(mem::take(&mut self.attrs))
            .with_parent_context(self.cx.clone())
            .start(&tracer);
        let cx = self.cx.with_span(span);

        global::get_text_map_propagator(|injector| {
            injector.inject_context(&cx, &mut ActixClientCarrier::new(&mut self.request));
        });

        f(self.request)
            .inspect_ok(|res| record_response(res, &cx))
            .inspect_err(|err| record_err(err, &cx))
            .await
    }

    /// Add additional attributes to the instrumented span for a given request.
    ///
    /// The standard otel attributes will still be tracked.
    ///
    /// Example:
    /// ```
    /// use actix_web_opentelemetry::ClientExt;
    /// use awc::{Client, error::SendRequestError};
    /// use opentelemetry::KeyValue;
    ///
    /// async fn execute_request(client: &Client) -> Result<(), SendRequestError> {
    ///     let attrs = [KeyValue::new("dye-key", "dye-value")];
    ///     let res = client.get("http://localhost:8080")
    ///         // Add `trace_request` before `send` to any awc request to add instrumentation
    ///         .trace_request()
    ///         .with_attributes(attrs)
    ///         .send()
    ///         .await?;
    ///
    ///     println!("Response: {:?}", res);
    ///     Ok(())
    /// }
    /// ```
    pub fn with_attributes(
        mut self,
        attrs: impl IntoIterator<Item = KeyValue>,
    ) -> InstrumentedClientRequest {
        self.attrs.extend(&mut attrs.into_iter());
        self
    }
}

// convert http status code to span status following the rules described by the spec:
// https://github.com/open-telemetry/opentelemetry-specification/blob/main/specification/trace/semantic_conventions/http.md#status
fn convert_status(status: http::StatusCode) -> (StatusCode, Option<String>) {
    match status.as_u16() {
        100..=399 => (StatusCode::Unset, None),
        // since we are the client, we MUST treat 4xx as error
        400..=599 => (StatusCode::Error, None),
        code => (
            StatusCode::Error,
            Some(format!("Invalid HTTP status code {}", code)),
        ),
    }
}

fn record_response<T>(response: &ClientResponse<T>, cx: &Context) {
    let span = cx.span();
    let (span_status, msg) = convert_status(response.status());
    span.set_status(span_status, msg.unwrap_or_default());
    span.set_attribute(HTTP_STATUS_CODE.i64(response.status().as_u16() as i64));
    span.end();
}

fn record_err<T: fmt::Debug>(err: T, cx: &Context) {
    let span = cx.span();
    span.set_status(StatusCode::Error, format!("{:?}", err));
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
