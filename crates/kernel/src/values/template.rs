//! Bộ dựng mẫu dùng chung `autoxide_kernel::template` (bước 37 mục 2, D-206, D-351): MỘT ngôn ngữ mẫu
//! cho cả dự án (cú pháp Jinja qua `minijinja`), cho mẫu do khách tự soạn — mẫu in chứng từ (bước 36),
//! mẫu mail và thông báo (bước 37, sau).
//!
//! Mẫu là dữ liệu KHÔNG tin cậy (khách sửa được), nên bộ dựng là một hộp cát:
//!
//! - Không đọc file, không gọi mạng, không nạp mẫu khác: bật `minijinja` không có `multi_template`
//!   (`include` / `extends` / `import` là lỗi cú pháp), không có loader; chỉ có hàm và bộ lọc dựng sẵn
//!   của `minijinja` (thuần tính toán) cùng ba bộ lọc của kernel (`money`, `number`, `date`).
//! - Giới hạn: độ dài mẫu (`AUTOXIDE_TEMPLATE_MAX_BYTES`, mặc định 64 KiB), số bước chạy ("fuel" của
//!   `minijinja`, cũng chặn bộ nhớ), độ sâu lồng nhau, độ dài kết quả. Vượt → `template.limit_exceeded`
//!   (422), không treo worker.
//! - Bản HTML: mọi giá trị chèn vào đều được thoát HTML; bộ lọc `safe` bị gỡ — dữ liệu người dùng
//!   không bao giờ thành HTML. Phần HTML là chữ của chính mẫu.
//! - Lưu mẫu thì kiểm với **schema biến** của loại mẫu ([`TemplateSchema`]): cú pháp, chỉ dùng biến
//!   có trong schema, đủ biến bắt buộc — gom mọi lỗi báo một lần (D-083) với dòng, cột.
//!
//! Kernel chỉ dựng chữ, không biết khái niệm chứng từ hay đơn hàng: dữ liệu do module đưa vào.

use crate::time::Date;
use std::collections::BTreeSet;
use std::io::Write;

use minijinja::value::{Serde, Value as JinjaValue};
use minijinja::{AutoEscape, Environment, ErrorKind as JinjaErrorKind};

use crate::config::{ConfigReader, Invalid, Setting};
use crate::error::{Error, ErrorKind, FieldError};
use crate::money::Currency;

/// Độ dài tối đa của một mẫu, tính bằng byte.
pub const MAX_BYTES: Setting = Setting::new(
    "AUTOXIDE_TEMPLATE_MAX_BYTES",
    "Largest template (print layouts, mail and notification templates written by tenants), in bytes \
     (1024 to 1048576).",
)
.with_default("65536");

/// Số bước chạy tối đa của một lần dựng (fuel của `minijinja`: khoảng một lệnh máy ảo mỗi đơn vị,
/// và khoảng 64 byte bộ nhớ). Đủ cho chứng từ vài nghìn dòng.
const FUEL: u64 = 2_000_000;
/// Độ sâu lồng nhau tối đa (`for` trong `for`, `if` trong `if`…).
const MAX_DEPTH: usize = 50;
/// Kết quả lớn hơn chừng này byte → `template.limit_exceeded`.
const MAX_OUTPUT: usize = 4 * 1024 * 1024;

/// Đọc `AUTOXIDE_TEMPLATE_MAX_BYTES` (module `kernel` gọi một lần lúc cấu hình).
pub(crate) fn read_setting(reader: &mut ConfigReader<'_>) -> Result<usize, Invalid> {
    reader.get_with(&MAX_BYTES, |raw| crate::config::parse_in_range(raw, 1024usize, 1_048_576))
}

/// Cách thoát giá trị chèn vào mẫu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    /// Bản HTML: mọi giá trị được thoát HTML.
    Html,
    /// Bản chữ thuần (mail dạng chữ, SMS): không thoát.
    Text,
}

/// Một biến mẫu được dùng: đường dẫn chấm (`document.number`, `lines`), bắt buộc hay không.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    /// Đường dẫn: `buyer.name`. Khai một nút cha (`document`) cho phép mọi thuộc tính con của nó.
    pub path: &'static str,
    /// Mẫu phải dùng biến này (vd link đặt lại mật khẩu của mail).
    pub required: bool,
    /// Ý nghĩa, tiếng Anh, cho người soạn mẫu (API trả kèm schema).
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

/// Hợp đồng biến của một loại mẫu: những biến mẫu được dùng.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TemplateSchema {
    pub variables: Vec<Variable>,
}

impl TemplateSchema {
    pub fn new(variables: Vec<Variable>) -> Self {
        TemplateSchema { variables }
    }

