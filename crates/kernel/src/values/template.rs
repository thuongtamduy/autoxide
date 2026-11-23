//! The shared template engine `autoxide_kernel::template` (step 37 item 2, D-206, D-351): ONE template
//! language for the whole project (Jinja syntax via `minijinja`), for templates written by tenants themselves
//! — document print templates (step 36), mail and notification templates (step 37, later).
//!
//! Templates are UNTRUSTED data (tenants can edit them), so the engine is a sandbox:
//!
//! - No reading files, no network calls, no loading other templates: `minijinja` is enabled without
//!   `multi_template` (`include` / `extends` / `import` are syntax errors), with no loader; only
//!   `minijinja`'s built-in functions and filters (pure computation) plus the kernel's three filters
//!   (`money`, `number`, `date`).
//! - Limits: template length (`AUTOXIDE_TEMPLATE_MAX_BYTES`, default 64 KiB), number of execution steps
//!   (`minijinja`'s "fuel", which also bounds memory), nesting depth, output length. Exceeding →
//!   `template.limit_exceeded` (422), never hanging a worker.
//! - HTML output: every inserted value is HTML-escaped; the `safe` filter is removed — user data
//!   never becomes HTML. The HTML parts are the template's own text.
//! - Saving a template validates it against the template kind's **variable schema** ([`TemplateSchema`]):
//!   syntax, only variables in the schema, all required variables present — every error is collected and
//!   reported at once (D-083) with line and column.
//!
//! The kernel only builds text and knows no document or order concepts: the data comes from modules.

use crate::time::Date;
use std::collections::BTreeSet;
use std::io::Write;

use minijinja::value::{Serde, Value as JinjaValue};
use minijinja::{AutoEscape, Environment, ErrorKind as JinjaErrorKind};

use crate::config::{ConfigReader, Invalid, Setting};
use crate::error::{Error, ErrorKind, FieldError};
use crate::money::Currency;

/// Maximum length of one template, in bytes.
pub const MAX_BYTES: Setting = Setting::new(
    "AUTOXIDE_TEMPLATE_MAX_BYTES",
    "Largest template (print layouts, mail and notification templates written by tenants), in bytes \
     (1024 to 1048576).",
)
.with_default("65536");

/// Maximum number of execution steps for one render (`minijinja`'s fuel: roughly one VM instruction per unit,
/// and about 64 bytes of memory). Enough for a document of a few thousand lines.
const FUEL: u64 = 2_000_000;
/// Maximum nesting depth (`for` in `for`, `if` in `if`…).
const MAX_DEPTH: usize = 50;
/// A result larger than this many bytes → `template.limit_exceeded`.
const MAX_OUTPUT: usize = 4 * 1024 * 1024;

/// Reads `AUTOXIDE_TEMPLATE_MAX_BYTES` (the `kernel` module calls it once during configuration).
pub(crate) fn read_setting(reader: &mut ConfigReader<'_>) -> Result<usize, Invalid> {
    reader.get_with(&MAX_BYTES, |raw| crate::config::parse_in_range(raw, 1024usize, 1_048_576))
}

/// How values inserted into a template are escaped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    /// HTML output: every value is HTML-escaped.
    Html,
    /// Plain-text output (text mail, SMS): no escaping.
    Text,
}

/// A template variable that may be used: a dotted path (`document.number`, `lines`), required or not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    /// Path: `buyer.name`. Declaring a parent node (`document`) allows all of its child properties.
    pub path: &'static str,
    /// The template must use this variable (e.g. the password-reset link of a mail).
    pub required: bool,
    /// Meaning, in English, for the template author (the API returns it with the schema).
    pub description: &'static str,
}

impl Variable {
    pub const fn optional(path: &'static str, description: &'static str) -> Self {
        Variable { path, required: false, description }
    }

    pub const fn required(path: &'static str, description: &'static str) -> Self {
        Variable { path, required: true, description }
    }
}

/// Variable contract of a template kind: the variables a template may use.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TemplateSchema {
    pub variables: Vec<Variable>,
}

impl TemplateSchema {
    pub fn new(variables: Vec<Variable>) -> Self {
        TemplateSchema { variables }
    }

    /// Whether `used` (a dotted path) is allowed: exactly a declared variable, under a declared variable,
    /// or a parent of a declared variable (e.g. `lines` when `lines` is declared, `document` when only
    /// `document.number` is declared).
    fn allows(&self, used: &str) -> bool {
        self.variables.iter().any(|variable| {
            let declared = variable.path;
            used == declared
                || used.strip_prefix(declared).is_some_and(|rest| rest.starts_with('.'))
                || declared.strip_prefix(used).is_some_and(|rest| rest.starts_with('.'))
        })
    }

