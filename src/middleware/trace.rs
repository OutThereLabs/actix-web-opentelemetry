use super::route_formatter::{RouteFormatter, UuidWildcardFormatter};
use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::http::{HeaderName, HeaderValue};
use actix_web::Error;
use futures::{
    future::{ok, FutureExt, Ready},
    Future,
};
use opentelemetry::api::{
    self, trace::futures::Instrument, B3Propagator, KeyValue, Provider, Span, Tracer, Value,
};
use std::pin::Pin;
use std::str::FromStr;
use std::task::Poll;

static SPAN_KIND_ATTRIBUTE: &str = "span.kind";
static COMPONENT_ATTRIBUTE: &str = "component";
static HTTP_METHOD_ATTRIBUTE: &str = "http.method";
static HTTP_TARGET_ATTRIBUTE: &str = "http.target";
static HTTP_ROUTE_ATTRIBUTE: &str = "http.route";
static HTTP_HOST_ATTRIBUTE: &str = "http.host";
static HTTP_SCHEME_ATTRIBUTE: &str = "http.scheme";
static HTTP_STATUS_CODE_ATTRIBUTE: &str = "http.status_code";
static HTTP_STATUS_TEXT_ATTRIBUTE: &str = "http.status_text";
static HTTP_FLAVOR_ATTRIBUTE: &str = "http.flavor";

static HTTP_SERVER_NAME_ATTRIBUTE: &str = "http.server_name";
static HTTP_CLIENT_IP_ATTRIBUTE: &str = "http.client_ip";
static HOST_NAME_ATTRIBUTE: &str = "host.name";
static HOST_PORT_ATTRIBUTE: &str = "host.port";
static ERROR_ATTRIBUTE: &str = "error";

/// Request tracing middleware.
///
/// Example:
/// ```rust,no_run
/// #[macro_use]
/// extern crate actix_web;
///
/// use actix_web::{web, App, HttpServer};
/// use actix_web_opentelemetry::RequestTracing;
/// use opentelemetry::api;
///
/// fn init_tracer() {
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
    extract_single_header: bool,
    route_formatter: R,
}

impl Default for RequestTracing<UuidWildcardFormatter> {
    fn default() -> Self {
        RequestTracing {
            extract_single_header: false,
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
    pub fn new(extract_single_header: bool, route_formatter: R) -> Self {
        RequestTracing {
            extract_single_header,
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
    type Transform = RequestTracingMiddleware<S, B3Propagator, R>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(RequestTracingMiddleware::new(
            service,
            B3Propagator::new(self.extract_single_header),
            self.route_formatter.clone(),
        ))
    }
}

#[derive(Debug)]
pub struct RequestTracingMiddleware<S, H: api::HttpTextFormat, R: RouteFormatter> {
    service: S,
    header_extractor: H,
    route_formatter: R,
}

impl<S, B, H, R> RequestTracingMiddleware<S, H, R>
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
    H: api::HttpTextFormat,
    R: RouteFormatter,
{
    fn new(service: S, header_extractor: H, route_formatter: R) -> Self {
        RequestTracingMiddleware {
            service,
            header_extractor,
            route_formatter,
        }
    }
}

impl<S, B, H, R> Service for RequestTracingMiddleware<S, H, R>
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
    H: api::HttpTextFormat,
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
        let parent = self
            .header_extractor
            .extract(&RequestHeaderCarrier::new(req.headers_mut()));
        let tracer = opentelemetry::global::trace_provider().get_tracer("actix-web-opentelemetry");
        let mut span = tracer.start("middleware", Some(parent));
        span.set_attribute(KeyValue::new(SPAN_KIND_ATTRIBUTE, "server"));
        span.set_attribute(KeyValue::new(COMPONENT_ATTRIBUTE, "http"));
        span.set_attribute(KeyValue::new(HTTP_METHOD_ATTRIBUTE, req.method().as_str()));
        span.set_attribute(KeyValue::new(
            HTTP_FLAVOR_ATTRIBUTE,
            format!("{:?}", req.version()).as_str(),
        ));
        let server_name = req.app_config().host();
        if server_name != req.connection_info().host() {
            span.set_attribute(KeyValue::new(HTTP_SERVER_NAME_ATTRIBUTE, server_name));
        }
        span.set_attribute(KeyValue::new(
            HOST_NAME_ATTRIBUTE,
            req.connection_info().host(),
        ));
        if let Some(port) = req.uri().port_u16() {
            span.set_attribute(KeyValue::new(HOST_PORT_ATTRIBUTE, Value::U64(port as u64)))
        }
        if let Some(host) = req.uri().host() {
            span.set_attribute(KeyValue::new(HTTP_HOST_ATTRIBUTE, host))
        }
        if let Some(scheme) = req.uri().scheme_str() {
            span.set_attribute(KeyValue::new(HTTP_SCHEME_ATTRIBUTE, scheme))
        }
        if let Some(path) = req.uri().path_and_query() {
            span.set_attribute(KeyValue::new(HTTP_TARGET_ATTRIBUTE, path.as_str()))
        }
        if let Some(path) = req.uri().path_and_query() {
            span.set_attribute(KeyValue::new(
                HTTP_ROUTE_ATTRIBUTE,
                self.route_formatter.format(path.as_str()).as_str(),
            ))
        }
        if let Some(remote) = req.connection_info().remote() {
            span.set_attribute(KeyValue::new(HTTP_CLIENT_IP_ATTRIBUTE, remote))
        }

        let fut = self
            .service
            .call(req)
            .instrument(tracer.clone_span(&span))
            .map(move |res| match res {
                Ok(ok_res) => {
                    span.set_attribute(KeyValue::new(
                        HTTP_STATUS_CODE_ATTRIBUTE,
                        Value::U64(ok_res.status().as_u16() as u64),
                    ));
                    if let Some(reason) = ok_res.status().canonical_reason() {
                        span.set_attribute(KeyValue::new(HTTP_STATUS_TEXT_ATTRIBUTE, reason));
                    }
                    span.end();
                    Ok(ok_res)
                }
                Err(err) => {
                    span.set_attribute(KeyValue::new(ERROR_ATTRIBUTE, Value::Bool(true)));
                    span.add_event(format!("{:?}", err));
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
    fn get(&self, key: &'static str) -> Option<&str> {
        self.headers.get(key).and_then(|v| v.to_str().ok())
    }

    fn set(&mut self, key: &'static str, value: String) {
        let header_name = HeaderName::from_str(key).expect("Must be header name");
        let header_value = HeaderValue::from_str(&value).expect("Must be a header value");
        self.headers.insert(header_name, header_value)
    }
}
