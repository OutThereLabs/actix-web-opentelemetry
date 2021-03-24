use super::route_formatter::RouteFormatter;
use crate::util::{http_flavor, http_method_str, http_scheme};
use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::{http::header, Error};
use futures::{
    future::{ok, FutureExt, Ready},
    Future,
};
use opentelemetry::{
    global,
    propagation::Extractor,
    trace::{FutureExt as OtelFutureExt, SpanKind, StatusCode, TraceContextExt, Tracer},
    Context,
};
use opentelemetry_semantic_conventions::trace::{
    HTTP_CLIENT_IP, HTTP_FLAVOR, HTTP_HOST, HTTP_METHOD, HTTP_ROUTE, HTTP_SCHEME, HTTP_SERVER_NAME,
    HTTP_STATUS_CODE, HTTP_TARGET, HTTP_USER_AGENT, NET_HOST_PORT, NET_PEER_IP,
};
use std::borrow::Cow;
use std::pin::Pin;
use std::rc::Rc;
use std::task::Poll;

/// Request tracing middleware.
///
/// # Examples:
///
/// ```no_run
/// use actix_web::{web, App, HttpServer};
/// use actix_web_opentelemetry::RequestTracing;
///
/// async fn index() -> &'static str {
///     "Hello world!"
/// }
///
/// #[actix_web::main]
/// async fn main() -> std::io::Result<()> {
///     // Install an OpenTelemetry trace pipeline.
///     // Swap for https://docs.rs/opentelemetry-jaeger or other compatible
///     // exporter to send trace information to your collector.
///     opentelemetry::sdk::export::trace::stdout::new_pipeline().install_simple();
///
///     HttpServer::new(|| {
///         App::new()
///             .wrap(RequestTracing::new())
///             .service(web::resource("/").to(index))
///     })
///     .bind("127.0.0.1:8080")?
///     .run()
///     .await
/// }
///```
#[derive(Default, Debug)]
pub struct RequestTracing {
    route_formatter: Option<Rc<dyn RouteFormatter + 'static>>,
}

impl RequestTracing {
    /// Actix web middleware to trace each request in an OpenTelemetry span.
    pub fn new() -> RequestTracing {
        RequestTracing::default()
    }

    /// Actix web middleware to trace each request in an OpenTelemetry span with
    /// formatted routes.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use actix_web::{web, App, HttpServer};
    /// use actix_web_opentelemetry::{RouteFormatter, RequestTracing};
    ///
    /// # #[actix_web::main]
    /// # async fn main() -> std::io::Result<()> {
    ///
    ///
    /// #[derive(Debug)]
    /// struct MyLowercaseFormatter;
    ///
    /// impl RouteFormatter for MyLowercaseFormatter {
    ///     fn format(&self, path: &str) -> String {
    ///         path.to_lowercase()
    ///     }
    /// }
    ///
    /// // report /users/{id} as /users/:id
    /// HttpServer::new(move || {
    ///     App::new()
    ///         .wrap(RequestTracing::with_formatter(MyLowercaseFormatter))
    ///         .service(web::resource("/users/{id}").to(|| async { "ok" }))
    /// })
    /// .bind("127.0.0.1:8080")?
    /// .run()
    /// .await
    /// # }
    /// ```
    pub fn with_formatter<T: RouteFormatter + 'static>(route_formatter: T) -> Self {
        RequestTracing {
            route_formatter: Some(Rc::new(route_formatter)),
        }
    }
}

impl<S, B> Transform<S, ServiceRequest> for RequestTracing
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = RequestTracingMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(RequestTracingMiddleware::new(
            global::tracer_with_version("actix-web-opentelemetry", env!("CARGO_PKG_VERSION")),
            service,
            self.route_formatter.clone(),
        ))
    }
}

#[derive(Debug)]
pub struct RequestTracingMiddleware<S> {
    tracer: global::BoxedTracer,
    service: S,
    route_formatter: Option<Rc<dyn RouteFormatter>>,
}

impl<S, B> RequestTracingMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    fn new(
        tracer: global::BoxedTracer,
        service: S,
        route_formatter: Option<Rc<dyn RouteFormatter>>,
    ) -> Self {
        RequestTracingMiddleware {
            tracer,
            service,
            route_formatter,
        }
    }
}