    /// Whether the required variable `path` is used (itself or a property beneath it).
    fn satisfied(path: &str, used: &BTreeSet<String>) -> bool {
        used.iter().any(|item| {
            item == path || item.strip_prefix(path).is_some_and(|rest| rest.starts_with('.'))
        })
    }
}

/// A template engine with limits. Cheap to clone; modules get it with `context.require::<TemplateEngine>()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateEngine {
    max_bytes: usize,
}

impl Default for TemplateEngine {
    fn default() -> Self {
        TemplateEngine { max_bytes: 65_536 }
    }
}

fn limit_exceeded(what: &'static str, limit: usize) -> Error {
    Error::new(
        ErrorKind::Validation,
        "template.limit_exceeded",
        "The template goes over a limit ({what}, at most {limit}). Make it shorter or simpler.",
    )
    .with_param("what", what)
    .with_param("limit", limit)
}

/// Line and column (from 1) of byte `offset` in `source`.
fn position(source: &str, offset: usize) -> (usize, usize) {
    let before = source.get(..offset).unwrap_or(source);
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |tail| tail.chars().count()) + 1;
    (line, column)
}

/// Position where `path` first appears as a word in the template (approximate: to point the author to the spot).
fn first_use(source: &str, path: &str) -> (usize, usize) {
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '.';
    let mut start = 0;
    while let Some(found) = source.get(start..).and_then(|rest| rest.find(path)) {
        let at = start + found;
        let before = source.get(..at).and_then(|head| head.chars().next_back());
        let after = source.get(at + path.len()..).and_then(|tail| tail.chars().next());
        if !before.is_some_and(is_word) && !after.is_some_and(|c| is_word(c) && c != '.') {
            return position(source, at);
        }
        start = at + path.len();
    }
    (1, 1)
}

/// The writer stops when the output exceeds `MAX_OUTPUT`.
struct Capped {
    out: Vec<u8>,
    over: bool,
}

impl Write for Capped {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.out.len() + buf.len() > MAX_OUTPUT {
            self.over = true;
            return Err(std::io::Error::other("template output too large"));
        }
        self.out.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Groups the thousands of the integer part `digits` with `separator`.
fn group(digits: &str, separator: char) -> String {
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(separator);
        }
        out.push(digit);
    }
    out
}

/// A decimal number as text (`-1234567.5`) by language: `vi` → `-1.234.567,5`, other → `-1,234,567.5`.
/// Text that is not a number is returned unchanged.
pub fn format_number(text: &str, locale: &str) -> String {
    let (sign, body) = text.strip_prefix('-').map_or(("", text), |rest| ("-", rest));
    let (whole, fraction) = body.split_once('.').unwrap_or((body, ""));
    let digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    if whole.is_empty() || !digits(whole) || !digits(fraction) {
        return text.to_owned();
    }
    let (thousands, decimal) = if locale == "vi" { ('.', ',') } else { (',', '.') };
    let mut out = format!("{sign}{}", group(whole, thousands));
    if !fraction.is_empty() {
        out.push(decimal);
        out.push_str(fraction);
    }
    out
}

/// Money (integer in the smallest unit + currency code) by language: `1000000 VND` → `1.000.000 ₫` (`vi`),
/// `1,000,000 VND` (other); `1999 USD` → `19.99 USD`.
pub fn format_money(minor: i64, currency: Currency, locale: &str) -> String {
    let digits = u32::from(currency.minor_units());
    let magnitude = minor.unsigned_abs();
    let sign = if minor < 0 { "-" } else { "" };
    let text = if digits == 0 {
        format!("{sign}{magnitude}")
    } else {
        let scale = 10u64.pow(digits);
        let width = digits as usize;
        format!("{sign}{}.{:0width$}", magnitude / scale, magnitude % scale)
    };
    let number = format_number(&text, locale);
    if locale == "vi" && currency == Currency::VND {
        format!("{number} ₫")
    } else {
        format!("{number} {}", currency.code())
    }
}

/// A `YYYY-MM-DD` date (or RFC 3339 instant) by language: `vi` → `dd/mm/yyyy`, other keeps ISO.
pub fn format_date(text: &str, locale: &str) -> String {
    let day = text.get(..10).unwrap_or(text);
    match Date::parse_from_str(day, "%Y-%m-%d") {
        Ok(date) if locale == "vi" => date.format("%d/%m/%Y").to_string(),
        Ok(date) => date.format("%Y-%m-%d").to_string(),
        Err(_) => text.to_owned(),
    }
}

