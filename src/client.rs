use actix_web::client::{ClientRequest, ClientResponse};
use actix_web::http::{HeaderName, HeaderValue};
use futures::{future::TryFutureExt, Future};
use opentelemetry::api::{
    Carrier, Context, FutureExt, HttpTextFormat, KeyValue, SpanKind, StatusCode, TraceContextExt,
    Tracer, Value,
};
use opentelemetry::global;
use std::str::FromStr;

static HTTP_METHOD_ATTRIBUTE: &str = "http.method";
static HTTP_URL_ATTRIBUTE: &str = "http.url";
static HTTP_STATUS_CODE_ATTRIBUTE: &str = "http.status_code";
static HTTP_STATUS_TEXT_ATTRIBUTE: &str = "http.status_text";
static HTTP_FLAVOR_ATTRIBUTE: &str = "http.flavor";

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
pub async fn with_tracing<F, R, RE, S>(
    mut request: ClientRequest,
    f: F,
) -> Result<ClientResponse<S>, RE>
where
    F: FnOnce(ClientRequest) -> R,
    R: Future<Output = Result<ClientResponse<S>, RE>>,
    RE: std::fmt::Debug,
{
    let tracer = global::tracer("actix-client");
    let injector = opentelemetry::api::B3Propagator::new(false);
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

    injector.inject_context(&cx, &mut ActixClientCarrier::new(&mut request));

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
