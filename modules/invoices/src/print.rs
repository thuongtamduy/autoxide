//! Mẫu in chứng từ (bước 36 mục 11, D-353): view model cố định, bản HTML dựng bằng bộ dựng mẫu của
//! kernel (`autoxide_kernel::template`) từ mẫu của khách hay mẫu mặc định, bản PDF dựng thẳng từ
//! view model (bố cục cố định) bằng `krilla`.
//!
//! - **View model** (hợp đồng biến của mẫu, [`schema`]): `document`, `seller`, `buyer`, `lines`,
//!   `totals`, `taxes`, `labels` (chữ theo ngôn ngữ, `templates/labels.<ngôn ngữ>.json`), `locale`.
//!   Tiền là `{amount, currency}` (đơn vị nhỏ nhất) — mẫu định dạng bằng bộ lọc `money`.
//! - **Mẫu**: mẫu của nút lập chứng từ → mẫu của cả khách → mẫu mặc định trong repo
//!   (`templates/document.html`, một mẫu cho mọi loại, chữ theo `labels`). Mẫu theo loại chứng từ và
//!   ngôn ngữ.
//! - **PDF**: không có thư viện thuần Rust dựng HTML/CSS đủ tốt (D-353), nên PDF dựng từ CÙNG view
//!   model theo một bố cục cố định (tiêu đề, hai bên, bảng dòng có xuống dòng và sang trang, tổng, chữ
//!   ký) với font Liberation Sans (OFL, đủ dấu tiếng Việt) nhúng tập con. Mẫu HTML của khách không
//!   đổi bản PDF; cần PDF theo mẫu riêng thì in bản HTML từ trình duyệt.

use std::sync::OnceLock;

use autoxide_kernel::error::Error;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::template::{
    TemplateSchema, Variable, format_date, format_money, format_number,
};
use krilla::Document as PdfDocument;
use krilla::geom::{PathBuilder, Point};
use krilla::page::PageSettings;
use krilla::paint::Stroke;
use krilla::text::{Font, GlyphId, KrillaGlyph};
use serde_json::{Value, json};
use skrifa::MetadataProvider;
use skrifa::instance::{LocationRef, Size};

use crate::api::{Document, is_sales_kind};

/// Mẫu mặc định (mọi loại chứng từ, mọi ngôn ngữ).
pub(crate) const DEFAULT_TEMPLATE: &str = include_str!("../templates/document.html");

const REGULAR: &[u8] = include_bytes!("../fonts/LiberationSans-Regular.ttf");
const BOLD: &[u8] = include_bytes!("../fonts/LiberationSans-Bold.ttf");

/// Ngôn ngữ có nhãn: `vi`, còn lại `en`.
pub(crate) fn locale_of(raw: &str) -> &'static str {
    if raw.eq_ignore_ascii_case("vi") || raw.to_ascii_lowercase().starts_with("vi-") {
        "vi"
    } else {
        "en"
    }
}

fn labels(locale: &str) -> &'static Value {
    static EN: OnceLock<Value> = OnceLock::new();
    static VI: OnceLock<Value> = OnceLock::new();
    let parse = |raw: &str| serde_json::from_str(raw).unwrap_or(Value::Null);
    if locale == "vi" {
        VI.get_or_init(|| parse(include_str!("../templates/labels.vi.json")))
    } else {
        EN.get_or_init(|| parse(include_str!("../templates/labels.en.json")))
    }
}