impl TemplateEngine {
    /// An engine with maximum template length `max_bytes`.
    pub fn new(max_bytes: usize) -> Self {
        TemplateEngine { max_bytes }
    }

    /// Maximum template length (bytes).
    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    /// The sandbox environment for one render: the kernel's filters per `locale`, without `safe`.
    fn environment(output: Output, locale: &str) -> Environment<'static> {
        let mut env = Environment::new();
        env.set_fuel(Some(FUEL));
        env.set_recursion_limit(MAX_DEPTH);
        env.set_auto_escape_callback(move |_| match output {
            Output::Html => AutoEscape::Html,
            Output::Text => AutoEscape::None,
        });
        // Không có đường nào biến dữ liệu thành HTML không thoát.
        env.remove_filter("safe");
        env.remove_global("debug");
        let locale = locale.to_owned();
        let (money, number, date) = (locale.clone(), locale.clone(), locale);
        env.add_filter("money", move |value: JinjaValue| -> String {
            let amount = value.get_attr("amount").ok();
            let currency = value.get_attr("currency").ok();
            let minor = amount.as_ref().and_then(|amount| {
                amount.as_str().and_then(|text| text.parse::<i64>().ok()).or(amount.as_i64())
            });
            let currency = currency
                .as_ref()
                .and_then(JinjaValue::as_str)
                .and_then(|code| Currency::parse(code).ok());
            match (minor, currency) {
                (Some(minor), Some(currency)) => format_money(minor, currency, &money),
                _ => value.to_string(),
            }
        });
        env.add_filter("number", move |value: JinjaValue| -> String {
            format_number(&value.to_string(), &number)
        });
        env.add_filter("date", move |value: JinjaValue| -> String {
            format_date(&value.to_string(), &date)
        });
        env
    }

    fn check_size(&self, source: &str) -> Result<(), Error> {
        if source.len() > self.max_bytes {
            return Err(limit_exceeded("template size in bytes", self.max_bytes));
        }
        Ok(())
    }

    /// A tenant's template must not turn off HTML escaping: minijinja does not allow removing the
    /// `{% autoescape %}` tag from the syntax, so every block opened with it is rejected (including `{%-` /
    /// `{%+`), both when validating and when rendering.
    fn check_autoescape(field: &str, source: &str) -> Result<(), Error> {
        let Some(offset) = autoescape_block(source) else { return Ok(()) };
        let (line, column) = position(source, offset);
        Err(Error::validation(vec![
            FieldError::new(
                field,
                "template.autoescape_forbidden",
                "uses `autoescape` at line {line}, column {column}: templates cannot turn off \
                 HTML escaping",
            )
            .with_param("line", line)
            .with_param("column", column),
        ]))
    }

    /// Validates template `source` (of field `field`) against `schema`: length, syntax, unknown variables,
    /// required variables. Every error is collected into one `validation_failed`; each error has `code`
    /// `template.syntax_error`, `template.unknown_variable` or `template.required_variable_missing` and the
    /// line and column position.
    pub fn validate(
        &self,
        field: &str,
        source: &str,
        schema: &TemplateSchema,
    ) -> Result<(), Error> {
        self.check_size(source)?;
        Self::check_autoescape(field, source)?;
        let env = Self::environment(Output::Html, "en");
        let template = match env.template_from_str(source) {
            Ok(template) => template,
            Err(error) => {
                let (line, column) = match error.range() {
                    Some(range) => position(source, range.start),
                    None => (error.line().unwrap_or(1), 1),
                };
                let reason = error.detail().map_or_else(|| error.kind().to_string(), str::to_owned);
                return Err(Error::validation(vec![
                    FieldError::new(
                        field,
                        "template.syntax_error",
                        "has a syntax error at line {line}, column {column}: {reason}",
                    )
                    .with_param("line", line)
                    .with_param("column", column)
                    .with_param("reason", reason),
                ]));
            }
        };
        // Hàm dựng sẵn của môi trường (`range`, `dict`, `namespace`…) không phải biến của dữ liệu.
        let globals: BTreeSet<&str> = env.globals().map(|(name, _)| name).collect();
        let used: BTreeSet<String> = template
            .undeclared_variables(true)
            .into_iter()
            .filter(|path| !globals.contains(path.split('.').next().unwrap_or_default()))
            .collect();
        let mut problems = Vec::new();
        for path in &used {
            if !schema.allows(path) {
                let (line, column) = first_use(source, path);
                problems.push(
                    FieldError::new(
                        field,
                        "template.unknown_variable",
                        "uses `{name}` at line {line}, column {column}, which this kind of template does not have",
                    )
                    .with_param("name", path)
                    .with_param("line", line)
                    .with_param("column", column),
                );
            }
        }
        for variable in schema.variables.iter().filter(|variable| variable.required) {
            if !TemplateSchema::satisfied(variable.path, &used) {
                problems.push(
                    FieldError::new(
                        field,
                        "template.required_variable_missing",
                        "must use `{name}`: this kind of template needs it",
                    )
                    .with_param("name", variable.path),
                );
            }
        }
        if problems.is_empty() { Ok(()) } else { Err(Error::validation(problems)) }
    }

    /// Renders `source` with data `context` (JSON), escaping per `output`, formatting numbers / money / dates
    /// per `locale` (`vi`, `en`…). A runtime error (wrong type, limit exceeded) is the template's fault: 422.
    pub fn render(
        &self,
        source: &str,
        output: Output,
        locale: &str,
        context: &serde_json::Value,
    ) -> Result<String, Error> {
        self.check_size(source)?;
        Self::check_autoescape("template", source)?;
        let env = Self::environment(output, locale);
        let template = env.template_from_str(source).map_err(|error| render_failed(&error))?;
        let mut sink = Capped { out: Vec::new(), over: false };
        let result = template.render_captured_to(JinjaValue::from(Serde(context)), &mut sink);
        if sink.over {
            return Err(limit_exceeded("output size in bytes", MAX_OUTPUT));
        }
        if let Err(error) = result {
            return Err(match error.kind() {
                JinjaErrorKind::OutOfFuel => limit_exceeded("steps", FUEL as usize),
                _ if error.to_string().contains("recursion") => {
                    limit_exceeded("nesting depth", MAX_DEPTH)
                }
                _ => render_failed(&error),
            });
        }
        String::from_utf8(sink.out).map_err(Error::internal)
    }
}

