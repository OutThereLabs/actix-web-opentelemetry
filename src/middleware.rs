use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::http::{HeaderName, HeaderValue};
use actix_web::Error;
use futures::future::{ok, FutureResult};
use futures::{Future, Poll};
use opentelemetry::api::{self, HttpB3Propagator, KeyValue, Provider, Span, Tracer, Value};
use std::str::FromStr;

static SPAN_KIND_ATTRIBUTE: &str = "span.kind";
static COMPONENT_ATTRIBUTE: &str = "component";
static HTTP_METHOD_ATTRIBUTE: &str = "http.method";
static HTTP_TARGET_ATTRIBUTE: &str = "http.target";
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


#[derive(Debug)]
pub struct RequestTracing {
    extract_single_header: bool,
}

impl RequestTracing {
    pub fn new(extract_single_header: bool) -> Self {
        RequestTracing {
            extract_single_header,
        }
    }
}

impl<S, B> Transform<S> for RequestTracing
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = RequestTracingMiddleware<S, HttpB3Propagator>;
    type InitError = ();
    type Future = FutureResult<Self::Transform, Self::InitError>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(RequestTracingMiddleware::new(
            service,
            HttpB3Propagator::new(self.extract_single_header),
        ))
    }
}

#[derive(Debug)]
pub struct RequestTracingMiddleware<S, T: api::HttpTextFormat> {
    service: S,
    extractor: T,
}

impl<S, B, T> RequestTracingMiddleware<S, T>
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
    T: api::HttpTextFormat,
{
    fn new(service: S, extractor: T) -> Self {
        RequestTracingMiddleware { service, extractor }
    }
}

impl<S, B, T> Service for RequestTracingMiddleware<S, T>
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
    T: api::HttpTextFormat,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = Box<dyn Future<Item = Self::Response, Error = Self::Error>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready()
    }

    fn call(&mut self, mut req: ServiceRequest) -> Self::Future {
        let parent = self
            .extractor
            .extract(&RequestHeaderCarrier::new(req.headers_mut()));
        let tracer = opentelemetry::global::trace_provider().get_tracer("middleware");
        let mut span = tracer.start("router", Some(parent));
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
        if let Some(remote) = req.connection_info().remote() {
            span.set_attribute(KeyValue::new(HTTP_CLIENT_IP_ATTRIBUTE, remote))
        }
        tracer.mark_span_as_active(&span);

        Box::new(self.service.call(req).then(move |res| match res {
            Ok(ok_res) => {
                span.set_attribute(KeyValue::new(
                    HTTP_STATUS_CODE_ATTRIBUTE,
                    Value::U64(ok_res.status().as_u16() as u64),
                ));
                if let Some(reason) = ok_res.status().canonical_reason() {
                    span.set_attribute(KeyValue::new(HTTP_STATUS_TEXT_ATTRIBUTE, reason));
                }
                span.end();
                tracer.mark_span_as_inactive(span.get_context().span_id());
                Ok(ok_res)
            }
            Err(err) => {
                span.set_attribute(KeyValue::new(ERROR_ATTRIBUTE, Value::Bool(true)));
                span.add_event(format!("{:?}", err));
                span.end();
                tracer.mark_span_as_inactive(span.get_context().span_id());
                Err(err)
            }
        }))
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
