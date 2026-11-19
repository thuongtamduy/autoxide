//! Luật chi tiết của API key (bước 54 P4, như token chi tiết của GitHub): hạn bắt buộc có trần cấu
//! hình, và dải IP được gọi.

use autoxide_kernel::config::IpRange;
use autoxide_kernel::error::FieldError;
use chrono::{DateTime, Duration, Utc};

/// Số dải IP tối đa của một key.
pub(super) const MAX_NETWORKS: usize = 50;

/// Hạn của key: `requested`, hay hôm nay + `max_days` khi không nêu. Quá khứ hay xa hơn trần là
/// lỗi của ô `expires_at`. Mọi key đều hết hạn: key lộ ra không sống mãi.
pub(super) fn expiry(
    requested: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    max_days: i64,
) -> Result<DateTime<Utc>, FieldError> {
    let latest = now + Duration::days(max_days);
    match requested {
        None => Ok(latest),
        Some(at) if at <= now => {
            Err(FieldError::new("expires_at", "past", "must be in the future"))
        }
        Some(at) if at > latest => Err(FieldError::new(
            "expires_at",
            "too_far",
            "must be at most {max_days} days from now",
        )
        .with_param("max_days", max_days)),
        Some(at) => Ok(at),
    }
}

/// Đọc các dải IP người tạo khai; gom mọi ô sai. Rỗng: không giới hạn địa chỉ.
pub(super) fn networks(raw: &[String]) -> Result<Vec<IpRange>, Vec<FieldError>> {
    let mut problems = Vec::new();
    if raw.len() > MAX_NETWORKS {
        problems.push(
            FieldError::new("allowed_ips", "length", "must list at most {max_networks} entries")
                .with_param("max_networks", MAX_NETWORKS),
        );
    }
    let mut ranges = Vec::with_capacity(raw.len());
    for (index, text) in raw.iter().enumerate() {
        match IpRange::parse(text) {
            Ok(range) if !ranges.contains(&range) => ranges.push(range),
            Ok(_) => {}
            Err(_) => problems.push(FieldError::new(
                format!("allowed_ips[{index}]"),
                "invalid",
                "must be an IP address or a range like `203.0.113.0/24`",
            )),
        }
    }
    if problems.is_empty() { Ok(ranges) } else { Err(problems) }
}

/// Dạng lưu trong cột `allowed_ips`: CIDR cách nhau bằng dấu phẩy; `None` khi không giới hạn.
pub(super) fn stored(ranges: &[IpRange]) -> Option<String> {
    (!ranges.is_empty())
        .then(|| ranges.iter().map(ToString::to_string).collect::<Vec<_>>().join(","))
}

/// Đọc lại cột `allowed_ips`. Giá trị hỏng (không thể có qua API) bị bỏ qua từng phần — nhưng nếu
/// cột có chữ mà không đọc được dải nào thì trả một dải không khớp địa chỉ nào: thà từ chối còn
/// hơn mở cho mọi nơi.
pub(super) fn parse_stored(column: Option<&str>) -> Vec<IpRange> {
    let Some(text) = column.filter(|text| !text.trim().is_empty()) else { return Vec::new() };
    let ranges: Vec<IpRange> =
        text.split(',').filter_map(|part| IpRange::parse(part).ok()).collect();
    if ranges.is_empty() {
        tracing::warn!("an API key has unreadable allowed_ips; refusing every address");
        // `0.0.0.0/32` không bao giờ là địa chỉ client.
        return IpRange::parse("0.0.0.0/32").into_iter().collect();
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn han_mac_dinh_va_tran() {
        let now = Utc::now();
        assert_eq!(expiry(None, now, 30).unwrap(), now + Duration::days(30));
        let soon = now + Duration::days(3);
        assert_eq!(expiry(Some(soon), now, 30).unwrap(), soon);
        assert_eq!(expiry(Some(now), now, 30).unwrap_err().code, "past");
        let far = now + Duration::days(31);
        assert_eq!(expiry(Some(far), now, 30).unwrap_err().code, "too_far");
    }

    #[test]
    fn doc_dai_ip_va_luu_lai() {
        let raw =
            vec!["203.0.113.7".to_owned(), " 10.0.0.0/8 ".to_owned(), "10.0.0.0/8".to_owned()];
        let ranges = networks(&raw).unwrap();
        assert_eq!(ranges.len(), 2, "trùng thì bỏ");
        let text = stored(&ranges).unwrap();
        assert_eq!(text, "203.0.113.7/32,10.0.0.0/8");
        assert_eq!(parse_stored(Some(&text)), ranges);
        assert!(stored(&[]).is_none());
        assert!(parse_stored(None).is_empty());

        let bad = networks(&["x".to_owned(), "10.0.0.0/40".to_owned()]).unwrap_err();
        let fields: Vec<&str> = bad.iter().map(|problem| problem.field.as_str()).collect();
        assert_eq!(fields, ["allowed_ips[0]", "allowed_ips[1]"]);
        // Cột hỏng: không mở cho mọi nơi.
        let refuse = parse_stored(Some("garbage"));
        assert_eq!(refuse.len(), 1);
        assert!(!refuse[0].contains("203.0.113.7".parse().unwrap()));
    }
}
