use super::route_formatter::{RouteFormatter, UuidWildcardFormatter};
use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::http::{HeaderName, HeaderValue};
use actix_web::Error;
use futures::{
    future::{ok, FutureExt, Ready},
    Future,
};
use opentelemetry::api::trace::b3_propagator::B3Encoding;
use opentelemetry::api::{
    self, Context, FutureExt as OtelFutureExt, KeyValue, StatusCode, TraceContextExt, Tracer, Value,
};
use opentelemetry::global;
use std::pin::Pin;
use std::str::FromStr;
use std::task::Poll;

// Http common attributes
static HTTP_METHOD_ATTRIBUTE: &str = "http.method";
static HTTP_TARGET_ATTRIBUTE: &str = "http.target";
static HTTP_SCHEME_ATTRIBUTE: &str = "http.scheme";
static HTTP_STATUS_CODE_ATTRIBUTE: &str = "http.status_code";
static HTTP_STATUS_TEXT_ATTRIBUTE: &str = "http.status_text";
static HTTP_FLAVOR_ATTRIBUTE: &str = "http.flavor";
static HTTP_USER_AGENT_ATTRIBUTE: &str = "http.user_agent";

// Http server attributes
static HTTP_HOST_ATTRIBUTE: &str = "http.host";
static HTTP_SERVER_NAME_ATTRIBUTE: &str = "http.server_name";
static HTTP_ROUTE_ATTRIBUTE: &str = "http.route";
static HTTP_CLIENT_IP_ATTRIBUTE: &str = "http.client_ip";
static NET_HOST_PORT_ATTRIBUTE: &str = "net.host.port";

/// Request tracing middleware.
///
/// Example:
/// ```no_run
/// #[macro_use]
/// extern crate actix_web;
///
/// use actix_web::{web, App, HttpServer};
/// use actix_web_opentelemetry::RequestTracing;
/// use opentelemetry::api;
///
/// fn init_tracer() {
///     // Replace this no-op provider with something like:
///     // https://docs.rs/opentelemetry-jaeger
///     opentelemetry::global::set_provider(api::NoopProvider {});
/// }
///
/// async fn index() -> &'static str {
///     "Hello world!"
/// }
///
/// #[actix_rt::main]
/// async fn main() -> std::io::Result<()> {
///     init_tracer();
///     HttpServer::new(|| {
///         App::new()
///             .wrap(RequestTracing::default())
///             .service(web::resource("/").to(index))
///     })
///     .bind("127.0.0.1:8080")?
///     .run()
///     .await
/// }
///```
#[derive(Debug)]
pub struct RequestTracing<R: RouteFormatter> {
    header_encoding: B3Encoding,
    route_formatter: R,
}

impl Default for RequestTracing<UuidWildcardFormatter> {
    fn default() -> Self {
        RequestTracing {
            header_encoding: B3Encoding::MultipleHeader,
            route_formatter: UuidWildcardFormatter::new(),
        }
    }
}

impl<R: RouteFormatter> RequestTracing<R> {
    /// Configures a request tracing middleware transformer.
    ///
    /// This middleware supports both version of B3 headers.
    ///  1. Single Header:
    ///
    ///    - X-B3: `{trace_id}-{span_id}-{sampling_state}-{parent_span_id}`
    ///
    ///  2. Multiple Headers:
    ///
    ///    - X-B3-TraceId: `{trace_id}`
    ///    - X-B3-ParentSpanId: `{parent_span_id}`
    ///    - X-B3-SpanId: `{span_id}`
    ///    - X-B3-Sampled: `{sampling_state}`
    ///    - X-B3-Flags: `{debug_flag}`
    pub fn new(header_encoding: B3Encoding, route_formatter: R) -> Self {
        RequestTracing {
            header_encoding,
            route_formatter,
        }
    }
}

impl<S, B, R> Transform<S> for RequestTracing<R>
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
    R: RouteFormatter + Clone,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = RequestTracingMiddleware<S, R>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(RequestTracingMiddleware::new(
            service,
            self.route_formatter.clone(),
        ))
    }
}

#[derive(Debug)]
pub struct RequestTracingMiddleware<S, R: RouteFormatter> {
    service: S,
    route_formatter: R,
}

impl<S, B, R> RequestTracingMiddleware<S, R>
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
    R: RouteFormatter,
{
    fn new(service: S, route_formatter: R) -> Self {
        RequestTracingMiddleware {
            service,
            route_formatter,
        }
    }
}

