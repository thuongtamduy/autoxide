//! Khung mail HTML chung (D-206, D-365): viết bằng MJML, dựng bằng `mrml` để hiển thị đúng trên
//! Outlook, Gmail, điện thoại. Khách chỉnh logo, màu, chân thư bằng dữ liệu (`notifications_branding`),
//! không sửa MJML. Mail nào cũng kèm bản chữ thuần (phần `text` của `Mail`).
//!
//! Nội dung thư (đã dựng và đã thoát HTML bởi bộ dựng mẫu của kernel) KHÔNG đi qua bộ phân tích MJML:
//! khung được dựng với một chỗ giữ chỗ rồi thay bằng nội dung — HTML do khách viết không làm hỏng
//! được khung, và giá trị thương hiệu được kiểm (màu `#rrggbb`, logo `https://`) rồi thoát XML trước
//! khi vào MJML.

use std::collections::HashMap;

use autoxide_kernel::error::Error;

/// Chỗ giữ chỗ nội dung trong khung (chỉ chữ và số: MJML giữ nguyên).
const BODY_SLOT: &str = "AUTOXIDEBODYSLOT7F3A";
/// Chỗ giữ chỗ chân thư huỷ nhận (tin quảng cáo).
const FOOTER_SLOT: &str = "AUTOXIDEFOOTERSLOT7F3A";

/// Thương hiệu đã kiểm, dùng cho khung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Brand {
    pub name: Option<String>,
    pub logo_url: Option<String>,
    pub primary_color: String,
    pub background_color: String,
    /// Dòng chân thư: chữ của khách, rồi tên và địa chỉ chủ sở hữu (hồ sơ pháp lý của `compliance`).
    pub footer_lines: Vec<String>,
}

pub(crate) const DEFAULT_PRIMARY: &str = "#1f6feb";
pub(crate) const DEFAULT_BACKGROUND: &str = "#f4f5f7";

impl Default for Brand {
    fn default() -> Self {
        Brand {
            name: None,
            logo_url: None,
            primary_color: DEFAULT_PRIMARY.to_owned(),
            background_color: DEFAULT_BACKGROUND.to_owned(),
            footer_lines: Vec::new(),
        }
    }
}

/// `#rrggbb`.
pub(crate) fn valid_color(raw: &str) -> bool {
    raw.len() == 7 && raw.starts_with('#') && raw[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Thoát chữ để đặt trong HTML / XML (cả thuộc tính).
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Bản HTML từ chữ thuần: thoát HTML, mỗi đoạn (cách bằng dòng trống) một `<p>`, xuống dòng là `<br>`.
pub(crate) fn text_to_html(text: &str) -> String {
    text.split("\n\n")
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| format!("<p>{}</p>", escape(part).replace('\n', "<br>")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Mail HTML hoàn chỉnh: khung MJML có thương hiệu, nội dung `body_html`, chân thư huỷ nhận
/// `footer_html` (tin quảng cáo; khung tự chèn, mẫu không gỡ được).
pub(crate) fn mail_html(
    brand: &Brand,
    subject: &str,
    body_html: &str,
    footer_html: Option<&str>,
) -> Result<String, Error> {
    let primary = if valid_color(&brand.primary_color) {
        brand.primary_color.as_str()
    } else {
        DEFAULT_PRIMARY
    };
    let background = if valid_color(&brand.background_color) {
        brand.background_color.as_str()
    } else {
        DEFAULT_BACKGROUND
    };
    let header = match (&brand.logo_url, &brand.name) {
        (Some(logo), name) => format!(
            r#"<mj-image src="{}" alt="{}" width="160px" align="left" padding="0" />"#,
            escape(logo),
            escape(name.as_deref().unwrap_or_default())
        ),
        (None, Some(name)) => format!(
            r#"<mj-text font-size="20px" font-weight="bold" color="{primary}" padding="0">{}</mj-text>"#,
            escape(name)
        ),
        (None, None) => String::new(),
    };
    let footer_lines: String =
        brand.footer_lines.iter().map(|line| format!("<p>{}</p>", escape(line))).collect();
    let footer_slot = if footer_html.is_some() { FOOTER_SLOT } else { "" };
    let source = format!(
        r##"<mjml lang="en">
  <mj-head>
    <mj-title>{title}</mj-title>
    <mj-attributes>
      <mj-all font-family="Arial, Helvetica, sans-serif" />
      <mj-text font-size="15px" line-height="1.5" color="#1f2328" />
    </mj-attributes>
  </mj-head>
  <mj-body background-color="{background}">
    <mj-section padding="24px 16px 8px">
      <mj-column>{header}</mj-column>
    </mj-section>
    <mj-section background-color="#ffffff" border-top="4px solid {primary}" padding="24px 16px">
      <mj-column>
        <mj-text>{BODY_SLOT}</mj-text>
      </mj-column>
    </mj-section>
    <mj-section padding="8px 16px 24px">
      <mj-column>
        <mj-text font-size="12px" color="#57606a">{footer_lines}{footer_slot}</mj-text>
      </mj-column>
    </mj-section>
  </mj-body>
</mjml>"##,
        title = escape(subject),
    );
    let parsed = mrml::mjml::Mjml::parse(&source)
        .map_err(|error| Error::internal(format!("the mail layout does not parse: {error:?}")))?;
    // Không font ngoài (không tải gì từ Google Fonts), không chú thích của mrml.
    let options = mrml::prelude::render::RenderOptions {
        disable_comments: true,
        social_icon_origin: None,
        fonts: HashMap::new(),
    };
    let html = parsed
        .element
        .render(&options)
        .map_err(|error| Error::internal(format!("the mail layout does not render: {error:?}")))?;
    let html = html.replacen(BODY_SLOT, body_html, 1);
    Ok(match footer_html {
        Some(footer) => html.replacen(FOOTER_SLOT, footer, 1),
        None => html,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_dung_tren_mau_mjml() {
        let brand = Brand {
            name: Some("Shop <A>".to_owned()),
            logo_url: Some("https://cdn.example/logo.png?a=1&b=2".to_owned()),
            primary_color: "#ff0000".to_owned(),
            background_color: "bad".to_owned(),
            footer_lines: vec!["Shop A Co., 1 Street".to_owned()],
        };
        let html =
            mail_html(&brand, "Hi", "<p>Body &amp; more</p>", Some("<a href=\"u\">Stop</a>"))
                .unwrap();
        assert!(html.contains("<p>Body &amp; more</p>"), "{html}");
        assert!(html.contains("<a href=\"u\">Stop</a>"));
        assert!(html.contains("#ff0000"));
        assert!(html.contains(DEFAULT_BACKGROUND));
        assert!(html.contains("logo.png?a=1&amp;b=2"));
        assert!(html.contains("Shop A Co., 1 Street"));
        assert!(!html.contains(BODY_SLOT) && !html.contains(FOOTER_SLOT));
        assert!(!html.contains("fonts.googleapis"));
        // Bản HTML từ chữ thuần thoát mọi thẻ.
        assert_eq!(text_to_html("a <b>\nc\n\nd"), "<p>a &lt;b&gt;<br>c</p>\n<p>d</p>");
    }
}
