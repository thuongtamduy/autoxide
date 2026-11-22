//! Dạng request của SePay eInvoice API v1, đúng như tài liệu công khai (xem đầu `lib.rs`), dựng từ
//! `EInvoice` của kernel. Không có I/O ở đây.

use autoxide_kernel::einvoice::{
    EInvoice, EInvoiceKind, EInvoiceLine, Party, Relation, not_supported, rejected,
};
use autoxide_kernel::error::Error;
use autoxide_kernel::money::Currency;
use serde_json::{Map, Number, Value, json};

use crate::api::PROVIDER;

/// Mã mẫu hoá đơn của SePay: `1` — hoá đơn giá trị gia tăng, `2` — hoá đơn bán hàng (trang "Chi
/// tiết tài khoản nhà cung cấp").
pub(crate) fn template_code(kind: EInvoiceKind) -> Result<&'static str, Error> {
    match kind {
        EInvoiceKind::Vat => Ok("1"),
        EInvoiceKind::Sales => Ok("2"),
        _ => Err(not_supported(PROVIDER)),
    }
}

/// Mã tham chiếu của hoá đơn ở SePay: ID chứng từ. SePay chặn mã đã có hoá đơn
/// (`409 EINVOICE_DOCUMENT_EXISTED`), nên gửi lại cùng chứng từ không tạo hoá đơn thứ hai.
pub(crate) fn reference_code(invoice: &EInvoice) -> String {
    invoice.document_id.to_string()
}

/// `provider_ref` lưu ở `invoices`: `<reference_code>|<tracking_code>` — tra chi tiết theo mã
/// tham chiếu, hỏi trạng thái và tải file theo mã tracking. Mã tracking rỗng khi hoá đơn đã có từ
/// một lần gửi trước mà câu trả lời bị mất.
pub(crate) fn provider_ref(reference: &str, tracking: &str) -> String {
    format!("{reference}|{tracking}")
}

/// Tách `provider_ref` thành (mã tham chiếu, mã tracking nếu có).
pub(crate) fn split_provider_ref(provider_ref: &str) -> (&str, Option<&str>) {
    match provider_ref.split_once('|') {
        Some((reference, tracking)) => (reference, Some(tracking).filter(|t| !t.is_empty())),
        None => (provider_ref, None),
    }
}

/// Đường dẫn và các trường riêng của loại yêu cầu: hoá đơn mới, thay thế hay điều chỉnh.
pub(crate) fn endpoint(invoice: &EInvoice) -> Result<(&'static str, Map<String, Value>), Error> {
    let mut extra = Map::new();
    let Some(related) = &invoice.related else {
        return Ok(("/v1/invoices/create", extra));
    };
    // Hoá đơn gốc phải do chính bộ nối này gửi: mã tham chiếu của nó nằm trong `provider_ref`.
    let original = related
        .provider_ref
        .as_deref()
        .map(|reference| split_provider_ref(reference).0.to_owned())
        .filter(|reference| !reference.is_empty())
        .ok_or_else(|| rejected("the original invoice was not issued through SePay eInvoice"))?;
    extra.insert("original_invoice_reference_code".to_owned(), Value::String(original));
    match related.relation {
        Relation::Replacement => Ok(("/v1/invoices/replace", extra)),
        Relation::Adjustment => {
            extra.insert("adjustment_type".to_owned(), json!(adjustment_type(&invoice.lines)));
            Ok(("/v1/invoices/adjust", extra))
        }
    }
}

/// `increase` / `decrease` / `combined` theo dấu tiền của các dòng; không dòng nào đổi tiền: `info`.
fn adjustment_type(lines: &[EInvoiceLine]) -> &'static str {
    let up = lines.iter().any(|line| line.total.is_positive());
    let down = lines.iter().any(|line| line.total.is_negative());
    match (up, down) {
        (true, true) => "combined",
        (true, false) => "increase",
        (false, true) => "decrease",
        (false, false) => "info",
    }
}

/// Thuế suất SePay nhận: `0`, `5`, `8`, `10`; `-2` là không chịu thuế.
fn tax_rate(rate_percent: Option<&str>) -> Result<i64, Error> {
    let Some(raw) = rate_percent else { return Ok(-2) };
    let raw = raw.trim();
    // `"8.00"` và `"8"` là một: bỏ số 0 thừa sau dấu chấm.
    let rate =
        if raw.contains('.') { raw.trim_end_matches('0').trim_end_matches('.') } else { raw };
    match rate {
        "0" => Ok(0),
        "5" => Ok(5),
        "8" => Ok(8),
        "10" => Ok(10),
        _ => Err(rejected(format!("SePay eInvoice does not take the tax rate {raw}%"))),
    }
}

/// Số lượng (chữ thập phân của kernel) thành số JSON; SePay nhận tối đa 2 chữ số thập phân.
fn quantity(raw: &str) -> Result<Number, Error> {
    let trimmed = raw.trim();
    let decimals = trimmed.split_once('.').map_or(0, |(_, fraction)| fraction.len());
    let plain = trimmed.trim_start_matches('-');
    let well_formed = !plain.is_empty()
        && plain.chars().all(|c| c.is_ascii_digit() || c == '.')
        && plain.matches('.').count() <= 1;
    if !well_formed || decimals > 2 {
        return Err(rejected(format!(
            "SePay eInvoice takes quantities with at most 2 decimal places, not {raw}"
        )));
    }
    serde_json::from_str(trimmed)
        .map_err(|_| rejected(format!("the quantity {raw} is not a number")))
}

