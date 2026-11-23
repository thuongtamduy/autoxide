//! Money: an integer in the smallest unit plus an ISO 4217 currency code, one type for every module (D-180).
//!
//! The data rule of `CLAUDE.md`: money is never `f32`/`f64` or a decimal database type. [`Money`] holds the
//! number of **smallest units** (`i64`: cents of USD, dong of VND) and a [`Currency`] that knows how many
//! decimal places the code has.
//!
//! - Addition, subtraction and multiplication by an integer are all **overflow-checked** and return [`Error`]
//!   instead of wrapping around.
//! - Arithmetic between two different currencies is refused (`money.currency_mismatch`): converting currency is
//!   business logic, with its own exchange rate and rounding rules.
//! - Proportional division with [`Money::allocate`] / [`Money::split`]: the parts always sum to the original
//!   amount, with no unit lost (100 split by 3 -> 34, 33, 33).
//! - JSON: `{"amount": "1999", "currency": "USD"}`; `amount` is an integer **string**, because JavaScript reads
//!   a JSON number as `f64` and rounds every number larger than 2^53 incorrectly.
//! - Stored in the database: two columns `<name>_minor` (`BIGINT`) and `<name>_currency` (a 3-character
//!   string), behaving the same on SQLite and Postgres. Read back with [`Money::from_columns`].
//!
//! Concepts such as account, balance and ledger are not here; they belong to modules.

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{ObjectBuilder, Schema, Type};
use utoipa::{PartialSchema, ToSchema};

use crate::error::{Error, ErrorKind};

/// A currency under ISO 4217: a three-letter code and its number of decimal places.
///
/// Accepts only codes currently in circulation that have a minor unit (the [`CURRENCIES`] table); precious
/// metal codes, fund codes with no decimal places (`XAU`, `XDR`, ...) and
/// withdrawn codes are refused.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Currency {
    // `&'static str`: trỏ thẳng vào bảng viết sẵn trong binary, nên `Currency` là `Copy` và
    // không cấp phát bộ nhớ.
    code: &'static str,
    minor_units: u8,
}

impl Currency {
    /// US dollar (2 decimal places).
    pub const USD: Currency = Currency { code: "USD", minor_units: 2 };
    /// Euro (2 decimal places).
    pub const EUR: Currency = Currency { code: "EUR", minor_units: 2 };
    /// Vietnamese dong (0 decimal places).
    pub const VND: Currency = Currency { code: "VND", minor_units: 0 };
    /// Japanese yen (0 decimal places).
    pub const JPY: Currency = Currency { code: "JPY", minor_units: 0 };

    /// Looks up an ISO 4217 code (`"USD"`, `"vnd"`; case-insensitive). A code not in the table
    /// returns the error `money.currency_unknown`.
    pub fn parse(code: &str) -> Result<Currency, Error> {
        let upper = code.trim().to_ascii_uppercase();
        CURRENCIES
            .iter()
            .find(|(known, _)| *known == upper)
            .map(|&(code, minor_units)| Currency { code, minor_units })
            .ok_or_else(|| currency_unknown(code))
    }

    /// Three-letter uppercase code, e.g. `"USD"`.
    pub fn code(self) -> &'static str {
        self.code
    }

    /// Number of decimal places: 2 for USD (1 USD = 100 cents), 0 for VND, 3 for KWD.
    pub fn minor_units(self) -> u8 {
        self.minor_units
    }
}

impl fmt::Debug for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code)
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code)
    }
}

impl FromStr for Currency {
    type Err = Error;

    fn from_str(code: &str) -> Result<Self, Self::Err> {
        Currency::parse(code)
    }
}

impl Serialize for Currency {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.code)
    }
}

impl<'de> Deserialize<'de> for Currency {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let code = String::deserialize(deserializer)?;
        Currency::parse(&code).map_err(|error| serde::de::Error::custom(error.message()))
    }
}

// Tài liệu OpenAPI viết tay: derive của utoipa chỉ tả được struct theo từng trường, còn
// `Currency` đi trên dây là một chuỗi.
impl PartialSchema for Currency {
    fn schema() -> RefOr<Schema> {
        ObjectBuilder::new()
            .schema_type(Type::String)
            .description(Some(
                "ISO 4217 currency code in upper case, e.g. `USD`, `EUR`, `VND`. Only codes in \
                 circulation that have minor units are accepted.",
            ))
            .pattern(Some("^[A-Z]{3}$"))
            .examples(["USD"])
            .into()
    }
}