/// Hợp đồng biến của mẫu in.
pub(crate) fn schema() -> TemplateSchema {
    TemplateSchema::new(vec![
        Variable::required(
            "document.number",
            "Number of the document in its series (empty while a draft).",
        ),
        Variable::optional(
            "document",
            "The document: id, kind, status, symbol, number, issue_date, currency, taxed, \
             exchange_rate, lookup_code, authority_code, retention_until.",
        ),
        Variable::optional(
            "seller",
            "Seller (or the unit writing the document): legal_name, tax_code, address.",
        ),
        Variable::optional(
            "buyer",
            "Buyer (or the other party): kind, name, tax_code, address, email.",
        ),
        Variable::optional(
            "lines",
            "Lines: line_no, kind, item_ref, description, unit, quantity, unit_price, discount, net, \
             tax_category, rate_percent, tax, total.",
        ),
        Variable::optional("totals", "Totals: subtotal, tax, total, base_total."),
        Variable::optional("taxes", "Totals per VAT rate: rate_percent, net, tax."),
        Variable::optional(
            "labels",
            "Printed words in the language of the print (titles per kind, column names).",
        ),
        Variable::optional("locale", "Language of the print, `vi` or `en`."),
    ])
}

/// View model của chứng từ `document` theo ngôn ngữ `locale`.
pub(crate) fn view_model(document: &Document, locale: &str) -> Value {
    let mut taxes: Vec<Value> = Vec::new();
    let mut groups: Vec<(Option<String>, i64, i64)> = Vec::new();
    for line in document.lines.iter().filter(|line| line.kind != "note") {
        match groups.iter_mut().find(|group| group.0 == line.rate_percent) {
            Some(group) => {
                group.1 = group.1.saturating_add(line.net.minor());
                group.2 = group.2.saturating_add(line.tax.minor());
            }
            None => groups.push((line.rate_percent.clone(), line.net.minor(), line.tax.minor())),
        }
    }
    let currency = document.subtotal.currency();
    for (rate, net, tax) in groups {
        taxes.push(json!({"rate_percent": rate, "net": Money::new(net, currency), "tax": Money::new(tax, currency)}));
    }
    json!({
        "document": {
            "id": document.id, "kind": document.kind, "status": document.status,
            "symbol": document.symbol, "number": document.number, "issue_date": document.issue_date,
            "currency": document.currency, "taxed": is_sales_kind(&document.kind) && document.tax.is_positive(),
            "prices_include_tax": document.prices_include_tax, "exchange_rate": document.exchange_rate,
            "lookup_code": document.lookup_code, "authority_code": document.authority_code,
            "retention_until": document.retention_until, "relation": document.relation,
        },
        "seller": {
            "legal_name": document.seller["legal_name"], "tax_code": document.seller["tax_code"],
            "address": document.seller["address"],
        },
        "buyer": document.buyer,
        "lines": document.lines,
        "totals": {"subtotal": document.subtotal, "tax": document.tax, "total": document.total,
                   "base_total": document.base_total},
        "taxes": taxes,
        "labels": labels(locale),
        "locale": locale,
    })
}

// ---- PDF ----------------------------------------------------------------------------------------

/// A4 theo điểm PDF (1/72 inch).
const PAGE_W: f32 = 595.0;
const PAGE_H: f32 = 842.0;
const MARGIN: f32 = 40.0;
const SIZE: f32 = 9.0;
const LEADING: f32 = 12.0;

/// Một font: bản cho `krilla` và dữ liệu thô để đo chữ bằng `skrifa`.
struct Face {
    pdf: Font,
    raw: &'static [u8],
}

impl Face {
    fn new(raw: &'static [u8]) -> Result<Self, Error> {
        let pdf = Font::new(raw.into(), 0)
            .ok_or_else(|| Error::internal("cannot read the print font"))?;
        Ok(Face { pdf, raw })
    }

    /// Glyph của `text` (mỗi ký tự một glyph, trái sang phải; bề ngang chuẩn hoá theo em). Đủ cho chữ
    /// Latin có dấu tiếng Việt (font có sẵn glyph dựng sẵn cho mọi chữ có dấu); không ghép chữ, không
    /// kerning.
    fn glyphs(&self, text: &str) -> Vec<KrillaGlyph> {
        let Ok(font) = skrifa::FontRef::new(self.raw) else { return Vec::new() };
        let charmap = font.charmap();
        let metrics = font.glyph_metrics(Size::new(1.0), LocationRef::default());
        text.char_indices()
            .map(|(start, c)| {
                let glyph = charmap.map(c).unwrap_or_default();
                let advance = metrics.advance_width(glyph).unwrap_or(0.5);
                KrillaGlyph::new(
                    GlyphId::new(glyph.to_u32()),
                    advance,
                    0.0,
                    0.0,
                    0.0,
                    start..start + c.len_utf8(),
                    None,
                )
            })
            .collect()
    }