impl<S, B, R> Service for RequestTracingMiddleware<S, R>
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
    R: RouteFormatter,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, mut req: ServiceRequest) -> Self::Future {
        let _parent_context = global::get_http_text_propagator(|propagator| {
            propagator.extract(&RequestHeaderCarrier::new(req.headers_mut()))
        })
        .attach();
        let tracer = global::tracer("actix-web-opentelemetry");
        let http_route = self.route_formatter.format(req.uri().path());
        let mut builder = tracer.span_builder(&http_route);
        builder.span_kind = Some(api::SpanKind::Server);
        let mut attributes = vec![
            KeyValue::new(HTTP_METHOD_ATTRIBUTE, req.method().as_str()),
            KeyValue::new(
                HTTP_FLAVOR_ATTRIBUTE,
                format!("{:?}", req.version()).replace("HTTP/", ""),
            ),
            KeyValue::new(HTTP_HOST_ATTRIBUTE, req.connection_info().host()),
            KeyValue::new(HTTP_ROUTE_ATTRIBUTE, http_route),
            KeyValue::new(HTTP_SCHEME_ATTRIBUTE, req.connection_info().scheme()),
        ];
        let server_name = req.app_config().host();
        if server_name != req.connection_info().host() {
            attributes.push(KeyValue::new(HTTP_SERVER_NAME_ATTRIBUTE, server_name));
        }
        if let Some(port) = req.connection_info().host().split_terminator(':').nth(1) {
            attributes.push(KeyValue::new(NET_HOST_PORT_ATTRIBUTE, port))
        }
        if let Some(path) = req.uri().path_and_query() {
            attributes.push(KeyValue::new(HTTP_TARGET_ATTRIBUTE, path.as_str()))
        }
        if let Some(user_agent) = req
            .headers()
            .get("User-Agent")
            .and_then(|s| s.to_str().ok())
        {
            attributes.push(KeyValue::new(HTTP_USER_AGENT_ATTRIBUTE, user_agent))
        }
        if let Some(remote) = req.connection_info().remote() {
            attributes.push(KeyValue::new(HTTP_CLIENT_IP_ATTRIBUTE, remote))
        }
        builder.attributes = Some(attributes);
        let span = tracer.build(builder);
        let cx = Context::current_with_span(span);

        let fut = self
            .service
            .call(req)
            .with_context(cx.clone())
            .map(move |res| match res {
                Ok(ok_res) => {
                    let span = cx.span();
                    span.set_attribute(KeyValue::new(
                        HTTP_STATUS_CODE_ATTRIBUTE,
                        Value::U64(ok_res.status().as_u16() as u64),
                    ));
                    if let Some(reason) = ok_res.status().canonical_reason() {
                        span.set_attribute(KeyValue::new(HTTP_STATUS_TEXT_ATTRIBUTE, reason));
                    }
                    let status_code = match ok_res.status().as_u16() {
                        100..=399 => StatusCode::OK,
                        401 => StatusCode::Unauthenticated,
                        403 => StatusCode::PermissionDenied,
                        404 => StatusCode::NotFound,
                        429 => StatusCode::ResourceExhausted,
                        400..=499 => StatusCode::InvalidArgument,
                        501 => StatusCode::Unimplemented,
                        503 => StatusCode::Unavailable,
                        504 => StatusCode::DeadlineExceeded,
                        500..=599 => StatusCode::Internal,
                        _ => StatusCode::Unknown,
                    };
                    span.set_status(status_code, "".to_string());
                    span.end();
                    Ok(ok_res)
                }
                Err(err) => {
                    let span = cx.span();
                    span.set_status(StatusCode::Internal, format!("{:?}", err));
                    span.end();
                    Err(err)
                }
            });

        Box::pin(async move { fut.await })
    }
}

struct RequestHeaderCarrier<'a> {
    headers: &'a mut actix_web::http::HeaderMap,
}

impl<'a> RequestHeaderCarrier<'a> {
    fn new(headers: &'a mut actix_web::http::HeaderMap) -> Self {
        RequestHeaderCarrier { headers }
    }
}

impl<'a> opentelemetry::api::Carrier for RequestHeaderCarrier<'a> {
    fn get(&self, key: &str) -> Option<&str> {
        self.headers.get(key).and_then(|v| v.to_str().ok())
    }

    fn set(&mut self, key: &str, value: String) {
        let header_name = HeaderName::from_str(key).expect("Must be header name");
        let header_value = HeaderValue::from_str(&value).expect("Must be a header value");
        self.headers.insert(header_name, header_value)
    }
}
