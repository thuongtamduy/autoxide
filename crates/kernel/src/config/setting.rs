//! Describes a configuration variable.

/// A configuration variable, declared with `const` next to the code that uses it.
///
/// Declare it ONCE and it serves four purposes: reading the value, reporting errors, printing it in the
/// `autoxide config` command, and generating the `.env.example` file. This way the configuration
/// documentation cannot drift from the code.
///
/// ```
/// use autoxide_kernel::config::Setting;
///
/// const HTTP_BIND: Setting =
///     Setting::new("AUTOXIDE_HTTP_BIND", "Address the HTTP server listens on.")
///         .with_default("127.0.0.1:8080");
/// assert_eq!(HTTP_BIND.default, Some("127.0.0.1:8080"));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Setting {
    /// Environment variable name. The industry-standard name if there is one (`DATABASE_URL`), otherwise
    /// prefixed with `AUTOXIDE_`.
    pub name: &'static str,
    /// Description for the installer, in English, one or two sentences.
    pub description: &'static str,
    /// Value used when the variable is not set. `None` means it must be set
    /// (or it is an optional variable - see [`ConfigReader::optional`](super::ConfigReader::optional)).
    pub default: Option<&'static str>,
    /// The value is a secret: never printed in error messages or by the
    /// `autoxide config` command.
    pub secret: bool,
    /// Illustrative value for `.env.example` when there is no default.
    pub example: Option<&'static str>,
}

impl Setting {
    // `const fn`: gọi được lúc biên dịch, nên `Setting` khai được bằng `const`.
    pub const fn new(name: &'static str, description: &'static str) -> Self {
        Setting { name, description, default: None, secret: false, example: None }
    }

    pub const fn with_default(mut self, default: &'static str) -> Self {
        self.default = Some(default);
        self
    }

    pub const fn secret(mut self) -> Self {
        self.secret = true;
        self
    }

    pub const fn with_example(mut self, example: &'static str) -> Self {
        self.example = Some(example);
        self
    }
}