    /// Bề rộng (điểm) của `text` ở cỡ `size` — tổng bề ngang từng glyph, đủ để xuống dòng và căn phải.
    fn width(&self, text: &str, size: f32) -> f32 {
        let Ok(font) = skrifa::FontRef::new(self.raw) else {
            return text.chars().count() as f32 * size * 0.5;
        };
        let charmap = font.charmap();
        let metrics = font.glyph_metrics(Size::new(size), LocationRef::default());
        text.chars()
            .map(|c| {
                charmap.map(c).and_then(|glyph| metrics.advance_width(glyph)).unwrap_or(size * 0.5)
            })
            .sum()
    }

    /// Cắt `text` thành các dòng không rộng quá `max` (theo từ; từ quá dài thì cắt theo ký tự).
    fn wrap(&self, text: &str, size: f32, max: f32) -> Vec<String> {
        let mut lines = Vec::new();
        for paragraph in text.split('\n') {
            let mut current = String::new();
            for word in paragraph.split_whitespace() {
                let candidate =
                    if current.is_empty() { word.to_owned() } else { format!("{current} {word}") };
                if self.width(&candidate, size) <= max {
                    current = candidate;
                    continue;
                }
                if !current.is_empty() {
                    lines.push(std::mem::take(&mut current));
                }
                for c in word.chars() {
                    let next = format!("{current}{c}");
                    if self.width(&next, size) > max && !current.is_empty() {
                        lines.push(std::mem::take(&mut current));
                        current.push(c);
                    } else {
                        current = next;
                    }
                }
            }
            lines.push(current);
        }
        lines
    }
}

/// Chữ một ô: tiền, số theo ngôn ngữ.
fn money_text(value: &Value, locale: &str) -> String {
    let minor = value["amount"].as_str().and_then(|text| text.parse::<i64>().ok());
    let currency = value["currency"].as_str().and_then(|code| Currency::parse(code).ok());
    match (minor, currency) {
        (Some(minor), Some(currency)) => format_money(minor, currency, locale),
        _ => String::new(),
    }
}

fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Một cột của bảng dòng: tiêu đề, bề rộng, căn phải.
struct Column {
    title: String,
    width: f32,
    right: bool,
}

/// Bộ vẽ: trang hiện tại, vị trí dòng (`y` tính từ mép trên), số trang.
struct Painter<'a> {
    regular: &'a Face,
    bold: &'a Face,
    pages: Vec<Vec<Op>>,
    y: f32,
}

/// Một thao tác vẽ (gom theo trang rồi mới vẽ, để biết tổng số trang khi cần).
enum Op {
    Text { x: f32, y: f32, size: f32, bold: bool, text: String },
    Line { x1: f32, y1: f32, x2: f32, y2: f32 },
}

impl<'a> Painter<'a> {
    fn new(regular: &'a Face, bold: &'a Face) -> Self {
        Painter { regular, bold, pages: vec![Vec::new()], y: MARGIN }
    }

    fn page(&mut self) -> &mut Vec<Op> {
        if self.pages.is_empty() {
            self.pages.push(Vec::new());
        }
        let last = self.pages.len() - 1;
        &mut self.pages[last]
    }

    fn new_page(&mut self) {
        self.pages.push(Vec::new());
        self.y = MARGIN;
    }

    /// Còn chỗ cho `height` điểm không; không thì sang trang.
    fn ensure(&mut self, height: f32) -> bool {
        if self.y + height > PAGE_H - MARGIN {
            self.new_page();
            return true;
        }
        false
    }