impl<S, B> Service<ServiceRequest> for RequestTracingMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    #[allow(clippy::type_complexity)]
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(&self, cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&self, mut req: ServiceRequest) -> Self::Future {
        let parent_context = global::get_text_map_propagator(|propagator| {
            propagator.extract(&RequestHeaderCarrier::new(req.headers_mut()))
        });
        let mut http_route: Cow<'static, str> = req
            .match_pattern()
            .map(Into::into)
            .unwrap_or_else(|| "default".into());
        if let Some(formatter) = &self.route_formatter {
            http_route = formatter.format(&http_route).into();
        }
        let conn_info = req.connection_info();
        let mut builder = self.tracer.span_builder(&http_route);
        builder.parent_context = Some(parent_context);
        builder.span_kind = Some(SpanKind::Server);
        let mut attributes = Vec::with_capacity(11);
        attributes.push(HTTP_METHOD.string(http_method_str(req.method())));
        attributes.push(HTTP_FLAVOR.string(http_flavor(req.version())));
        attributes.push(HTTP_HOST.string(conn_info.host().to_string()));
        attributes.push(HTTP_ROUTE.string(http_route));
        attributes.push(HTTP_SCHEME.string(http_scheme(conn_info.scheme())));

        let server_name = req.app_config().host();
        if server_name != conn_info.host() {
            attributes.push(HTTP_SERVER_NAME.string(server_name.to_string()));
        }
        if let Some(port) = conn_info
            .host()
            .split_terminator(':')
            .nth(1)
            .and_then(|port| port.parse().ok())
        {
            attributes.push(NET_HOST_PORT.i64(port))
        }
        if let Some(path) = req.uri().path_and_query() {
            attributes.push(HTTP_TARGET.string(path.as_str().to_string()))
        }
        if let Some(user_agent) = req
            .headers()
            .get(header::USER_AGENT)
            .and_then(|s| s.to_str().ok())
        {
            attributes.push(HTTP_USER_AGENT.string(user_agent.to_string()))
        }
        let remote_addr = conn_info.realip_remote_addr();
        if let Some(remote) = remote_addr {
            attributes.push(HTTP_CLIENT_IP.string(remote.to_string()))
        }
        if let Some(peer_addr) = req.peer_addr().map(|socket| socket.to_string()) {
            if Some(peer_addr.as_str()) != remote_addr {
                // Client is going through a proxy
                attributes.push(NET_PEER_IP.string(peer_addr))
            }
        }
        builder.attributes = Some(attributes);
        let span = self.tracer.build(builder);
        let cx = Context::current_with_span(span);
        #[cfg(feature = "sync-middleware")]
        let attachment = cx.clone().attach();
        drop(conn_info);

        let fut = self
            .service
            .call(req)
            .with_context(cx.clone())
            .map(move |res| match res {
                Ok(ok_res) => {
                    let span = cx.span();
                    span.set_attribute(HTTP_STATUS_CODE.i64(ok_res.status().as_u16() as i64));
                    if ok_res.status().is_server_error() {
                        span.set_status(
                            StatusCode::Error,
                            ok_res
                                .status()
                                .canonical_reason()
                                .map(ToString::to_string)
                                .unwrap_or_default(),
                        );
                    };
                    span.end();
                    Ok(ok_res)
                }
                Err(err) => {
                    let span = cx.span();
                    span.set_status(StatusCode::Error, format!("{:?}", err));
                    span.end();
                    Err(err)
                }
            });

        #[cfg(feature = "sync-middleware")]
        drop(attachment);
        Box::pin(async move { fut.await })
    }
}

struct RequestHeaderCarrier<'a> {
    headers: &'a actix_web::http::HeaderMap,
}

impl<'a> RequestHeaderCarrier<'a> {
    fn new(headers: &'a actix_web::http::HeaderMap) -> Self {
        RequestHeaderCarrier { headers }
    }
}

impl<'a> Extractor for RequestHeaderCarrier<'a> {
    fn get(&self, key: &str) -> Option<&str> {
        self.headers.get(key).and_then(|v| v.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.headers.keys().map(|header| header.as_str()).collect()
    }
}
