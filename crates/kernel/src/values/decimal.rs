//! Số thập phân cho tính toán tiền, thuế, tỉ giá (bước 36 mục 1, D-188, D-322).
//!
//! Tiền LƯU vẫn là [`Money`] — số nguyên đơn vị nhỏ nhất (D-180). [`Decimal`] chỉ dùng khi TÍNH:
//! thuế suất `0.08`, tỉ giá `25345.5`, số lượng `1.5` kg. Ba luật:
//!
//! - **Không bao giờ đi qua `f64`**: số thập phân đọc từ chuỗi, ghi ra chuỗi.
//! - **Làm tròn có chủ đích**: mỗi chỗ đổi `Decimal` về `Money` nêu một [`Rounding`]; không có quy
//!   tắc mặc định ngầm.
//! - **Lưu và gửi dạng chữ**: cột database là chuỗi đã chuẩn hoá ([`to_column`] / [`from_column`]),
//!   không dùng kiểu thập phân của database (SQLite không có); JSON là chuỗi ([`DecimalString`]),
//!   vì JavaScript đọc số JSON thành `f64`.

use std::fmt;
use std::str::FromStr;

use rust_decimal::RoundingStrategy;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{ObjectBuilder, Schema, Type};
use utoipa::{PartialSchema, ToSchema};

use crate::error::{Error, ErrorKind};
use crate::money::{Currency, Money};

pub use rust_decimal::Decimal;

/// Dạng chữ của một số thập phân ở API: `-` tuỳ chọn, chữ số, phần lẻ tuỳ chọn sau dấu chấm.
pub const PATTERN: &str = "^-?[0-9]+(\\.[0-9]+)?$";

/// Số chữ số tối đa `rust_decimal` giữ được (mantissa 96 bit).
const MAX_DIGITS: usize = 28;

/// Quy tắc làm tròn khi đổi một số thập phân về số đơn vị nhỏ nhất. Mỗi chỗ dùng chọn một (bảng
/// làm tròn của đặc tả bước 36): thuế GTGT `HalfUp`, chiết khấu thường `Down`…
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Rounding {
    /// Nửa thì làm tròn ra xa số 0: 2.5 → 3, −2.5 → −3.
    HalfUp,
    /// Nửa thì làm tròn về số chẵn (làm tròn ngân hàng): 2.5 → 2, 3.5 → 4.
    HalfEven,
    /// Bỏ phần lẻ, về phía số 0: 2.9 → 2, −2.9 → −2.
    Down,
    /// Có phần lẻ thì ra xa số 0: 2.1 → 3, −2.1 → −3.
    Up,
    /// Về phía âm vô cùng: 2.9 → 2, −2.1 → −3.
    Floor,
    /// Về phía dương vô cùng: 2.1 → 3, −2.9 → −2.
    Ceiling,
}

impl Rounding {
    fn strategy(self) -> RoundingStrategy {
        match self {
            Rounding::HalfUp => RoundingStrategy::MidpointAwayFromZero,
            Rounding::HalfEven => RoundingStrategy::MidpointNearestEven,
            Rounding::Down => RoundingStrategy::ToZero,
            Rounding::Up => RoundingStrategy::AwayFromZero,
            Rounding::Floor => RoundingStrategy::ToNegativeInfinity,
            Rounding::Ceiling => RoundingStrategy::ToPositiveInfinity,
        }
    }
}

/// `value` làm tròn tới `places` chữ số sau dấu chấm theo `rounding`.
pub fn round(value: Decimal, places: u32, rounding: Rounding) -> Decimal {
    value.round_dp_with_strategy(places, rounding.strategy())
}

/// Đọc đầu vào của người gọi: `"12.5"`, `"-0.08"`, `"25345"`. Dấu phẩy, khoảng trắng, `+`, số mũ
/// hay quá 28 chữ số → `decimal.invalid`.
pub fn parse(text: &str) -> Result<Decimal, Error> {
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let (whole, fraction) = match unsigned.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (unsigned, None),
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    let well_formed = digits(whole) && fraction.is_none_or(digits);
    let count = whole.len() + fraction.map_or(0, str::len);
    if !well_formed || count > MAX_DIGITS {
        return Err(invalid(text));
    }
    Decimal::from_str_exact(text).map_err(|_| invalid(text))
}

/// Giá trị cho cột lưu số thập phân: dạng chữ đã chuẩn hoá (`0.080` → `"0.08"`, `-0` → `"0"`), chạy
/// như nhau trên SQLite và Postgres.
pub fn to_column(value: Decimal) -> String {
    value.normalize().to_string()
}

/// Đọc lại cột ghi bằng [`to_column`]. Chuỗi không hợp lệ trong database là dữ liệu hỏng: lỗi nội
/// bộ, không phải lỗi của người gọi.
pub fn from_column(text: &str) -> Result<Decimal, Error> {
    parse(text).map_err(|_| {
        Error::internal(format!(
            "stored decimal `{}` is malformed",
            text.chars().take(40).collect::<String>()
        ))
    })
}

