use crate::util::{http_method_str, http_url, NET_PEER_IP};
use actix_http::{encoding::Decoder, BoxedPayloadStream, Error, Payload};
use actix_web::{
    body::MessageBody,
    http::{
        self,
        header::{HeaderName, HeaderValue},
    },
    web::Bytes,
};
use awc::{
    error::SendRequestError,
    http::header::{CONTENT_LENGTH, USER_AGENT},
    ClientRequest, ClientResponse,
};
use futures_util::{future::TryFutureExt as _, Future, Stream};
use opentelemetry::{
    global,
    propagation::Injector,
    trace::{SpanKind, Status, TraceContextExt, Tracer, TracerProvider},
    Context, KeyValue,
};
use opentelemetry_semantic_conventions::trace::{
    HTTP_FLAVOR, HTTP_METHOD, HTTP_REQUEST_CONTENT_LENGTH, HTTP_STATUS_CODE, HTTP_URL,
    HTTP_USER_AGENT, NET_PEER_NAME, NET_PEER_PORT,
};
use serde::Serialize;
use std::fmt::{self, Debug};
use std::mem;
use std::str::FromStr;

/// A wrapper for the actix-web [awc::ClientRequest].
pub struct InstrumentedClientRequest {
    cx: Context,
    attrs: Vec<KeyValue>,
    span_namer: fn(&ClientRequest) -> String,
    request: ClientRequest,
}

impl Debug for InstrumentedClientRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let span_namer = fmt::Pointer::fmt(&(self.span_namer as usize as *const ()), f);
        f.debug_struct("InstrumentedClientRequest")
            .field("cx", &self.cx)
            .field("attrs", &self.attrs)
            .field("span_namer", &span_namer)
            .field("request", &self.request)
            .finish()
    }
}

fn default_span_namer(request: &ClientRequest) -> String {
    format!(
        "{} {}",
        request.get_method(),
        request.get_uri().host().unwrap_or_default()
    )
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
            span_namer: default_span_namer,
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
        let tracer = global::tracer_provider().versioned_tracer(
            "actix-web-opentelemetry",
            Some(env!("CARGO_PKG_VERSION")),
            None,
        );
        self.attrs.extend(
            &mut [
                KeyValue::new(HTTP_METHOD, http_method_str(self.request.get_method())),
                KeyValue::new(HTTP_URL, http_url(self.request.get_uri())),
                KeyValue::new(HTTP_FLAVOR, format!("{:?}", self.request.get_version())),
            ]
            .into_iter(),
        );
        if let Some(user_agent) = self
            .request
            .headers()
            .get(USER_AGENT)
            .and_then(|len| len.to_str().ok())
        {
            self.attrs
                .push(KeyValue::new(HTTP_USER_AGENT, user_agent.to_string()))
        }

        if let Some(content_length) = self.request.headers().get(CONTENT_LENGTH).and_then(|len| {
            len.to_str()
                .ok()
                .and_then(|str_len| str_len.parse::<i64>().ok())
        }) {
            self.attrs
                .push(KeyValue::new(HTTP_REQUEST_CONTENT_LENGTH, content_length))
        }

        if let Some(host) = self.request.get_uri().host() {
            self.attrs
                .push(KeyValue::new(NET_PEER_NAME, host.to_string()));
        }

        if let Some(peer_addr) = self.request.get_peer_addr() {
            self.attrs.push(NET_PEER_IP.string(peer_addr.to_string()));
        }

        if let Some(peer_port) = self.request.get_uri().port_u16() {
            if peer_port != 80 && peer_port != 443 {
                self.attrs.push(NET_PEER_PORT.i64(peer_port.into()));
            }
        }

        let span = tracer
            .span_builder((self.span_namer)(&self.request))
            .with_kind(SpanKind::Client)
            .with_attributes(mem::take(&mut self.attrs))
            .start_with_context(&tracer, &self.cx);
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

    /// Customise the Span Name, for example to reduce cardinality
    ///
    /// Example:
    /// ```
    /// use actix_web_opentelemetry::ClientExt;
    /// use awc::{Client, error::SendRequestError};
    ///
    /// async fn execute_request(client: &Client) -> Result<(), SendRequestError> {
    ///     let res = client.get("http://localhost:8080")
    ///         // Add `trace_request` before `send` to any awc request to add instrumentation
    ///         .trace_request()
    ///         .with_span_namer(|r| format!("HTTP {}", r.get_method()))
    ///         .send()
    ///         .await?;
    ///
    ///     println!("Response: {:?}", res);
    ///     Ok(())
    /// }
    /// ```
    pub fn with_span_namer(
        mut self,
        span_namer: fn(&ClientRequest) -> String,
    ) -> InstrumentedClientRequest {
        self.span_namer = span_namer;
        self
    }
}

// convert http status code to span status following the rules described by the spec:
// https://github.com/open-telemetry/opentelemetry-specification/blob/main/specification/trace/semantic_conventions/http.md#status
fn convert_status(status: http::StatusCode) -> Status {
    match status.as_u16() {
        100..=399 => Status::Unset,
        // since we are the client, we MUST treat 4xx as error
        400..=599 => Status::error(""),
        code => Status::error(format!("Invalid HTTP status code {}", code)),
    }
}

fn record_response<T>(response: &ClientResponse<T>, cx: &Context) {
    let span = cx.span();
    let status = convert_status(response.status());
    span.set_status(status);
    span.set_attribute(HTTP_STATUS_CODE.i64(response.status().as_u16() as i64));
    span.end();
}

fn record_err<T: fmt::Debug>(err: T, cx: &Context) {
    let span = cx.span();
    span.set_status(Status::error(format!("{:?}", err)));
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