fn buyer(party: &Party) -> Value {
    let mut buyer = Map::new();
    // Có mã số thuế: người mua là tổ chức.
    let kind = if party.tax_code.is_some() { "company" } else { "personal" };
    buyer.insert("type".to_owned(), json!(kind));
    buyer.insert("name".to_owned(), json!(party.name));
    for (field, value) in
        [("tax_code", &party.tax_code), ("address", &party.address), ("email", &party.email)]
    {
        if let Some(value) = value {
            buyer.insert(field.to_owned(), json!(value));
        }
    }
    Value::Object(buyer)
}

fn item(line: &EInvoiceLine, sales: bool) -> Result<Value, Error> {
    let mut item = Map::new();
    item.insert("line_number".to_owned(), json!(line.line_no));
    item.insert("line_type".to_owned(), json!(1));
    item.insert("item_name".to_owned(), json!(line.description));
    if let Some(unit) = &line.unit {
        item.insert("unit".to_owned(), json!(unit));
    }
    item.insert("quantity".to_owned(), Value::Number(quantity(&line.quantity)?));
    item.insert("unit_price".to_owned(), json!(line.unit_price.minor()));
    // Hoá đơn bán hàng (mẫu 2): SePay bỏ qua thuế suất và tiền thuế, nên không gửi.
    if !sales {
        item.insert("tax_rate".to_owned(), json!(tax_rate(line.rate_percent.as_deref())?));
        item.insert("tax_amount".to_owned(), json!(line.tax.minor()));
    }
    Ok(Value::Object(item))
}

/// Thân request của `create` / `replace` / `adjust` cho `invoice`, với tài khoản nhà cung cấp
/// `provider_account` và ký hiệu `series` lấy từ SePay (API "chi tiết tài khoản" — không viết cứng:
/// phần `C{YY}` phải khớp năm hiện tại).
pub(crate) fn request_body(
    invoice: &EInvoice,
    provider_account: &str,
    series: &str,
    extra: Map<String, Value>,
) -> Result<Value, Error> {
    // Tài liệu: VND, USD, CAD. Bộ nối này chỉ gửi đồng Việt Nam (số nguyên đồng, không quy đổi).
    if invoice.currency != Currency::VND {
        return Err(not_supported(PROVIDER));
    }
    let template = template_code(invoice.kind)?;
    let sales = invoice.kind == EInvoiceKind::Sales;
    let items =
        invoice.lines.iter().map(|line| item(line, sales)).collect::<Result<Vec<_>, _>>()?;
    let total: i64 = invoice.totals.iter().map(|total| total.net.minor() + total.tax.minor()).sum();
    let payment_method = match invoice.payment_method.as_str() {
        known @ ("TM" | "CK" | "TM/CK") => known,
        _ => "KHAC",
    };
    let mut body = Map::new();
    body.insert("provider_account_id".to_owned(), json!(provider_account));
    body.insert("reference_code".to_owned(), json!(reference_code(invoice)));
    // Tài liệu: `YYYY-MM-DD HH:mm:ss`, chỉ nhận ngày hiện tại.
    body.insert("issued_date".to_owned(), json!(format!("{} 00:00:00", invoice.issue_date)));
    body.insert("currency".to_owned(), json!("VND"));
    body.insert("payment_method".to_owned(), json!(payment_method));
    // Phải là boolean thật; `false`: xuất và phát hành luôn.
    body.insert("is_draft".to_owned(), json!(false));
    body.insert("buyer".to_owned(), buyer(&invoice.buyer));
    body.insert("items".to_owned(), Value::Array(items));
    if invoice.related.is_none() {
        body.insert("template_code".to_owned(), json!(template));
        body.insert("invoice_series".to_owned(), json!(series));
    }
    if !invoice.totals.is_empty() {
        body.insert("total_amount".to_owned(), json!(total));
    }
    body.extend(extra);
    Ok(Value::Object(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thue_suat_theo_bang_cua_sepay() {
        for (raw, wanted) in [
            (None, -2),
            (Some("0"), 0),
            (Some("0.00"), 0),
            (Some("5"), 5),
            (Some("8.0"), 8),
            (Some("10"), 10),
            (Some("10.00"), 10),
        ] {
            assert_eq!(tax_rate(raw).unwrap(), wanted, "{raw:?}");
        }
        for bad in ["3.5", "1", "100", "-2", "abc", ""] {
            assert_eq!(tax_rate(Some(bad)).unwrap_err().code(), "einvoice.rejected", "{bad}");
        }
    }

    #[test]
    fn so_luong_toi_da_hai_chu_so_thap_phan() {
        assert_eq!(quantity("2").unwrap().to_string(), "2");
        assert_eq!(quantity("1.5").unwrap().to_string(), "1.5");
        assert_eq!(quantity("-0.25").unwrap().to_string(), "-0.25");
        for bad in ["1.234", "", "1e3", "1.2.3", "abc", "--1"] {
            assert!(quantity(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn tach_provider_ref() {
        assert_eq!(split_provider_ref("ref-1|trk-1"), ("ref-1", Some("trk-1")));
        assert_eq!(split_provider_ref("ref-1|"), ("ref-1", None));
        assert_eq!(split_provider_ref("ref-1"), ("ref-1", None));
        assert_eq!(provider_ref("a", "b"), "a|b");
    }
}
