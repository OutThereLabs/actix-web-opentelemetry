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

/// Resource ID formatter replaces resource ids with `:id`.
#[derive(Clone, Debug, Default)]
pub struct ResourceIdFormatter {}

impl ResourceIdFormatter {
    /// Create a new `ResourceIdFormatter`
    pub fn new() -> Self {
        ResourceIdFormatter {}
    }
}

impl RouteFormatter for ResourceIdFormatter {
    /// Function from path to route
    /// e.g. /users/123 -> /users/:id
    fn format(&self, uri: &str) -> String {
        lazy_static::lazy_static! {
            static ref RESOURCE_ID: Regex = Regex::new(r"/[0-9]+").expect("invalid routeformatter regex");
        }
        RESOURCE_ID.replace_all(uri, "/:id").into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replace_resource_id() {
        let formatter = ResourceIdFormatter::new();

        assert_eq!(formatter.format("/users/123"), String::from("/users/:id"));
        assert_eq!(
            formatter.format("/users/123/foo/5"),
            String::from("/users/:id/foo/:id")
        )
    }
}
