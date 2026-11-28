//! Trần của số liệu hàng người gọi gửi (cân nặng, kích thước, số lượng) — P17 của bước 70.
//!
//! Trước đây cân nặng và kích thước không có trần: cộng / nhân bão hoà về `i64::MAX` nên không
//! panic, nhưng bậc phí và cân nặng gửi hãng thành số vô nghĩa thay vì báo lỗi. Nay đầu vào có trần
//! (như `catalog`: 10⁷ gram, 10⁷ mm, số lượng 10⁷), cân tính phí của một lần giao có trần
//! [`MAX_PARCEL_WEIGHT_GRAMS`], và phép tính trên đó dùng `i128` rồi so trần — không còn bão hoà.

use autoxide_kernel::error::{Error, FieldError};

use crate::api::{MAX_PARCEL_WEIGHT_GRAMS, MAX_SIDE_MM};
use crate::errors;

/// Cân nặng một món (gram), như trần của `catalog`.
pub(crate) const MAX_ITEM_WEIGHT_GRAMS: i64 = 10_000_000;
/// Số lượng một dòng, như trần của `catalog`.
pub(crate) const MAX_QUANTITY: i64 = 10_000_000;

/// Kiểm số lượng, cân nặng và kích thước của một món ở `prefix` (vd `items[0]`). Số lượng `0` được
/// (dòng không giao, bỏ qua lúc tính); số âm thì nơi gọi đã kiểm theo luật riêng của nó.
pub(crate) fn check_item(
    problems: &mut Vec<FieldError>,
    prefix: &str,
    quantity: i64,
    weight_grams: Option<i64>,
    sides: [Option<i64>; 3],
) {
    if quantity > MAX_QUANTITY {
        problems.push(at_most(format!("{prefix}.quantity"), MAX_QUANTITY));
    }
    if weight_grams.is_some_and(|grams| !(0..=MAX_ITEM_WEIGHT_GRAMS).contains(&grams)) {
        problems.push(at_most(format!("{prefix}.weight_grams"), MAX_ITEM_WEIGHT_GRAMS));
    }
    check_sides(problems, prefix, sides);
}

/// Kiểm ba cạnh (mm): mỗi cạnh có thì `0..=MAX_SIDE_MM`.
pub(crate) fn check_sides(problems: &mut Vec<FieldError>, prefix: &str, sides: [Option<i64>; 3]) {
    for (name, side) in ["length_mm", "width_mm", "height_mm"].into_iter().zip(sides) {
        if side.is_some_and(|mm| !(0..=MAX_SIDE_MM).contains(&mm)) {
            problems.push(at_most(format!("{prefix}.{name}"), MAX_SIDE_MM));
        }
    }
}

fn at_most(field: String, max: i64) -> FieldError {
    FieldError::new(field, "range", "must be 0 to {max}").with_param("max", max)
}

/// Gram tính bằng `i128` (không tràn với đầu vào đã kiểm) → `i64` nếu không vượt trần một lần giao.
pub(crate) fn within_load(grams: i128) -> Result<i64, Error> {
    i64::try_from(grams)
        .ok()
        .filter(|grams| *grams <= MAX_PARCEL_WEIGHT_GRAMS)
        .ok_or_else(|| errors::load_too_large(MAX_PARCEL_WEIGHT_GRAMS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mon_vuot_tran_bao_dung_truong() {
        let mut problems = Vec::new();
        check_item(&mut problems, "items[1]", i64::MAX, Some(i64::MAX), [Some(1), None, Some(-1)]);
        let fields: Vec<_> = problems.iter().map(|problem| problem.field.clone()).collect();
        assert_eq!(
            fields,
            ["items[1].quantity", "items[1].weight_grams", "items[1].height_mm"].map(String::from)
        );
        let mut problems = Vec::new();
        check_item(
            &mut problems,
            "items[0]",
            MAX_QUANTITY,
            Some(MAX_ITEM_WEIGHT_GRAMS),
            [Some(MAX_SIDE_MM), Some(0), None],
        );
        assert!(problems.is_empty());
    }

    #[test]
    fn tai_vuot_tran_la_loi_co_ma() {
        assert_eq!(
            within_load(i128::from(MAX_PARCEL_WEIGHT_GRAMS)).unwrap(),
            MAX_PARCEL_WEIGHT_GRAMS
        );
        let error = within_load(i128::from(MAX_PARCEL_WEIGHT_GRAMS) + 1).unwrap_err();
        assert_eq!(error.code(), "shipping.load_too_large");
        assert_eq!(within_load(i128::MAX).unwrap_err().code(), "shipping.load_too_large");
    }
}
