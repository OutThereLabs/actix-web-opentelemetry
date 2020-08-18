//! # Route Formatter
//!
//! Format routes from paths.
use regex::Regex;

/// A regular expression for matching UUIDs.
pub const UUID_REGEX: &str =
    r"[0-9a-fA-F]{8}\-[0-9a-fA-F]{4}\-[0-9a-fA-F]{4}\-[0-9a-fA-F]{4}\-[0-9a-fA-F]{12}";

/// Interface for formatting routes from paths
///
/// # Examples
///
/// Using the built in regex route formatter:
///
/// ```
/// use regex::Regex;
/// use actix_web_opentelemetry::{RouteFormatter, RegexFormatter, UUID_REGEX};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let uuid_formatter = RegexFormatter::new(UUID_REGEX, "*")?;
/// let uuid_path = "/users/4f5accfe-45d2-43b1-bf10-fdad708732a8";
/// assert_eq!(uuid_formatter.format(uuid_path), "/users/*".to_string());
///
/// let numeric_formatter = RegexFormatter::new(r"\d+", ":id")?;
/// assert_eq!(numeric_formatter.format("/users/123"), "/users/:id".to_string());
/// # Ok(())
/// # }
/// ```
///
/// Or create your own custom formatter:
///
/// ```
/// use actix_web_opentelemetry::RouteFormatter;
///
/// // A formatter to ensure all paths are reported as lowercase.
/// struct MyLowercaseFormatter;
///
/// impl RouteFormatter for MyLowercaseFormatter {
///     fn format(&self, path: &str) -> String {
///         path.to_lowercase()
///     }
/// }
///
/// // now a request with path `/USERS/123` would be recorded as `/users/123`
/// ```
pub trait RouteFormatter {
    /// Function from path to route.
    /// e.g. /users/123 -> /users/:id
    fn format(&self, path: &str) -> String;
}

/// A route formatter that uses a regular expression to replace path components.
///
/// # Examples
///
/// ```
/// use regex::Regex;
/// use actix_web_opentelemetry::{RouteFormatter, RegexFormatter, UUID_REGEX};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let uuid_formatter = RegexFormatter::new(UUID_REGEX, "*")?;
/// let uuid_path = "/users/4f5accfe-45d2-43b1-bf10-fdad708732a8";
/// assert_eq!(uuid_formatter.format(uuid_path), "/users/*".to_string());
///
/// let numeric_formatter = RegexFormatter::new(r"\d+", ":id")?;
/// assert_eq!(numeric_formatter.format("/users/123"), "/users/:id".to_string());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct RegexFormatter {
    regex: Regex,
    replacement: &'static str,
}

impl RegexFormatter {
    /// Create a new `RegexFormatter`
    pub fn new(re: &str, replacement: &'static str) -> Result<Self, regex::Error> {
        let regex = Regex::new(re)?;
        Ok(RegexFormatter { regex, replacement })
    }
}

impl RouteFormatter for RegexFormatter {
    fn format(&self, path: &str) -> String {
        self.regex.replace_all(path, self.replacement).into_owned()
    }
}

/// A formatter that passes the path through unchanged.
#[derive(Clone, Debug, Default)]
pub struct PassThroughFormatter;

impl RouteFormatter for PassThroughFormatter {
    fn format(&self, path: &str) -> String {
        path.to_string()
    }
}

/// UUID wildcard formatter replaces UUIDs with asterisks.
#[derive(Clone, Debug)]
pub struct UuidWildcardFormatter {
    formatter: RegexFormatter,
}

impl Default for UuidWildcardFormatter {
    fn default() -> Self {
        UuidWildcardFormatter {
            formatter: RegexFormatter::new(UUID_REGEX, "*").unwrap(),
        }
    }
}

impl UuidWildcardFormatter {
    /// Create a new `UuidWildcardFormatter`
    #[deprecated = "please use RegexFormatter instead"]
    pub fn new() -> Self {
        UuidWildcardFormatter::default()
    }
}

impl RouteFormatter for UuidWildcardFormatter {
    fn format(&self, path: &str) -> String {
        self.formatter.format(path)
    }
}
