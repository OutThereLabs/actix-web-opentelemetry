use std::borrow::Cow;

use actix_http::header;
use actix_web::{
    dev::ServiceRequest,
    http::{Method, Version},
};
use opentelemetry::KeyValue;
use opentelemetry_semantic_conventions::trace::{
    HTTP_CLIENT_IP, HTTP_FLAVOR, HTTP_HOST, HTTP_METHOD, HTTP_ROUTE, HTTP_SCHEME, HTTP_SERVER_NAME,
    HTTP_TARGET, HTTP_USER_AGENT, NET_HOST_PORT, NET_PEER_IP,
};

#[inline]
pub(super) fn http_method_str(method: &Method) -> Cow<'static, str> {
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
pub(super) fn http_flavor(version: Version) -> Cow<'static, str> {
    match version {
        Version::HTTP_09 => "0.9".into(),
        Version::HTTP_10 => "1.0".into(),
        Version::HTTP_11 => "1.1".into(),
        Version::HTTP_2 => "2.0".into(),
        Version::HTTP_3 => "3.0".into(),
        other => format!("{:?}", other).into(),
    }
}

#[inline]
pub(super) fn http_scheme(scheme: &str) -> Cow<'static, str> {
    match scheme {
        "http" => "http".into(),
        "https" => "https".into(),
        other => other.to_string().into(),
    }
}

pub(super) fn trace_attributes_from_request(
    req: &ServiceRequest,
    http_route: &str,
) -> Vec<KeyValue> {
    let conn_info = req.connection_info();

    let mut attributes = Vec::with_capacity(11);
    attributes.push(HTTP_METHOD.string(http_method_str(req.method())));
    attributes.push(HTTP_FLAVOR.string(http_flavor(req.version())));
    attributes.push(HTTP_HOST.string(conn_info.host().to_string()));
    attributes.push(HTTP_ROUTE.string(http_route.to_owned()));
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

    attributes
}