impl Money {
    /// Số tiền dạng thập phân theo đơn vị chính, chính xác: 1999 cent USD → `19.99`, 50000 VND →
    /// `50000`.
    pub fn to_decimal(self) -> Decimal {
        // `Decimal::new` chỉ dừng chương trình khi số chữ số thập phân > 28; tiền tệ có tối đa 4.
        Decimal::new(self.minor(), u32::from(self.currency().minor_units()))
    }

    /// Số tiền từ một số thập phân theo đơn vị chính của `currency`, làm tròn tới đơn vị nhỏ nhất
    /// theo `rounding`: `19.995` USD `HalfUp` → 2000 cent. Ngoài khoảng `i64` → `money.overflow`.
    pub fn from_decimal(
        value: Decimal,
        currency: Currency,
        rounding: Rounding,
    ) -> Result<Money, Error> {
        let places = u32::from(currency.minor_units());
        let mut rounded = round(value, places, rounding);
        // Đưa về đúng `places` chữ số lẻ: mantissa lúc đó chính là số đơn vị nhỏ nhất.
        rounded.rescale(places);
        if rounded.scale() != places {
            return Err(crate::money::overflow());
        }
        let minor = i64::try_from(rounded.mantissa()).map_err(|_| crate::money::overflow())?;
        Ok(Money::new(minor, currency))
    }

    /// `self × rate` cùng loại tiền, làm tròn theo `rounding` (vd thuế 8%: `rate = 0.08`).
    pub fn apply_rate(self, rate: Decimal, rounding: Rounding) -> Result<Money, Error> {
        self.convert(rate, self.currency(), rounding)
    }

    /// Đổi sang loại tiền `to` theo tỉ giá `rate` (một đơn vị chính của tiền này = `rate` đơn vị
    /// chính của `to`), làm tròn theo `rounding`: 10 USD × 25345.5 → 253455 VND.
    pub fn convert(self, rate: Decimal, to: Currency, rounding: Rounding) -> Result<Money, Error> {
        let product = self.to_decimal().checked_mul(rate).ok_or_else(crate::money::overflow)?;
        Money::from_decimal(product, to, rounding)
    }
}

/// Số thập phân ở API: JSON là **chuỗi** (`"0.08"`), không phải số — số JSON bị JavaScript đọc
/// thành `f64` và mất chính xác. Đầu vào sai (kể cả số JSON) bị từ chối với `decimal.invalid`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DecimalString(pub Decimal);

impl fmt::Debug for DecimalString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl fmt::Display for DecimalString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&to_column(self.0))
    }
}

impl FromStr for DecimalString {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse(text).map(DecimalString)
    }
}

impl Serialize for DecimalString {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for DecimalString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        parse(&text).map(DecimalString).map_err(|error| serde::de::Error::custom(error.message()))
    }
}

impl PartialSchema for DecimalString {
    fn schema() -> RefOr<Schema> {
        ObjectBuilder::new()
            .schema_type(Type::String)
            .pattern(Some(PATTERN))
            .description(Some(
                "A decimal number written as a string, like \"12.5\" or \"-0.08\": a dot before \
                 the fraction, no thousands separators, no exponent, at most 28 digits.",
            ))
            .examples(["0.08"])
            .into()
    }
}

impl ToSchema for DecimalString {}