    fn face(&self, bold: bool) -> &'a Face {
        if bold { self.bold } else { self.regular }
    }

    fn text(&mut self, x: f32, size: f32, bold: bool, text: impl Into<String>) {
        let y = self.y;
        self.page().push(Op::Text { x, y, size, bold, text: text.into() });
    }

    fn centered(&mut self, size: f32, bold: bool, text: &str) {
        let width = self.face(bold).width(text, size);
        let x = ((PAGE_W - width) / 2.0).max(MARGIN);
        self.text(x, size, bold, text);
        self.y += size * 1.4;
    }

    fn rule(&mut self, y: f32, x1: f32, x2: f32) {
        self.page().push(Op::Line { x1, y1: y, x2, y2: y });
    }

    /// Một khối chữ trái, xuống dòng trong `width`.
    fn paragraph(&mut self, x: f32, width: f32, bold: bool, body: &str) {
        for line in self.face(bold).wrap(body, SIZE, width) {
            self.ensure(LEADING);
            self.text(x, SIZE, bold, line);
            self.y += LEADING;
        }
    }

    /// Một hàng của bảng: mỗi ô xuống dòng trong bề rộng cột; hàng cao bằng ô nhiều dòng nhất.
    fn row(&mut self, columns: &[Column], cells: &[String], bold: bool) {
        let face = self.face(bold);
        let wrapped: Vec<Vec<String>> = columns
            .iter()
            .zip(cells)
            .map(|(column, cell)| face.wrap(cell, SIZE, column.width - 6.0))
            .collect();
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1).max(1) as f32 * LEADING + 4.0;
        if self.ensure(height) {
            // Sang trang: lặp lại tiêu đề bảng.
            let titles: Vec<String> = columns.iter().map(|column| column.title.clone()).collect();
            if !bold {
                self.row(columns, &titles, true);
            }
        }
        let top = self.y;
        let mut x = MARGIN;
        for (column, lines) in columns.iter().zip(&wrapped) {
            for (index, line) in lines.iter().enumerate() {
                let tx = if column.right {
                    x + column.width - 3.0 - face.width(line, SIZE)
                } else {
                    x + 3.0
                };
                let y = top + LEADING * (index as f32 + 1.0) - 2.0;
                self.page().push(Op::Text { x: tx, y, size: SIZE, bold, text: line.clone() });
            }
            x += column.width;
        }
        self.y = top + height;
        let right = MARGIN + columns.iter().map(|column| column.width).sum::<f32>();
        self.rule(self.y, MARGIN, right);
    }
}