impl ToSchema for Currency {}

/// A sum of money: an integer number of the currency's minor units (cents for `USD`, dong for
/// `VND`) plus the currency.
// `Hash`/`Eq` có, `Ord` thì không: so 1 USD với 1 EUR không có nghĩa — dùng
// [`Money::compare`], hàm này từ chối hai loại tiền khác nhau.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
pub struct Money {
    /// Amount in minor units of `currency`, as a base-10 integer string (JSON numbers above
    /// 2^53 lose precision in JavaScript). `"1999"` with `USD` is 19.99 US dollars; negative
    /// amounts start with `-`.
    #[serde(rename = "amount", with = "amount_string")]
    #[schema(value_type = String, pattern = "^-?[0-9]{1,19}$", example = "1999")]
    minor: i64,
    currency: Currency,
}

impl Money {
    /// `minor` smallest units of `currency`: `Money::new(1999, Currency::USD)` is 19.99 USD.
    pub const fn new(minor: i64, currency: Currency) -> Money {
        Money { minor, currency }
    }

    /// The zero of `currency`.
    pub const fn zero(currency: Currency) -> Money {
        Money { minor: 0, currency }
    }

    /// Reads from the caller's input: `amount` is an integer string in the smallest unit (`"1999"`,
    /// `"-5"`), `currency` is an ISO 4217 code. Otherwise the error is `money.amount_invalid` or
    /// `money.currency_unknown`.
    pub fn parse(amount: &str, currency: &str) -> Result<Money, Error> {
        Ok(Money { minor: parse_minor(amount)?, currency: Currency::parse(currency)? })
    }

    /// Rebuilds from the two standard storage columns `<name>_minor` and `<name>_currency`. A currency code
    /// in the database that is no longer valid is corrupt data: an internal error, not the caller's error.
    pub fn from_columns(minor: i64, currency: &str) -> Result<Money, Error> {
        let currency = Currency::parse(currency).map_err(|_| {
            Error::internal(format!(
                "stored currency code `{currency}` is not a known ISO 4217 code"
            ))
        })?;
        Ok(Money { minor, currency })
    }

    /// Number of smallest units: the value for the column `<name>_minor`.
    pub const fn minor(self) -> i64 {
        self.minor
    }

    /// The currency; `currency().code()` is the value for the column `<name>_currency`.
    pub const fn currency(self) -> Currency {
        self.currency
    }

    pub const fn is_zero(self) -> bool {
        self.minor == 0
    }

    pub const fn is_positive(self) -> bool {
        self.minor > 0
    }

    pub const fn is_negative(self) -> bool {
        self.minor < 0
    }

    /// `self + other`. Different currencies: `money.currency_mismatch`; `i64` overflow: `money.overflow`.
    pub fn checked_add(self, other: Money) -> Result<Money, Error> {
        self.same_currency(other)?;
        self.with_minor(self.minor.checked_add(other.minor))
    }

    /// `self - other`. Different currencies: `money.currency_mismatch`; `i64` overflow: `money.overflow`.
    pub fn checked_sub(self, other: Money) -> Result<Money, Error> {
        self.same_currency(other)?;
        self.with_minor(self.minor.checked_sub(other.minor))
    }

    /// `self × factor` (e.g. unit price × quantity). `i64` overflow: `money.overflow`.
    pub fn checked_mul(self, factor: i64) -> Result<Money, Error> {
        self.with_minor(self.minor.checked_mul(factor))
    }

    /// `-self` (e.g. the amount of a reversing transaction). Overflow (`i64::MIN`): `money.overflow`.
    pub fn checked_neg(self) -> Result<Money, Error> {
        self.with_minor(self.minor.checked_neg())
    }

    /// Sum of `items`, all of which must be `currency` (an empty list gives zero).
    pub fn sum(currency: Currency, items: impl IntoIterator<Item = Money>) -> Result<Money, Error> {
        items.into_iter().try_fold(Money::zero(currency), Money::checked_add)
    }

    /// Compares two amounts of the same currency. Different currencies: `money.currency_mismatch`.
    pub fn compare(self, other: Money) -> Result<Ordering, Error> {
        self.same_currency(other)?;
        Ok(self.minor.cmp(&other.minor))
    }

