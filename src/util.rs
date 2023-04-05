use actix_http::header;
use actix_web::{
    dev::ServiceRequest,
    http::{Method, Version},
};
#[cfg(feature = "metrics")]
use opentelemetry::KeyValue;
use opentelemetry::{trace::OrderMap, Key, Value};
use opentelemetry_semantic_conventions::{
    resource::HOST_NAME,
    trace::{
        HTTP_CLIENT_IP, HTTP_FLAVOR, HTTP_METHOD, HTTP_ROUTE, HTTP_SCHEME, HTTP_TARGET,
        HTTP_USER_AGENT, NET_HOST_PORT,
    },
};
pub(crate) const NET_PEER_IP: Key = Key::from_static_str("net.peer.ip");
pub(crate) const HTTP_SERVER_NAME: Key = Key::from_static_str("http.server_name");

#[cfg(feature = "awc")]
use std::fmt::Write;

#[cfg(feature = "awc")]
#[inline]
pub(super) fn http_url(uri: &actix_web::http::Uri) -> String {
    let mut url = String::new();
    if let Some(scheme) = uri.scheme() {
        url.push_str(scheme.as_str());
        url.push_str("://")
    }

    if let Some(host) = uri.host() {
        url.push_str(host);
    }

    if let Some(port) = uri.port_u16() {
        if port != 80 && port != 443 {
            let _ = write!(&mut url, ":{}", port);
        }
    }

    url.push_str(uri.path());

    if let Some(query) = uri.query() {
        url.push('?');
        url.push_str(query);
    }

    url
}

#[inline]
pub(super) fn http_method_str(method: &Method) -> Value {
    match method {
        &Method::OPTIONS => "OPTIONS".into(),
        &Method::GET => "GET".into(),
        &Method::POST => "POST".into(),
        &Method::PUT => "PUT".into(),
        &Method::DELETE => "DELETE".into(),
        &Method::HEAD => "HEAD".into(),
        &Method::TRACE => "TRACE".into(),
        &Method::CONNECT => "CONNECT".into(),
        &Method::PATCH => "PATCH".into(),
        other => other.to_string().into(),
    }
}

#[inline]
pub(super) fn http_flavor(version: Version) -> Value {
    match version {
        Version::HTTP_09 => "HTTP/0.9".into(),
        Version::HTTP_10 => "HTTP/1.0".into(),
        Version::HTTP_11 => "HTTP/1.1".into(),
        Version::HTTP_2 => "HTTP/2".into(),
        Version::HTTP_3 => "HTTP/3".into(),
        other => format!("{:?}", other).into(),
    }
}

#[inline]
pub(super) fn http_scheme(scheme: &str) -> Value {
    match scheme {
        "http" => "http".into(),
        "https" => "https".into(),
        other => other.to_string().into(),
    }
}

pub(super) fn trace_attributes_from_request(
    req: &ServiceRequest,
    http_route: &str,
) -> OrderMap<Key, Value> {
    let conn_info = req.connection_info();

    let mut attributes = OrderMap::with_capacity(11);
    attributes.insert(HTTP_METHOD, http_method_str(req.method()));
    attributes.insert(HTTP_FLAVOR, http_flavor(req.version()));
    attributes.insert(HOST_NAME, conn_info.host().to_string().into());
    attributes.insert(HTTP_ROUTE, http_route.to_owned().into());
    attributes.insert(HTTP_SCHEME, http_scheme(conn_info.scheme()));

    let server_name = req.app_config().host();
    if server_name != conn_info.host() {
        attributes.insert(HTTP_SERVER_NAME, server_name.to_string().into());
    }
    if let Some(port) = conn_info
        .host()
        .split_terminator(':')
        .nth(1)
        .and_then(|port| port.parse::<i64>().ok())
    {
        if port != 80 && port != 443 {
            attributes.insert(NET_HOST_PORT, port.into());
        }
    }
    if let Some(path) = req.uri().path_and_query() {
        attributes.insert(HTTP_TARGET, path.as_str().to_string().into());
    }
    if let Some(user_agent) = req
        .headers()
        .get(header::USER_AGENT)
        .and_then(|s| s.to_str().ok())
    {
        attributes.insert(HTTP_USER_AGENT, user_agent.to_string().into());
    }
    let remote_addr = conn_info.realip_remote_addr();
    if let Some(remote) = remote_addr {
        attributes.insert(HTTP_CLIENT_IP, remote.to_string().into());
    }
    if let Some(peer_addr) = req.peer_addr().map(|socket| socket.ip().to_string()) {
        if Some(peer_addr.as_str()) != remote_addr {
            // Client is going through a proxy
            attributes.insert(NET_PEER_IP, peer_addr.into());
        }
    }

    attributes
}

#[cfg(feature = "metrics")]
pub(super) fn metrics_attributes_from_request(
    req: &ServiceRequest,
    http_target: &str,
) -> Vec<KeyValue> {
    let conn_info = req.connection_info();

    let mut attributes = Vec::with_capacity(11);
    attributes.push(KeyValue::new(HTTP_METHOD, http_method_str(req.method())));
    attributes.push(KeyValue::new(HTTP_FLAVOR, http_flavor(req.version())));
    attributes.push(HOST_NAME.string(conn_info.host().to_string()));
    attributes.push(HTTP_TARGET.string(http_target.to_owned()));
    attributes.push(KeyValue::new(HTTP_SCHEME, http_scheme(conn_info.scheme())));

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

    let remote_addr = conn_info.realip_remote_addr();
    if let Some(peer_addr) = req.peer_addr().map(|socket| socket.ip().to_string()) {
        if Some(peer_addr.as_str()) != remote_addr {
            // Client is going through a proxy
            attributes.push(NET_PEER_IP.string(peer_addr))
        }
    }

    attributes
}