fn invalid(text: &str) -> Error {
    // Chỉ nêu độ dài, không chép lại đầu vào: có thể rất dài hay chứa ký tự lạ.
    Error::new(
        ErrorKind::Validation,
        "decimal.invalid",
        "A decimal number must be a string like \"12.5\": digits, an optional dot and fraction, \
         no comma, space, exponent or JSON number, at most 28 digits; got {length} characters \
         that are not one.",
    )
    .with_param("length", text.chars().count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::money::CURRENCIES;

    fn dec(text: &str) -> Decimal {
        parse(text).unwrap()
    }

    #[test]
    fn decimal_money_roundtrip_every_currency() {
        for &(code, _) in CURRENCIES {
            let currency = Currency::parse(code).unwrap();
            for minor in [0, 1, -1, 1999, -1999, 123_456_789, i64::MAX, i64::MIN] {
                let money = Money::new(minor, currency);
                let back = Money::from_decimal(money.to_decimal(), currency, Rounding::Down);
                assert_eq!(back.unwrap(), money, "{code} {minor}");
            }
        }
        assert_eq!(Money::new(1999, Currency::USD).to_decimal(), dec("19.99"));
        assert_eq!(Money::new(50_000, Currency::VND).to_decimal(), dec("50000"));
    }

    #[test]
    fn decimal_rounding_strategies() {
        let cases = [
            // giá trị, HalfUp, HalfEven, Down, Up, Floor, Ceiling (tới 0 chữ số lẻ)
            ("2.5", [3, 2, 2, 3, 2, 3]),
            ("3.5", [4, 4, 3, 4, 3, 4]),
            ("-2.5", [-3, -2, -2, -3, -3, -2]),
            ("2.1", [2, 2, 2, 3, 2, 3]),
            ("-2.9", [-3, -3, -2, -3, -3, -2]),
            ("7", [7, 7, 7, 7, 7, 7]),
        ];
        let rules = [
            Rounding::HalfUp,
            Rounding::HalfEven,
            Rounding::Down,
            Rounding::Up,
            Rounding::Floor,
            Rounding::Ceiling,
        ];
        for (value, expected) in cases {
            for (rule, want) in rules.iter().zip(expected) {
                let got = Money::from_decimal(dec(value), Currency::VND, *rule).unwrap();
                assert_eq!(got.minor(), want, "{value} {rule:?}");
            }
        }
        // USD: làm tròn tới cent.
        let cents = Money::from_decimal(dec("19.995"), Currency::USD, Rounding::HalfUp).unwrap();
        assert_eq!(cents.minor(), 2000);
        assert_eq!(round(dec("1.005"), 2, Rounding::HalfEven), dec("1.00"));
    }

    #[test]
    fn decimal_from_overflow_is_money_overflow() {
        let huge = dec("9223372036854775808");
        let error = Money::from_decimal(huge, Currency::VND, Rounding::Down).unwrap_err();
        assert_eq!(error.code(), "money.overflow");
        // USD thêm hai chữ số lẻ: 10^17 USD = 10^19 cent, quá i64.
        let error = Money::from_decimal(dec("100000000000000000"), Currency::USD, Rounding::Down)
            .unwrap_err();
        assert_eq!(error.code(), "money.overflow");
        let error = Money::new(i64::MAX, Currency::VND)
            .convert(dec("2"), Currency::VND, Rounding::Down)
            .unwrap_err();
        assert_eq!(error.code(), "money.overflow");
        let max = Money::from_decimal(dec("9223372036854775807"), Currency::VND, Rounding::Down);
        assert_eq!(max.unwrap().minor(), i64::MAX);
    }

    #[test]
    fn ap_ti_le_va_doi_tien() {
        // Thuế 8% của 123 456 đ = 9 876,48 → 9 876 (HalfUp).
        let tax = Money::new(123_456, Currency::VND).apply_rate(dec("0.08"), Rounding::HalfUp);
        assert_eq!(tax.unwrap(), Money::new(9876, Currency::VND));
        // 10 USD × 25 345,5 = 253 455 VND.
        let vnd = Money::new(1000, Currency::USD).convert(
            dec("25345.5"),
            Currency::VND,
            Rounding::HalfUp,
        );
        assert_eq!(vnd.unwrap(), Money::new(253_455, Currency::VND));
        // 253 455 VND × 0,0000394 USD = 9,986127 → 9,99 USD (HalfUp) / 9,98 (Down).
        let back = Money::new(253_455, Currency::VND);
        let up = back.convert(dec("0.0000394"), Currency::USD, Rounding::HalfUp).unwrap();
        let down = back.convert(dec("0.0000394"), Currency::USD, Rounding::Down).unwrap();
        assert_eq!((up.minor(), down.minor()), (999, 998));
    }

    #[test]
    fn doc_chuoi_va_cot() {
        for good in ["0", "12.5", "-0.08", "25345", "0.000001"] {
            assert!(parse(good).is_ok(), "{good}");
        }
        for bad in ["", "-", "1,5", " 1", "1 ", "+1", "1e3", ".5", "5.", "1.2.3", "NaN", "１"] {
            assert_eq!(parse(bad).unwrap_err().code(), "decimal.invalid", "{bad:?}");
        }
        assert_eq!(parse(&"9".repeat(29)).unwrap_err().code(), "decimal.invalid");
        assert_eq!(to_column(dec("0.080")), "0.08");
        assert_eq!(to_column(dec("-0")), "0");
        assert_eq!(to_column(dec("25345.50")), "25345.5");
        assert_eq!(from_column("0.08").unwrap(), dec("0.08"));
        assert!(from_column("abc").unwrap_err().code() != "decimal.invalid");
    }

    #[test]
    fn json_la_chuoi() {
        let value: DecimalString = serde_json::from_str("\"0.080\"").unwrap();
        assert_eq!(serde_json::to_string(&value).unwrap(), "\"0.08\"");
        assert!(serde_json::from_str::<DecimalString>("0.08").is_err(), "số JSON bị từ chối");
        let error = serde_json::from_str::<DecimalString>("\"1,5\"").unwrap_err().to_string();
        assert!(error.contains("decimal number"), "{error}");
    }
}