    /// Splits into parts according to `ratios` (e.g. `[1, 1, 1]` splits in three, `[70, 30]` splits 70/30).
    ///
    /// The parts **always** sum to `self`: each part takes the integer part of its share, then the leftover
    /// units are handed out one by one to the parts with the largest remainders (ties go to the earlier
    /// part); 100 split by `[1, 1, 1]` gives 34, 33, 33. A part with ratio 0 always gets 0. A negative amount
    /// splits symmetrically to a positive one. At least one non-zero ratio is required, otherwise
    /// `money.allocation_invalid`.
    pub fn allocate(self, ratios: &[u64]) -> Result<Vec<Money>, Error> {
        // Tính bằng `i128`: `minor × ratio` của hai số 64-bit không bao giờ tràn 128-bit.
        let total: i128 = ratios.iter().map(|&ratio| i128::from(ratio)).sum();
        if total == 0 {
            return Err(allocation_invalid());
        }
        let amount = i128::from(self.minor);
        let magnitude = amount.abs();
        let mut shares = Vec::with_capacity(ratios.len());
        let mut remainders = Vec::with_capacity(ratios.len());
        for (index, &ratio) in ratios.iter().enumerate() {
            let exact = magnitude * i128::from(ratio);
            shares.push(exact / total);
            remainders.push((exact % total, index));
        }
        let mut left = magnitude - shares.iter().sum::<i128>();
        // Phần lẻ lớn nhất trước; bằng nhau thì phần đứng trước.
        remainders.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        for (_, index) in remainders {
            if left == 0 {
                break;
            }
            shares[index] += 1;
            left -= 1;
        }
        shares
            .into_iter()
            .map(|share| {
                let signed = if amount < 0 { -share } else { share };
                // Mỗi phần có trị tuyệt đối không quá `|self|`, nên luôn vừa `i64`.
                self.with_minor(i64::try_from(signed).ok())
            })
            .collect()
    }

    /// Splits evenly into `parts` parts (100 split by 3 -> 34, 33, 33). `parts` = 0:
    /// `money.allocation_invalid`.
    pub fn split(self, parts: usize) -> Result<Vec<Money>, Error> {
        self.allocate(&vec![1; parts])
    }

    fn same_currency(self, other: Money) -> Result<(), Error> {
        if self.currency == other.currency {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::Validation,
                "money.currency_mismatch",
                "Amounts in {left} and {right} cannot be combined; convert them to one currency \
                 first.",
            )
            .with_param("left", self.currency)
            .with_param("right", other.currency))
        }
    }

    fn with_minor(self, minor: Option<i64>) -> Result<Money, Error> {
        minor.map(|minor| Money { minor, currency: self.currency }).ok_or_else(overflow)
    }
}

/// Readable form for logs and mail: `-12.34 USD`, `50000 VND`.
impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let digits = usize::from(self.currency.minor_units);
        let sign = if self.minor < 0 { "-" } else { "" };
        // `unsigned_abs`: trị tuyệt đối của `i64::MIN` không vừa `i64` nhưng vừa `u64`.
        let magnitude = self.minor.unsigned_abs();
        if digits == 0 {
            return write!(f, "{sign}{magnitude} {}", self.currency);
        }
        let scale = 10u64.pow(u32::from(self.currency.minor_units));
        write!(f, "{sign}{}.{:0digits$} {}", magnitude / scale, magnitude % scale, self.currency)
    }
}

impl fmt::Debug for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Money({} {})", self.minor, self.currency)
    }
}

/// The `amount` of the JSON: an integer string (see the documentation at the top of the file).
mod amount_string {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(minor: &i64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(minor)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
        let text = String::deserialize(deserializer)?;
        super::parse_minor(&text).map_err(|error| serde::de::Error::custom(error.message()))
    }
}

/// Decimal integer string: an optional `-` then 1-19 digits, no whitespace, no `+`.
fn parse_minor(text: &str) -> Result<i64, Error> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    let well_formed =
        !digits.is_empty() && digits.len() <= 19 && digits.bytes().all(|b| b.is_ascii_digit());
    if !well_formed {
        return Err(amount_invalid(text));
    }
    text.parse::<i64>().map_err(|_| overflow())
}

pub(crate) fn overflow() -> Error {
    Error::new(
        ErrorKind::Validation,
        "money.overflow",
        "The amount is outside the range that can be stored: at most 9223372036854775807 minor \
         units either side of zero.",
    )
}