    /// `used` (đường dẫn chấm) có được phép không: đúng một biến đã khai, nằm dưới một biến đã khai,
    /// hay là cha của một biến đã khai (vd `lines` khi `lines` được khai, `document` khi chỉ khai
    /// `document.number`).
    fn allows(&self, used: &str) -> bool {
        self.variables.iter().any(|variable| {
            let declared = variable.path;
            used == declared
                || used.strip_prefix(declared).is_some_and(|rest| rest.starts_with('.'))
                || declared.strip_prefix(used).is_some_and(|rest| rest.starts_with('.'))
        })
    }

    /// Biến bắt buộc `path` có được dùng không (chính nó hay một thuộc tính bên dưới).
    fn satisfied(path: &str, used: &BTreeSet<String>) -> bool {
        used.iter().any(|item| {
            item == path || item.strip_prefix(path).is_some_and(|rest| rest.starts_with('.'))
        })
    }
}

/// Bộ dựng mẫu có giới hạn. Rẻ để clone; module lấy bằng `context.require::<TemplateEngine>()`.
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

/// Dòng, cột (từ 1) của byte `offset` trong `source`.
fn position(source: &str, offset: usize) -> (usize, usize) {
    let before = source.get(..offset).unwrap_or(source);
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |tail| tail.chars().count()) + 1;
    (line, column)
}

/// Vị trí lần đầu `path` xuất hiện như một từ trong mẫu (gần đúng: để chỉ chỗ cho người soạn).
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

/// Bộ ghi dừng khi kết quả vượt `MAX_OUTPUT`.
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

/// Nhóm hàng nghìn của phần nguyên `digits` bằng `separator`.
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

/// Số thập phân dạng chữ (`-1234567.5`) theo ngôn ngữ: `vi` → `-1.234.567,5`, khác → `-1,234,567.5`.
/// Chữ không phải số thì trả nguyên.
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

/// Tiền (số nguyên đơn vị nhỏ nhất + mã tiền) theo ngôn ngữ: `1000000 VND` → `1.000.000 ₫` (`vi`),
/// `1,000,000 VND` (khác); `1999 USD` → `19.99 USD`.
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

/// Ngày `YYYY-MM-DD` (hay thời điểm RFC 3339) theo ngôn ngữ: `vi` → `dd/mm/yyyy`, khác giữ ISO.
pub fn format_date(text: &str, locale: &str) -> String {
    let day = text.get(..10).unwrap_or(text);
    match Date::parse_from_str(day, "%Y-%m-%d") {
        Ok(date) if locale == "vi" => date.format("%d/%m/%Y").to_string(),
        Ok(date) => date.format("%Y-%m-%d").to_string(),
        Err(_) => text.to_owned(),
    }
}

impl TemplateEngine {
    /// Bộ dựng với độ dài mẫu tối đa `max_bytes`.
    pub fn new(max_bytes: usize) -> Self {
        TemplateEngine { max_bytes }
    }

    /// Độ dài mẫu tối đa (byte).
    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    /// Môi trường hộp cát cho một lần dựng: bộ lọc của kernel theo `locale`, không có `safe`.
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

    /// Mẫu của khách không được tắt thoát HTML: minijinja không cho bỏ thẻ `{% autoescape %}` khỏi
    /// cú pháp, nên chặn mọi khối mở bằng nó (kể cả `{%-` / `{%+`), lúc kiểm và lúc dựng.
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

    /// Kiểm mẫu `source` (của trường `field`) với `schema`: độ dài, cú pháp, biến lạ, biến bắt buộc.
    /// Mọi lỗi gom vào một `validation_failed`; mỗi lỗi có `code` `template.syntax_error`,
    /// `template.unknown_variable` hay `template.required_variable_missing` và vị trí dòng, cột.
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

    /// Dựng `source` với dữ liệu `context` (JSON), thoát theo `output`, định dạng số / tiền / ngày
    /// theo `locale` (`vi`, `en`…). Lỗi lúc chạy (sai kiểu, vượt giới hạn) là lỗi của mẫu: 422.
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

/// Vị trí (byte) của khối `{% autoescape … %}` đầu tiên trong `source`, nếu có.
fn autoescape_block(source: &str) -> Option<usize> {
    source.match_indices("{%").find_map(|(start, _)| {
        let rest = source[start + 2..].trim_start_matches(['-', '+']).trim_start();
        let word: String = rest.chars().take_while(char::is_ascii_alphanumeric).collect();
        (word == "autoescape").then_some(start)
    })
}

/// Mẫu không chạy được (sai kiểu, bộ lọc lạ…): 422, kèm dòng và lý do.
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
