//! # Route Formatter
//!
//! Format routes from paths.
use regex::Regex;

/// Interface for formatting routes from paths
pub trait RouteFormatter {
    /// Function from path to route.
    /// e.g. /users/123 -> /users/:id
    fn format(&self, uri: &str) -> String;
}

/// UUID wildcard formatter replaces UUIDs with asterisks.
#[derive(Clone, Debug, Default)]
pub struct UuidWildcardFormatter {}

impl UuidWildcardFormatter {
    /// Create a new `UuidWildcardFormatter`
    pub fn new() -> Self {
        UuidWildcardFormatter {}
    }
}

impl RouteFormatter for UuidWildcardFormatter {
    /// Function from path to route
    /// e.g. /users/4f5accfe-45d2-43b1-bf10-fdad708732a8 -> /users/*
    fn format(&self, uri: &str) -> String {
        lazy_static::lazy_static! {
            static ref UUID: Regex = Regex::new(r"[0-9a-fA-F]{8}\-[0-9a-fA-F]{4}\-[0-9a-fA-F]{4}\-[0-9a-fA-F]{4}\-[0-9a-fA-F]{12}").unwrap();
        }
        UUID.replace_all(uri, "*").into_owned()
    }
}