fn amount_invalid(text: &str) -> Error {
    // Chỉ nêu độ dài, không chép lại đầu vào: có thể rất dài hay chứa ký tự lạ.
    Error::new(
        ErrorKind::Validation,
        "money.amount_invalid",
        "An amount must be a whole number of minor units written as a string, like \"1999\" \
         for 19.99 USD or \"-500\"; got {length} characters that are not one.",
    )
    .with_param("length", text.chars().count())
}

fn currency_unknown(code: &str) -> Error {
    let shown: String = code.chars().take(8).collect();
    Error::new(
        ErrorKind::Validation,
        "money.currency_unknown",
        "`{code}` is not an ISO 4217 currency code in circulation; use a three-letter code such \
         as `USD`, `EUR` or `VND`.",
    )
    .with_param("code", shown)
}

fn allocation_invalid() -> Error {
    Error::new(
        ErrorKind::Validation,
        "money.allocation_invalid",
        "An amount can only be divided by at least one ratio, and not every ratio may be zero.",
    )
}

/// ISO 4217 codes currently in circulation and their number of decimal places (table A.1 of ISO 4217,
/// including fund codes with a minor unit such as `CLF`, `UYW`). Not included: precious metals and codes with
/// no minor unit (`XAU`, `XAG`, `XDR`, `XSU`, `XUA`, ...), test codes (`XTS`, `XXX`) and withdrawn codes
/// (`HRK`, `ZWL`, `SLL`, `ANG`, ...). Add a code when ISO publishes an amendment.
// Viết bảy mã một dòng cho dễ dò; rustfmt sẽ tách mỗi mã một dòng (gần 200 dòng).
#[rustfmt::skip]
pub const CURRENCIES: &[(&str, u8)] = &[
    ("AED", 2), ("AFN", 2), ("ALL", 2), ("AMD", 2), ("AOA", 2), ("ARS", 2), ("AUD", 2),
    ("AWG", 2), ("AZN", 2), ("BAM", 2), ("BBD", 2), ("BDT", 2), ("BGN", 2), ("BHD", 3),
    ("BIF", 0), ("BMD", 2), ("BND", 2), ("BOB", 2), ("BOV", 2), ("BRL", 2), ("BSD", 2),
    ("BTN", 2), ("BWP", 2), ("BYN", 2), ("BZD", 2), ("CAD", 2), ("CDF", 2), ("CHE", 2),
    ("CHF", 2), ("CHW", 2), ("CLF", 4), ("CLP", 0), ("CNY", 2), ("COP", 2), ("COU", 2),
    ("CRC", 2), ("CUP", 2), ("CVE", 2), ("CZK", 2), ("DJF", 0), ("DKK", 2), ("DOP", 2),
    ("DZD", 2), ("EGP", 2), ("ERN", 2), ("ETB", 2), ("EUR", 2), ("FJD", 2), ("FKP", 2),
    ("GBP", 2), ("GEL", 2), ("GHS", 2), ("GIP", 2), ("GMD", 2), ("GNF", 0), ("GTQ", 2),
    ("GYD", 2), ("HKD", 2), ("HNL", 2), ("HTG", 2), ("HUF", 2), ("IDR", 2), ("ILS", 2),
    ("INR", 2), ("IQD", 3), ("IRR", 2), ("ISK", 0), ("JMD", 2), ("JOD", 3), ("JPY", 0),
    ("KES", 2), ("KGS", 2), ("KHR", 2), ("KMF", 0), ("KPW", 2), ("KRW", 0), ("KWD", 3),
    ("KYD", 2), ("KZT", 2), ("LAK", 2), ("LBP", 2), ("LKR", 2), ("LRD", 2), ("LSL", 2),
    ("LYD", 3), ("MAD", 2), ("MDL", 2), ("MGA", 2), ("MKD", 2), ("MMK", 2), ("MNT", 2),
    ("MOP", 2), ("MRU", 2), ("MUR", 2), ("MVR", 2), ("MWK", 2), ("MXN", 2), ("MXV", 2),
    ("MYR", 2), ("MZN", 2), ("NAD", 2), ("NGN", 2), ("NIO", 2), ("NOK", 2), ("NPR", 2),
    ("NZD", 2), ("OMR", 3), ("PAB", 2), ("PEN", 2), ("PGK", 2), ("PHP", 2), ("PKR", 2),
    ("PLN", 2), ("PYG", 0), ("QAR", 2), ("RON", 2), ("RSD", 2), ("RUB", 2), ("RWF", 0),
    ("SAR", 2), ("SBD", 2), ("SCR", 2), ("SDG", 2), ("SEK", 2), ("SGD", 2), ("SHP", 2),
    ("SLE", 2), ("SOS", 2), ("SRD", 2), ("SSP", 2), ("STN", 2), ("SVC", 2), ("SYP", 2),
    ("SZL", 2), ("THB", 2), ("TJS", 2), ("TMT", 2), ("TND", 3), ("TOP", 2), ("TRY", 2),
    ("TTD", 2), ("TWD", 2), ("TZS", 2), ("UAH", 2), ("UGX", 0), ("USD", 2), ("USN", 2),
    ("UYI", 0), ("UYU", 2), ("UYW", 4), ("UZS", 2), ("VED", 2), ("VES", 2), ("VND", 0),
    ("VUV", 0), ("WST", 2), ("XAF", 0), ("XCD", 2), ("XCG", 2), ("XOF", 0), ("XPF", 0),
    ("YER", 2), ("ZAR", 2), ("ZMW", 2), ("ZWG", 2),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn usd(minor: i64) -> Money {
        Money::new(minor, Currency::USD)
    }

    fn minors(parts: &[Money]) -> Vec<i64> {
        parts.iter().map(|part| part.minor()).collect()
    }

    #[test]
    fn bang_ma_dung_va_khong_trung() {
        let mut codes: Vec<&str> = CURRENCIES.iter().map(|(code, _)| *code).collect();
        assert!(codes.windows(2).all(|pair| pair[0] < pair[1]), "bảng phải sắp theo mã");
        codes.dedup();
        assert_eq!(codes.len(), CURRENCIES.len());
        for (code, units) in CURRENCIES {
            assert!(code.len() == 3 && code.bytes().all(|b| b.is_ascii_uppercase()), "{code}");
            assert!(*units <= 4, "{code}");
        }
        for constant in [Currency::USD, Currency::EUR, Currency::VND, Currency::JPY] {
            assert_eq!(Currency::parse(constant.code()).unwrap(), constant);
        }
    }

    #[test]
    fn tra_ma_tien_te() {
        assert_eq!(Currency::parse("usd").unwrap(), Currency::USD);
        assert_eq!(Currency::parse("KWD").unwrap().minor_units(), 3);
        assert_eq!(Currency::parse("VND").unwrap().minor_units(), 0);
        for bad in ["", "US", "USDX", "XAU", "XXX", "HRK", "123"] {
            assert_eq!(Currency::parse(bad).unwrap_err().code(), "money.currency_unknown", "{bad}");
        }
    }

    #[test]
    fn cong_tru_nhan_kiem_tran() {
        assert_eq!(usd(150).checked_add(usd(-50)).unwrap(), usd(100));
        assert_eq!(usd(150).checked_sub(usd(200)).unwrap(), usd(-50));
        assert_eq!(usd(250).checked_mul(3).unwrap(), usd(750));
        assert_eq!(usd(7).checked_neg().unwrap(), usd(-7));
        let max = usd(i64::MAX);
        assert_eq!(max.checked_add(usd(1)).unwrap_err().code(), "money.overflow");
        assert_eq!(usd(i64::MIN).checked_sub(usd(1)).unwrap_err().code(), "money.overflow");
        assert_eq!(max.checked_mul(2).unwrap_err().code(), "money.overflow");
        assert_eq!(usd(i64::MIN).checked_neg().unwrap_err().code(), "money.overflow");
        let total = Money::sum(Currency::USD, [usd(1), usd(2), usd(3)]).unwrap();
        assert_eq!(total, usd(6));
        assert_eq!(Money::sum(Currency::USD, []).unwrap(), usd(0));
    }

    #[test]
    fn cam_tinh_giua_hai_loai_tien() {
        let eur = Money::new(100, Currency::EUR);
        let error = usd(100).checked_add(eur).unwrap_err();
        assert_eq!(error.code(), "money.currency_mismatch");
        assert_eq!(error.kind(), ErrorKind::Validation);
        assert_eq!(
            error.message(),
            "Amounts in USD and EUR cannot be combined; convert them to one currency first."
        );
        assert!(usd(1).checked_sub(eur).is_err());
        assert!(usd(1).compare(eur).is_err());
        assert!(Money::sum(Currency::USD, [usd(1), eur]).is_err());
        assert_eq!(usd(1).compare(usd(2)).unwrap(), Ordering::Less);
    }

    #[test]
    fn phan_bo_khong_mat_don_vi() {
        assert_eq!(minors(&usd(100).split(3).unwrap()), [34, 33, 33]);
        assert_eq!(minors(&usd(-100).split(3).unwrap()), [-34, -33, -33]);
        assert_eq!(minors(&usd(5).allocate(&[70, 30]).unwrap()), [4, 1]);
        // Phần lẻ lớn nhất nhận đơn vị dư, không phải phần đứng đầu.
        assert_eq!(minors(&usd(1).allocate(&[1, 99]).unwrap()), [0, 1]);
        assert_eq!(minors(&usd(10).allocate(&[0, 1, 0, 1]).unwrap()), [0, 5, 0, 5]);
        assert_eq!(minors(&usd(2).split(5).unwrap()), [1, 1, 0, 0, 0]);
        for amount in [i64::MAX, i64::MIN, 1, -1, 0, 999_999_999_999] {
            for ratios in [&[1u64, 1, 1][..], &[u64::MAX, 1], &[3, 0, 7, 11]] {
                let parts = usd(amount).allocate(ratios).unwrap();
                let sum: i128 = parts.iter().map(|part| i128::from(part.minor())).sum();
                assert_eq!(sum, i128::from(amount), "{amount} {ratios:?}");
            }
        }
        assert_eq!(usd(1).split(0).unwrap_err().code(), "money.allocation_invalid");
        assert_eq!(usd(1).allocate(&[0, 0]).unwrap_err().code(), "money.allocation_invalid");
    }

    #[test]
    fn json_amount_la_chuoi() {
        let value = serde_json::to_value(usd(i64::MAX)).unwrap();
        assert_eq!(value, serde_json::json!({"amount": "9223372036854775807", "currency": "USD"}));
        let back: Money = serde_json::from_value(value).unwrap();
        assert_eq!(back, usd(i64::MAX));
        let parsed: Money = serde_json::from_str(r#"{"amount":"-500","currency":"vnd"}"#).unwrap();
        assert_eq!(parsed, Money::new(-500, Currency::VND));
        for bad in [
            r#"{"amount":1999,"currency":"USD"}"#,
            r#"{"amount":"19.99","currency":"USD"}"#,
            r#"{"amount":"+5","currency":"USD"}"#,
            r#"{"amount":" 5","currency":"USD"}"#,
            r#"{"amount":"","currency":"USD"}"#,
            r#"{"amount":"9223372036854775808","currency":"USD"}"#,
            r#"{"amount":"5","currency":"ABC"}"#,
            r#"{"amount":"5"}"#,
        ] {
            assert!(serde_json::from_str::<Money>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn doc_tu_dau_vao_va_cot() {
        assert_eq!(Money::parse("1999", "USD").unwrap(), usd(1999));
        assert_eq!(Money::parse("1.5", "USD").unwrap_err().code(), "money.amount_invalid");
        assert_eq!(
            Money::parse("99999999999999999999", "USD").unwrap_err().code(),
            "money.amount_invalid"
        );
        assert_eq!(
            Money::parse("9999999999999999999", "USD").unwrap_err().code(),
            "money.overflow"
        );
        assert_eq!(Money::from_columns(5, "EUR").unwrap(), Money::new(5, Currency::EUR));
        assert_eq!(Money::from_columns(5, "??").unwrap_err().code(), "internal_error");
    }

    #[test]
    fn hien_thi_theo_so_chu_so_thap_phan() {
        assert_eq!(usd(-1234).to_string(), "-12.34 USD");
        assert_eq!(usd(5).to_string(), "0.05 USD");
        assert_eq!(Money::new(50_000, Currency::VND).to_string(), "50000 VND");
        assert_eq!(Money::parse("1", "KWD").unwrap().to_string(), "0.001 KWD");
        assert_eq!(usd(i64::MIN).to_string(), "-92233720368547758.08 USD");
    }

    #[test]
    fn tai_lieu_openapi() {
        let schema = serde_json::to_value(Money::schema()).unwrap();
        assert_eq!(schema["properties"]["amount"]["type"], "string");
        assert_eq!(schema["required"], serde_json::json!(["amount", "currency"]));
        let currency = serde_json::to_value(Currency::schema()).unwrap();
        assert_eq!(currency["type"], "string");
        assert_eq!(currency["pattern"], "^[A-Z]{3}$");
    }
}

// Thử phá phép tính tiền (D-20261123-property-fuzzing): chia không mất đồng nào, không tràn thành panic.
#[cfg(test)]
mod properties {
    use super::*;
    use proptest::prelude::*;

    fn currency() -> impl Strategy<Value = Currency> {
        prop_oneof![Just(Currency::VND), Just(Currency::USD), Just(Currency::parse("KWD").unwrap())]
    }

    proptest! {
        #[test]
        fn chia_theo_ti_le_tong_dung_bang_va_cung_dau(
            minor in any::<i64>(),
            currency in currency(),
            ratios in proptest::collection::vec(
                prop_oneof![Just(0u64), 0u64..1_000, any::<u64>()],
                1..12,
            ),
        ) {
            let amount = Money::new(minor, currency);
            let total: u128 = ratios.iter().map(|ratio| u128::from(*ratio)).sum();
            let Ok(parts) = amount.allocate(&ratios) else {
                prop_assert_eq!(total, 0);
                return Ok(());
            };
            prop_assert_eq!(parts.len(), ratios.len());
            prop_assert_eq!(Money::sum(currency, parts.iter().copied()).unwrap(), amount);
            for (part, ratio) in parts.iter().zip(&ratios) {
                prop_assert_eq!(part.currency(), currency);
                // Cùng dấu với số tiền, và phần tỉ lệ 0 luôn bằng 0.
                prop_assert!(part.minor() == 0 || (part.minor() < 0) == (minor < 0));
                if *ratio == 0 {
                    prop_assert!(part.is_zero());
                }
                // Mỗi phần cách phần đúng tỉ lệ dưới một đơn vị nhỏ nhất.
                let exact = i128::from(minor).unsigned_abs() * u128::from(*ratio);
                let got = u128::from(part.minor().unsigned_abs()) * total;
                prop_assert!(got.abs_diff(exact) < total, "{part} / {ratio}");
            }
        }

        #[test]
        fn chia_deu_lech_nhau_toi_da_mot_don_vi(
            minor in any::<i64>(),
            parts in 1usize..50,
        ) {
            let split = Money::new(minor, Currency::VND).split(parts).unwrap();
            let values: Vec<i64> = split.iter().map(|part| part.minor()).collect();
            let (low, high) = (values.iter().min().unwrap(), values.iter().max().unwrap());
            prop_assert!(high.abs_diff(*low) <= 1);
            prop_assert_eq!(values.iter().map(|value| i128::from(*value)).sum::<i128>(), i128::from(minor));
        }

        #[test]
        fn cong_tru_nhan_tran_thi_bao_loi_khong_panic(
            a in any::<i64>(),
            b in any::<i64>(),
        ) {
            let (left, right) = (Money::new(a, Currency::VND), Money::new(b, Currency::VND));
            match left.checked_add(right) {
                Ok(sum) => prop_assert_eq!(sum.checked_sub(right).unwrap(), left),
                Err(error) => prop_assert_eq!(error.code(), "money.overflow"),
            }
            match left.checked_mul(b) {
                Ok(product) => prop_assert_eq!(i128::from(product.minor()), i128::from(a) * i128::from(b)),
                Err(error) => prop_assert_eq!(error.code(), "money.overflow"),
            }
            prop_assert_eq!(left.checked_neg().is_err(), a == i64::MIN);
            prop_assert!(left.checked_add(Money::new(b, Currency::USD)).is_err());
        }

        #[test]
        fn doc_so_tien_khong_panic_va_doc_lai_nguyen_ven(
            raw in "-?[0-9]{0,22}|\\PC{0,30}",
            minor in any::<i64>(),
            currency in currency(),
        ) {
            let _ = Money::parse(&raw, "VND");
            let money = Money::new(minor, currency);
            prop_assert_eq!(Money::parse(&minor.to_string(), currency.code()).unwrap(), money);
            let json = serde_json::to_string(&money).unwrap();
            prop_assert_eq!(serde_json::from_str::<Money>(&json).unwrap(), money);
            let _ = money.to_string();
        }
    }
}