/// Bản PDF của chứng từ từ view model `view` (của [`view_model`]).
pub(crate) fn render_pdf(view: &Value) -> Result<Vec<u8>, Error> {
    let regular = Face::new(REGULAR)?;
    let bold = Face::new(BOLD)?;
    let locale = view["locale"].as_str().unwrap_or("en");
    let labels = &view["labels"];
    let document = &view["document"];
    let label = |key: &str| text(&labels[key]);
    let mut painter = Painter::new(&regular, &bold);
    let content_width = PAGE_W - 2.0 * MARGIN;

    if document["status"] == "draft" {
        painter.centered(10.0, true, &label("draft"));
    }
    let kind = text(&document["kind"]);
    let title = text(&labels["title"][kind.as_str()]).to_uppercase();
    painter.y += 10.0;
    painter.centered(15.0, true, &title);
    let mut meta = Vec::new();
    if !document["symbol"].is_null() {
        meta.push(format!("{}: {}", label("symbol"), text(&document["symbol"])));
    }
    meta.push(format!("{}: {}", label("number"), text(&document["number"])));
    if let Some(day) = document["issue_date"].as_str() {
        meta.push(format!("{}: {}", label("date"), format_date(day, locale)));
    }
    painter.centered(SIZE, false, &meta.join("  ·  "));
    painter.y += 8.0;

    // Hai bên: người bán / người mua (hay người nhận / nhà cung cấp, người nộp / người nhận tiền).
    let (left, right) = match kind.as_str() {
        "goods_receipt" => (label("receiver"), label("supplier")),
        "cash_receipt" => (label("payee"), label("payer")),
        "cash_payment" => (label("payer"), label("payee")),
        _ => (label("seller"), label("buyer")),
    };
    let party = |name: &str, value: &Value, name_key: &str| {
        let mut body = format!("{name}: {}", text(&value[name_key]));
        if !value["tax_code"].is_null() {
            body.push_str(&format!("\n{}: {}", label("tax_code"), text(&value["tax_code"])));
        }
        if !value["address"].is_null() {
            body.push_str(&format!("\n{}: {}", label("address"), text(&value["address"])));
        }
        body
    };
    let start = painter.y;
    let half = content_width / 2.0 - 10.0;
    painter.paragraph(MARGIN, half, false, &party(&left, &view["seller"], "legal_name"));
    let after_left = painter.y;
    painter.y = start;
    painter.paragraph(
        MARGIN + content_width / 2.0 + 10.0,
        half,
        false,
        &party(&right, &view["buyer"], "name"),
    );
    painter.y = painter.y.max(after_left) + 10.0;

    // Bảng dòng.
    let taxed = document["taxed"].as_bool().unwrap_or(false);
    let mut columns = vec![
        Column { title: label("line_no"), width: 24.0, right: true },
        Column { title: label("description"), width: 0.0, right: false },
        Column { title: label("unit"), width: 36.0, right: false },
        Column { title: label("quantity"), width: 40.0, right: true },
        Column { title: label("unit_price"), width: 72.0, right: true },
        Column { title: label("amount"), width: 80.0, right: true },
    ];
    if taxed {
        columns.push(Column { title: label("vat_rate"), width: 40.0, right: true });
        columns.push(Column { title: label("vat"), width: 66.0, right: true });
    }
    let fixed: f32 = columns.iter().map(|column| column.width).sum();
    columns[1].width = (content_width - fixed).max(80.0);
    painter.rule(painter.y, MARGIN, MARGIN + content_width);
    let titles: Vec<String> = columns.iter().map(|column| column.title.clone()).collect();
    painter.row(&columns, &titles, true);
    for line in view["lines"].as_array().map(Vec::as_slice).unwrap_or_default() {
        let note = line["kind"] == "note";
        let mut cells = vec![
            text(&line["line_no"]),
            text(&line["description"]),
            text(&line["unit"]),
            if note { String::new() } else { format_number(&text(&line["quantity"]), locale) },
            if note { String::new() } else { money_text(&line["unit_price"], locale) },
            if note { String::new() } else { money_text(&line["net"], locale) },
        ];
        if taxed {
            cells.push(
                line["rate_percent"].as_str().map(|rate| format!("{rate}%")).unwrap_or_default(),
            );
            cells.push(if note { String::new() } else { money_text(&line["tax"], locale) });
        }
        painter.row(&columns, &cells, false);
    }

    // Tổng (dòng chữ vẽ trên đường cơ sở `y`: chừa một dòng dưới đường kẻ cuối bảng).
    painter.y += LEADING + 2.0;
    let totals = &view["totals"];
    let mut sums = vec![(label("subtotal"), money_text(&totals["subtotal"], locale), false)];
    if taxed {
        sums.push((label("tax_total"), money_text(&totals["tax"], locale), false));
    }
    sums.push((label("total"), money_text(&totals["total"], locale), true));
    if let Some(rate) = document["exchange_rate"].as_str() {
        sums.push((
            format!("{} {}", label("exchange_rate"), format_number(rate, locale)),
            money_text(&totals["base_total"], locale),
            false,
        ));
    }
    for (name, amount, strong) in sums {
        painter.ensure(LEADING);
        let line = format!("{name}: {amount}");
        let width = painter.face(strong).width(&line, SIZE);
        painter.text(MARGIN + content_width - width, SIZE, strong, line);
        painter.y += LEADING;
    }
    if let Some(code) = document["lookup_code"].as_str() {
        painter.y += 4.0;
        painter.paragraph(
            MARGIN,
            content_width,
            false,
            &format!("{}: {code}", label("lookup_code")),
        );
    }

    // Chữ ký.
    let (first, second) = match kind.as_str() {
        "delivery_note" => (label("sign_receiver"), label("sign_deliverer")),
        "goods_receipt" => (label("sign_deliverer"), label("sign_storekeeper")),
        "cash_receipt" | "cash_payment" => (right.clone(), label("sign_cashier")),
        _ => (label("sign_buyer"), label("sign_seller")),
    };
    painter.ensure(60.0);
    painter.y += 24.0;
    for (index, name) in [first, second].iter().enumerate() {
        let center = MARGIN + content_width * (0.25 + 0.5 * index as f32);
        let note = label("sign_note");
        let top = painter.y;
        let width = bold.width(name, SIZE);
        painter.text(center - width / 2.0, SIZE, true, name.clone());
        painter.y = top + LEADING;
        let width = regular.width(&note, 8.0);
        painter.text(center - width / 2.0, 8.0, false, note);
        painter.y = top;
    }

    draw(painter.pages, &regular, &bold, &label("page"))
}

