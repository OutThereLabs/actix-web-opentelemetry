use actix_web::client::{ClientRequest, ClientResponse};
use actix_web::http::{HeaderName, HeaderValue};
use futures::{
    future::{self, FutureExt},
    Future,
};
use opentelemetry::api::{
    trace::futures::Instrument, Carrier, HttpTextFormat, KeyValue, Provider, Span, Tracer, Value,
};
use std::str::FromStr;

static SPAN_KIND_ATTRIBUTE: &str = "span.kind";
static COMPONENT_ATTRIBUTE: &str = "component";
static HTTP_METHOD_ATTRIBUTE: &str = "http.method";
static HTTP_URL_ATTRIBUTE: &str = "http.url";
static HTTP_TARGET_ATTRIBUTE: &str = "http.target";
static HTTP_HOST_ATTRIBUTE: &str = "http.host";
static HTTP_SCHEME_ATTRIBUTE: &str = "http.scheme";
static HTTP_STATUS_CODE_ATTRIBUTE: &str = "http.status_code";
static HTTP_STATUS_TEXT_ATTRIBUTE: &str = "http.status_text";
static HTTP_FLAVOR_ATTRIBUTE: &str = "http.flavor";
static ERROR_ATTRIBUTE: &str = "error";

/// Trace an `actix_web::client::Client` request.
///
/// Example:
/// ```rust,no_run
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
pub fn with_tracing<F, R, RE, S>(
    mut request: ClientRequest,
    f: F,
) -> impl Future<Output = Result<ClientResponse<S>, RE>>
where
    F: FnOnce(ClientRequest) -> R,
    R: Future<Output = Result<ClientResponse<S>, RE>>,
    RE: std::fmt::Debug,
{
    let tracer = opentelemetry::global::trace_provider().get_tracer("actix-client");
    let injector = opentelemetry::api::B3Propagator::new(false);
    let mut span = tracer.start(
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
        None,
    );
    span.set_attribute(KeyValue::new(SPAN_KIND_ATTRIBUTE, "client"));
    span.set_attribute(KeyValue::new(COMPONENT_ATTRIBUTE, "http"));
    span.set_attribute(KeyValue::new(
        HTTP_METHOD_ATTRIBUTE,
        request.get_method().as_str(),
    ));
    span.set_attribute(KeyValue::new(
        HTTP_URL_ATTRIBUTE,
        request.get_uri().to_string().as_str(),
    ));
    if let Some(target) = request.get_uri().path_and_query() {
        span.set_attribute(KeyValue::new(HTTP_TARGET_ATTRIBUTE, target.as_str()));
    }
    if let Some(host) = request.get_uri().host() {
        span.set_attribute(KeyValue::new(HTTP_HOST_ATTRIBUTE, host));
    }
    if let Some(scheme) = request.get_uri().scheme_str() {
        span.set_attribute(KeyValue::new(HTTP_SCHEME_ATTRIBUTE, scheme));
    }
    span.set_attribute(KeyValue::new(
        HTTP_FLAVOR_ATTRIBUTE,
        format!("{:?}", request.get_version()).as_str(),
    ));
    injector.inject(
        span.get_context(),
        &mut ActixClientCarrier::new(&mut request),
    );

    f(request)
        .instrument(tracer.clone_span(&span))
        .then(move |result| match result {
            Ok(ok_result) => {
                span.set_attribute(KeyValue::new(
                    HTTP_STATUS_CODE_ATTRIBUTE,
                    Value::U64(ok_result.status().as_u16() as u64),
                ));
                if let Some(reason) = ok_result.status().canonical_reason() {
                    span.set_attribute(KeyValue::new(HTTP_STATUS_TEXT_ATTRIBUTE, reason));
                }
                span.end();
                future::ok(ok_result)
            }
            Err(err) => {
                span.set_attribute(KeyValue::new(ERROR_ATTRIBUTE, Value::Bool(true)));
                span.add_event(format!("{:?}", err));
                span.end();
                future::err(err)
            }
        })
}

struct ActixClientCarrier<'a> {
    request: &'a mut ClientRequest,
}

impl<'a> ActixClientCarrier<'a> {
    fn new(request: &'a mut ClientRequest) -> Self {
        ActixClientCarrier { request }
    }
}

impl<'a> Carrier for ActixClientCarrier<'a> {
    fn get(&self, key: &'static str) -> Option<&str> {
        self.request
            .headers()
            .get(key)
            .map(|value| value.to_str().unwrap())
    }

    fn set(&mut self, key: &'static str, value: String) {
        let header_name = HeaderName::from_str(key).expect("Must be header name");
        let header_value = HeaderValue::from_str(&value).expect("Must be a header value");
        self.request.headers_mut().insert(header_name, header_value);
    }
}