/// Position (byte) of the first `{% autoescape … %}` block in `source`, if any.
fn autoescape_block(source: &str) -> Option<usize> {
    source.match_indices("{%").find_map(|(start, _)| {
        let rest = source[start + 2..].trim_start_matches(['-', '+']).trim_start();
        let word: String = rest.chars().take_while(char::is_ascii_alphanumeric).collect();
        (word == "autoescape").then_some(start)
    })
}

/// A template that cannot run (wrong type, unknown filter…): 422, with line and reason.
fn render_failed(error: &minijinja::Error) -> Error {
    Error::new(
        ErrorKind::Validation,
        "template.render_failed",
        "The template cannot be rendered at line {line}: {reason}",
    )
    .with_param("line", error.line().unwrap_or(1))
    .with_param("reason", error.detail().map_or_else(|| error.kind().to_string(), str::to_owned))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> TemplateSchema {
        TemplateSchema::new(vec![
            Variable::required("document.number", "Number"),
            Variable::optional("buyer", "Buyer"),
            Variable::optional("lines", "Lines"),
            Variable::optional("total", "Total"),
        ])
    }

    #[test]
    fn ham_dung_san_khong_tinh_la_bien() {
        let engine = TemplateEngine::default();
        let source = "{% for i in range(3) %}{{ document.number }}{% endfor %}";
        engine.validate("body", source, &schema()).unwrap();
    }

    #[test]
    fn thoat_html_moi_gia_tri_va_khong_co_safe() {
        let engine = TemplateEngine::default();
        let data = json!({"buyer": {"name": "<script>alert(1)</script>"}});
        let html = engine.render("<b>{{ buyer.name }}</b>", Output::Html, "en", &data).unwrap();
        assert_eq!(html, "<b>&lt;script&gt;alert(1)&lt;&#x2f;script&gt;</b>");
        let error = engine.render("{{ buyer.name|safe }}", Output::Html, "en", &data).unwrap_err();
        assert_eq!(error.code(), "template.render_failed");
        let text = engine.render("{{ buyer.name }}", Output::Text, "en", &data).unwrap();
        assert_eq!(text, "<script>alert(1)</script>");
    }

    #[test]
    fn khong_tat_duoc_thoat_html() {
        let engine = TemplateEngine::default();
        let data = json!({"buyer": {"name": "<script>alert(1)</script>"}});
        for source in [
            "{% autoescape false %}{{ buyer.name }}{% endautoescape %}",
            "{%- autoescape 'none' -%}{{ buyer.name }}{% endautoescape %}",
            "{{ document.number }}\n  {%+   autoescape false %}x{% endautoescape %}",
        ] {
            let error = engine.render(source, Output::Html, "en", &data).unwrap_err();
            assert_eq!(error.fields()[0].code, "template.autoescape_forbidden", "{source}");
            let error = engine.validate("body", source, &schema()).unwrap_err();
            assert_eq!(error.fields()[0].code, "template.autoescape_forbidden", "{source}");
        }
        assert_eq!(autoescape_block("{{ x }}\n{% autoescape true %}"), Some(8));
        assert_eq!(autoescape_block("{% if autoescape %}{% endif %}"), None);
    }

    #[test]
    fn khong_doc_file_khong_nap_mau_khac() {
        let engine = TemplateEngine::default();
        for source in
            ["{% include 'x.html' %}", "{% extends '/etc/passwd' %}", "{% import 'a' as b %}"]
        {
            let error = engine.validate("body", source, &schema()).unwrap_err();
            assert_eq!(error.fields()[0].code, "template.syntax_error", "{source}");
        }
    }

    #[test]
    fn gom_moi_loi_bien_mot_lan_kem_vi_tri() {
        let engine = TemplateEngine::default();
        let source = "Hi\n{{ password }} {{ buyer.name }}\n{{ secret.key }}";
        let error = engine.validate("body", source, &schema()).unwrap_err();
        let codes: Vec<(&str, String)> =
            error.fields().iter().map(|item| (item.code.as_str(), item.message.clone())).collect();
        assert_eq!(codes.len(), 3, "{codes:?}");
        assert!(codes.iter().any(|(code, message)| *code == "template.unknown_variable"
            && message.contains("`password` at line 2, column 4")));
        assert!(codes.iter().any(|(code, message)| *code == "template.unknown_variable"
            && message.contains("`secret.key` at line 3")));
        assert!(codes.iter().any(|(code, _)| *code == "template.required_variable_missing"));
        // Biến vòng lặp không phải biến lạ; dùng `document.number` thì đủ.
        let good = "{{ document.number }}{% for line in lines %}{{ line.description }}{% endfor %}";
        engine.validate("body", good, &schema()).unwrap();
    }

    #[test]
    fn loi_cu_phap_co_dong_cot() {
        let engine = TemplateEngine::default();
        let error = engine.validate("body", "a\n{% if %}", &schema()).unwrap_err();
        assert_eq!(error.fields()[0].code, "template.syntax_error");
        assert!(error.fields()[0].message.contains("line 2"), "{}", error.fields()[0].message);
    }

    #[test]
    fn gioi_han_kich_thuoc_buoc_chay_va_ket_qua() {
        let engine = TemplateEngine::new(1024);
        let big = "x".repeat(2000);
        assert_eq!(
            engine.validate("body", &big, &schema()).unwrap_err().code(),
            "template.limit_exceeded"
        );
        let engine = TemplateEngine::default();
        let endless =
            "{% for a in range(100000) %}{% for b in range(100000) %}{% endfor %}{% endfor %}";
        let error = engine.render(endless, Output::Text, "en", &json!({})).unwrap_err();
        assert_eq!(error.code(), "template.limit_exceeded");
        let wide = "{% for a in range(10000) %}{{ 'x' * 1000 }}{% endfor %}";
        let error = engine.render(wide, Output::Text, "en", &json!({})).unwrap_err();
        assert_eq!(error.code(), "template.limit_exceeded");
    }

    #[test]
    fn dinh_dang_tien_so_ngay_theo_ngon_ngu() {
        let engine = TemplateEngine::default();
        let data = json!({"total": {"amount": "1234567", "currency": "VND"},
                          "usd": {"amount": "-199950", "currency": "USD"},
                          "qty": "1234.5", "day": "2026-11-17"});
        let source = "{{ total|money }}|{{ usd|money }}|{{ qty|number }}|{{ day|date }}";
        assert_eq!(
            engine.render(source, Output::Text, "vi", &data).unwrap(),
            "1.234.567 ₫|-1.999,50 USD|1.234,5|17/11/2026"
        );
        assert_eq!(
            engine.render(source, Output::Text, "en", &data).unwrap(),
            "1,234,567 VND|-1,999.50 USD|1,234.5|2026-11-17"
        );
    }
}