/// Vẽ các trang đã gom thành file PDF (kèm số trang ở chân trang).
fn draw(
    pages: Vec<Vec<Op>>,
    regular: &Face,
    bold: &Face,
    page_word: &str,
) -> Result<Vec<u8>, Error> {
    let mut pdf = PdfDocument::new();
    let total = pages.len();
    let settings =
        PageSettings::from_wh(PAGE_W, PAGE_H).ok_or_else(|| Error::internal("bad page size"))?;
    for (index, ops) in pages.into_iter().enumerate() {
        let mut page = pdf.start_page_with(settings.clone());
        let mut surface = page.surface();
        for op in ops {
            match op {
                Op::Text { x, y, size, bold: strong, text } => {
                    let face = if strong { bold } else { regular };
                    let glyphs = face.glyphs(&text);
                    // Chữ chỉ tô, không viền (viền làm chữ đậm lên).
                    surface.set_stroke(None);
                    surface.draw_glyphs(
                        Point::from_xy(x, y),
                        &glyphs,
                        face.pdf.clone(),
                        &text,
                        size,
                        false,
                    );
                }
                Op::Line { x1, y1, x2, y2 } => {
                    surface.set_stroke(Some(Stroke { width: 0.5, ..Stroke::default() }));
                    let mut path = PathBuilder::new();
                    path.move_to(x1, y1);
                    path.line_to(x2, y2);
                    if let Some(path) = path.finish() {
                        surface.draw_path(&path);
                    }
                }
            }
        }
        let footer = format!("{page_word} {}/{total}", index + 1);
        let width = regular.width(&footer, 8.0);
        surface.draw_glyphs(
            Point::from_xy(PAGE_W - MARGIN - width, PAGE_H - MARGIN / 2.0),
            &regular.glyphs(&footer),
            regular.pdf.clone(),
            &footer,
            8.0,
            false,
        );
        surface.finish();
        page.finish();
    }
    pdf.finish().map_err(|error| Error::internal(format!("pdf: {error:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_tieng_viet_nhieu_trang() {
        let mut document = crate::templates::sample_document("retail_receipt");
        let line = document.lines[0].clone();
        document.lines = (1..=120)
            .map(|index| {
                let mut item = line.clone();
                item.line_no = index;
                item.description = format!(
                    "Áo thun cổ tròn màu xanh dương, cỡ L — dòng thứ {index} của hoá đơn bán lẻ"
                );
                item
            })
            .collect();
        let bytes = render_pdf(&view_model(&document, "vi")).unwrap();
        assert!(bytes.starts_with(b"%PDF"));
        // 120 dòng không vừa một trang A4: có trang thứ hai.
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            text.matches("/Type /Page").count() >= 3 || text.matches("/Type/Page").count() >= 3,
            "pages"
        );
    }

    #[test]
    fn xuong_dong_theo_be_rong() {
        let face = Face::new(REGULAR).unwrap();
        let lines = face.wrap("Hoá đơn giá trị gia tăng cho khách hàng thân thiết", SIZE, 60.0);
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|line| face.width(line, SIZE) <= 60.0 || !line.contains(' ')));
    }
}
