//! Decimal numbers for computing money, tax and exchange rates (step 36 item 1, D-188, D-322).
//!
//! Money is still STORED as [`Money`] — an integer in the smallest unit (D-180). [`Decimal`] is used only
//! when COMPUTING: a tax rate `0.08`, an exchange rate `25345.5`, a quantity `1.5` kg. Three rules:
//!
//! - **Never goes through `f64`**: decimals are read from strings and written to strings.
//! - **Rounding is deliberate**: every place that converts a `Decimal` back to `Money` names a [`Rounding`];
//!   there is no implicit default rule.
//! - **Stored and sent as text**: the database column is a normalized string ([`to_column`] / [`from_column`]),
//!   not the database's decimal type (SQLite has none); JSON is a string ([`DecimalString`]),
//!   because JavaScript reads JSON numbers as `f64`.

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

/// Text form of a decimal at the API: optional `-`, digits, an optional fractional part after the dot.
pub const PATTERN: &str = "^-?[0-9]+(\\.[0-9]+)?$";

/// Maximum number of digits `rust_decimal` can hold (96-bit mantissa).
const MAX_DIGITS: usize = 28;

/// Rounding rule when converting a decimal to a number of smallest units. Each caller picks one (the rounding
/// table of the step 36 spec): VAT `HalfUp`, ordinary discounts `Down`…
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Rounding {
    /// Halves round away from zero: 2.5 → 3, −2.5 → −3.
    HalfUp,
    /// Halves round to even (banker's rounding): 2.5 → 2, 3.5 → 4.
    HalfEven,
    /// Drops the fraction, toward zero: 2.9 → 2, −2.9 → −2.
    Down,
    /// Any fraction goes away from zero: 2.1 → 3, −2.1 → −3.
    Up,
    /// Toward negative infinity: 2.9 → 2, −2.1 → −3.
    Floor,
    /// Toward positive infinity: 2.1 → 3, −2.9 → −2.
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

/// `value` rounded to `places` digits after the dot according to `rounding`.
pub fn round(value: Decimal, places: u32, rounding: Rounding) -> Decimal {
    value.round_dp_with_strategy(places, rounding.strategy())
}

/// Reads caller input: `"12.5"`, `"-0.08"`, `"25345"`. A comma, whitespace, `+`, an exponent
/// or more than 28 digits → `decimal.invalid`.
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

/// Value for a decimal-storing column: the normalized text form (`0.080` → `"0.08"`, `-0` → `"0"`), behaving
/// identically on SQLite and Postgres.
pub fn to_column(value: Decimal) -> String {
    value.normalize().to_string()
}

/// Reads back a column written with [`to_column`]. An invalid string in the database is corrupt data: an
/// internal error, not the caller's error.
pub fn from_column(text: &str) -> Result<Decimal, Error> {
    parse(text).map_err(|_| {
        Error::internal(format!(
            "stored decimal `{}` is malformed",
            text.chars().take(40).collect::<String>()
        ))
    })
}

impl Money {
    /// The amount as an exact decimal in the main unit: 1999 USD cents → `19.99`, 50000 VND →
    /// `50000`.
    pub fn to_decimal(self) -> Decimal {
        // `Decimal::new` chỉ dừng chương trình khi số chữ số thập phân > 28; tiền tệ có tối đa 4.
        Decimal::new(self.minor(), u32::from(self.currency().minor_units()))
    }

    /// The amount from a decimal in the main unit of `currency`, rounded to the smallest unit
    /// according to `rounding`: `19.995` USD `HalfUp` → 2000 cents. Outside the `i64` range → `money.overflow`.
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

    /// `self × rate` in the same currency, rounded according to `rounding` (e.g. 8% tax: `rate = 0.08`).
    pub fn apply_rate(self, rate: Decimal, rounding: Rounding) -> Result<Money, Error> {
        self.convert(rate, self.currency(), rounding)
    }

    /// Converts to currency `to` at exchange rate `rate` (one main unit of this currency = `rate` main
    /// units of `to`), rounded according to `rounding`: 10 USD × 25345.5 → 253455 VND.
    pub fn convert(self, rate: Decimal, to: Currency, rounding: Rounding) -> Result<Money, Error> {
        let product = self.to_decimal().checked_mul(rate).ok_or_else(crate::money::overflow)?;
        Money::from_decimal(product, to, rounding)
    }
}

/// A decimal at the API: JSON is a **string** (`"0.08"`), not a number — JavaScript reads JSON numbers
/// as `f64` and loses precision. Wrong input (including a JSON number) is rejected with `decimal.invalid`.
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

// Thử phá số thập phân và quy đổi tiền (D-20261123-property-fuzzing).
#[cfg(test)]
mod properties {
    use super::*;
    use proptest::prelude::*;

    fn rounding() -> impl Strategy<Value = Rounding> {
        prop_oneof![
            Just(Rounding::HalfUp),
            Just(Rounding::HalfEven),
            Just(Rounding::Down),
            Just(Rounding::Up),
            Just(Rounding::Floor),
            Just(Rounding::Ceiling),
        ]
    }

    proptest! {
        #[test]
        fn doc_chuoi_bat_ky_khong_panic_va_cot_doc_lai_nguyen_ven(
            raw in "-?[0-9]{0,30}(\\.[0-9]{0,30})?|\\PC{0,40}",
        ) {
            if let Ok(value) = parse(&raw) {
                prop_assert_eq!(from_column(&to_column(value)).unwrap(), value);
            }
        }

        #[test]
        fn tien_doi_ra_thap_phan_roi_doi_lai_nguyen_ven(
            minor in any::<i64>(),
            currency in prop_oneof![Just(Currency::VND), Just(Currency::USD), Just(Currency::parse("KWD").unwrap())],
            rounding in rounding(),
        ) {
            let money = Money::new(minor, currency);
            prop_assert_eq!(Money::from_decimal(money.to_decimal(), currency, rounding).unwrap(), money);
        }

        #[test]
        fn nhan_ti_le_tu_0_toi_1_khong_vuot_so_goc_va_cung_dau(
            minor in any::<i64>(),
            basis_points in 0i64..=10_000,
            rounding in rounding(),
        ) {
            let money = Money::new(minor, Currency::VND);
            let rate = Decimal::new(basis_points, 4);
            let applied = money.apply_rate(rate, rounding).unwrap();
            prop_assert!(applied.minor().unsigned_abs() <= minor.unsigned_abs());
            prop_assert!(applied.minor() == 0 || (applied.minor() < 0) == (minor < 0));
            // Sai khác so với giá trị đúng dưới một đơn vị nhỏ nhất.
            let exact = Decimal::from(minor) * rate;
            prop_assert!((Decimal::from(applied.minor()) - exact).abs() < Decimal::ONE);
        }

        #[test]
        fn quy_doi_ti_gia_bat_ky_khong_panic(
            minor in any::<i64>(),
            mantissa in any::<i64>(),
            scale in 0u32..=28,
            rounding in rounding(),
        ) {
            let rate = Decimal::new(mantissa, scale);
            if let Err(error) = Money::new(minor, Currency::USD).convert(rate, Currency::VND, rounding) {
                prop_assert_eq!(error.code(), "money.overflow");
            }
        }
    }
}
