//! Đọc và kiểm giá trị cấu hình, gom MỌI lỗi rồi mới báo.
//!
//! Người cài đặt sửa cấu hình một lần là xong, thay vì sửa một lỗi, chạy lại,
//! gặp lỗi kế, sửa tiếp.

use std::fmt;
use std::path::Path;
use std::str::FromStr;

use super::{ConfigSource, Setting};

/// Dấu hiệu "giá trị này không hợp lệ, và lỗi ĐÃ được ghi vào [`ConfigReader`]".
///
/// Hàm đọc một nhóm cấu hình trả `Result<Nhóm, Invalid>`. Cách viết: đọc hết các
/// biến vào biến cục bộ trước, rồi mới dùng `?` khi dựng struct — để một biến
/// sai không chặn việc kiểm các biến còn lại:
///
/// ```
/// # use autoxide_kernel::config::{ConfigReader, Invalid, Setting};
/// # const HOST: Setting = Setting::new("HOST", "").with_default("localhost");
/// # const PORT: Setting = Setting::new("PORT", "").with_default("8080");
/// # struct Http { host: String, port: u16 }
/// fn read(reader: &mut ConfigReader) -> Result<Http, Invalid> {
///     let host = reader.get(&HOST);
///     let port = reader.get(&PORT);
///     Ok(Http { host: host?, port: port? })
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Invalid;

/// Một vấn đề trong cấu hình.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConfigError {
    /// Tên biến, hoặc đường dẫn file `.env`.
    pub subject: String,
    /// Sai gì và phải sửa thế nào.
    pub problem: String,
}

impl ConfigError {
    /// A problem with `subject` (a variable name or a file path): what is wrong and how to fix it.
    pub fn new(subject: impl Into<String>, problem: impl Into<String>) -> Self {
        ConfigError { subject: subject.into(), problem: problem.into() }
    }

    pub(super) fn file(path: &Path, problem: String) -> Self {
        ConfigError { subject: path.display().to_string(), problem }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.subject, self.problem)
    }
}

/// Mọi vấn đề tìm thấy trong một lần đọc cấu hình (ít nhất một).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigErrors(Vec<ConfigError>);

impl ConfigErrors {
    pub(super) fn single(error: ConfigError) -> Self {
        ConfigErrors(vec![error])
    }

    pub fn errors(&self) -> &[ConfigError] {
        &self.0
    }
}

// Một lỗi đơn lẻ thành "danh sách lỗi": cho code ngoài kernel báo lỗi lúc khởi
// động (vd đăng ký module sai) theo cùng một đường với lỗi cấu hình.
impl From<ConfigError> for ConfigErrors {
    fn from(error: ConfigError) -> Self {
        ConfigErrors::single(error)
    }
}

impl fmt::Display for ConfigErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let count = self.0.len();
        let noun = if count == 1 { "problem" } else { "problems" };
        write!(f, "invalid configuration ({count} {noun}):")?;
        for error in &self.0 {
            write!(f, "\n  - {error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ConfigErrors {}

/// Giá trị đang có hiệu lực của một biến lấy từ đâu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Người cài đặt đã đặt (biến môi trường hoặc `.env`).
    Configured,
    /// Không đặt; dùng giá trị mặc định khai trong code.
    Default,
    /// Không đặt và không có mặc định.
    Unset,
}

/// Một biến đã được đọc: mô tả của nó và giá trị đang có hiệu lực.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingValue {
    pub setting: Setting,
    pub origin: Origin,
    /// Biến tuỳ chọn: được phép không đặt dù không có mặc định.
    pub optional: bool,
    /// Giá trị thô đang có hiệu lực. Truy cập qua [`display_value`](Self::display_value)
    /// để không lộ secret.
    raw: Option<String>,
}

impl SettingValue {
    /// Giá trị để in cho người đọc: secret do người cài đặt đặt thì bị che.
    pub fn display_value(&self) -> &str {
        match (&self.raw, self.origin, self.setting.secret) {
            (None, _, _) => "(not set)",
            (Some(_), Origin::Configured, true) => "(set, hidden)",
            (Some(raw), _, _) => raw,
        }
    }
}

/// Đọc từng biến từ một [`ConfigSource`], ghi lại lỗi và danh sách biến đã đọc.
#[derive(Debug)]
pub struct ConfigReader<'a> {
    source: &'a ConfigSource,
    errors: Vec<ConfigError>,
    values: Vec<SettingValue>,
}

// `<'a>`: reader MƯỢN source, không sở hữu. Trình biên dịch dùng `'a` để bảo đảm
// source sống lâu hơn reader.
impl<'a> ConfigReader<'a> {
    pub fn new(source: &'a ConfigSource) -> Self {
        ConfigReader { source, errors: Vec::new(), values: Vec::new() }
    }

    /// Đọc một biến có mặc định hoặc bắt buộc, đổi sang kiểu `T` bằng `FromStr`
    /// (số, `bool`, `SocketAddr`, hoặc kiểu tự định nghĩa).
    pub fn get<T>(&mut self, setting: &Setting) -> Result<T, Invalid>
    where
        T: FromStr,
        T::Err: fmt::Display,
    {
        self.get_with(setting, |raw| raw.parse::<T>().map_err(|error| error.to_string()))
    }

    /// Như [`get`](Self::get) nhưng tự viết hàm đổi kiểu và kiểm giá trị. `parse`
    /// trả `Err(câu nói rõ giá trị phải như thế nào)` khi giá trị sai.
    pub fn get_with<T>(
        &mut self,
        setting: &Setting,
        parse: impl FnOnce(&str) -> Result<T, String>,
    ) -> Result<T, Invalid> {
        match self.read(setting, false) {
            Some(raw) => self.parse(setting, &raw, parse),
            None => {
                let problem = format!("is required but not set. {}", setting.description);
                self.fail(setting, problem.trim_end().to_owned())
            }
        }
    }

    /// Đọc một biến tuỳ chọn: không đặt (và không có mặc định) thì trả `Ok(None)`.
    pub fn optional<T>(&mut self, setting: &Setting) -> Result<Option<T>, Invalid>
    where
        T: FromStr,
        T::Err: fmt::Display,
    {
        self.optional_with(setting, |raw| raw.parse::<T>().map_err(|error| error.to_string()))
    }

    /// Như [`optional`](Self::optional) nhưng tự viết hàm đổi kiểu.
    pub fn optional_with<T>(
        &mut self,
        setting: &Setting,
        parse: impl FnOnce(&str) -> Result<T, String>,
    ) -> Result<Option<T>, Invalid> {
        match self.read(setting, true) {
            Some(raw) => self.parse(setting, &raw, parse).map(Some),
            None => Ok(None),
        }
    }

    /// Ghi một lỗi cho `setting` khi giá trị đúng kiểu nhưng không hợp với biến
    /// khác (vd hai biến mâu thuẫn nhau). `problem` nói rõ phải sửa gì.
    pub fn reject<T>(
        &mut self,
        setting: &Setting,
        problem: impl Into<String>,
    ) -> Result<T, Invalid> {
        self.fail(setting, problem.into())
    }

    /// Kết thúc việc đọc. Có lỗi nào thì trả toàn bộ lỗi; không thì trả giá trị đã
    /// dựng kèm danh sách biến đã đọc (để in ra hoặc sinh `.env.example`).
    pub fn finish<T>(
        self,
        value: Result<T, Invalid>,
    ) -> Result<(T, Vec<SettingValue>), ConfigErrors> {
        match value {
            Ok(value) if self.errors.is_empty() => Ok((value, self.values)),
            Ok(_) => Err(ConfigErrors(self.errors)),
            Err(Invalid) if !self.errors.is_empty() => Err(ConfigErrors(self.errors)),
            // `Invalid` mà không có lỗi nào được ghi: code đọc cấu hình tự tạo
            // `Invalid` thay vì đi qua reader. Vẫn phải báo, không được coi là hợp lệ.
            Err(Invalid) => Err(ConfigErrors::single(ConfigError {
                subject: "configuration".to_owned(),
                problem: "a setting was rejected without an explanation (this is a bug)".to_owned(),
            })),
        }
    }

    /// Danh sách biến đã đọc tới lúc này, kể cả khi có lỗi.
    pub fn values(&self) -> &[SettingValue] {
        &self.values
    }

    /// Lấy giá trị thô (đã đặt, hoặc mặc định) và ghi biến vào danh sách đã đọc.
    fn read(&mut self, setting: &Setting, optional: bool) -> Option<String> {
        let (raw, origin) = match (self.source.get(setting.name), setting.default) {
            (Some(configured), _) => (Some(configured.to_owned()), Origin::Configured),
            (None, Some(default)) => (Some(default.to_owned()), Origin::Default),
            (None, None) => (None, Origin::Unset),
        };
        self.values.push(SettingValue { setting: *setting, origin, optional, raw: raw.clone() });
        raw
    }

    fn parse<T>(
        &mut self,
        setting: &Setting,
        raw: &str,
        parse: impl FnOnce(&str) -> Result<T, String>,
    ) -> Result<T, Invalid> {
        match parse(raw) {
            Ok(value) => Ok(value),
            // Không bao giờ chép giá trị của secret vào thông báo lỗi: thông báo
            // đi vào log và màn hình.
            Err(reason) if setting.secret => self.fail(setting, reason),
            Err(reason) => self.fail(setting, format!("{reason} (got `{raw}`)")),
        }
    }

    fn fail<T>(&mut self, setting: &Setting, problem: String) -> Result<T, Invalid> {
        self.errors.push(ConfigError { subject: setting.name.to_owned(), problem });
        Err(Invalid)
    }
}

/// Đổi `raw` thành số nguyên nằm trong `[min, max]`; dùng với
/// [`ConfigReader::get_with`]. Thông báo lỗi nói rõ khoảng hợp lệ.
pub fn parse_in_range<T>(raw: &str, min: T, max: T) -> Result<T, String>
where
    T: FromStr + PartialOrd + fmt::Display,
{
    match raw.parse::<T>() {
        Ok(value) if value >= min && value <= max => Ok(value),
        _ => Err(format!("must be a whole number between {min} and {max}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn so_ngoai_khoang_hoac_khong_phai_so_bi_tu_choi() {
        assert_eq!(parse_in_range("5", 1u32, 10), Ok(5));
        assert_eq!(parse_in_range("1", 1u32, 10), Ok(1));
        assert_eq!(parse_in_range("10", 1u32, 10), Ok(10));
        for raw in ["0", "11", "-1", "abc", "1.5", ""] {
            assert_eq!(
                parse_in_range(raw, 1u32, 10),
                Err("must be a whole number between 1 and 10".to_owned()),
                "{raw}"
            );
        }
    }

    #[test]
    fn reject_ghi_loi_cho_dung_bien() {
        let source = source(&[]);
        let mut reader = ConfigReader::new(&source);
        assert_eq!(reader.reject::<()>(&PORT, "conflicts with NAME"), Err(Invalid));
        let errors = reader.finish::<()>(Err(Invalid)).unwrap_err();
        assert_eq!(errors.errors()[0].to_string(), "PORT: conflicts with NAME");
    }

    const PORT: Setting = Setting::new("PORT", "Port to listen on.").with_default("8080");
    const NAME: Setting = Setting::new("NAME", "Name of the thing.");
    const TOKEN: Setting = Setting::new("TOKEN", "Access token.").secret();
    const LIMIT: Setting = Setting::new("LIMIT", "Optional limit.");

    fn source(pairs: &[(&str, &str)]) -> ConfigSource {
        ConfigSource::from_pairs(pairs.iter().copied())
    }

    #[test]
    fn gia_tri_da_dat_thang_mac_dinh() {
        let source = source(&[("PORT", "9000")]);
        let mut reader = ConfigReader::new(&source);
        assert_eq!(reader.get::<u16>(&PORT), Ok(9000));
        assert_eq!(reader.values()[0].origin, Origin::Configured);
    }

    #[test]
    fn khong_dat_thi_dung_mac_dinh() {
        let source = source(&[]);
        let mut reader = ConfigReader::new(&source);
        assert_eq!(reader.get::<u16>(&PORT), Ok(8080));
        assert_eq!(reader.values()[0].origin, Origin::Default);
        assert_eq!(reader.values()[0].display_value(), "8080");
    }

    #[test]
    fn bien_bat_buoc_ma_thieu_la_loi_co_mo_ta() {
        let source = source(&[]);
        let mut reader = ConfigReader::new(&source);
        assert_eq!(reader.get::<String>(&NAME), Err(Invalid));

        let errors = reader.finish::<()>(Err(Invalid)).unwrap_err();
        assert_eq!(
            errors.to_string(),
            "invalid configuration (1 problem):\n  \
             - NAME: is required but not set. Name of the thing."
        );
    }

    #[test]
    fn gia_tri_sai_kieu_la_loi_co_gia_tri_da_nhan() {
        let source = source(&[("PORT", "eighty")]);
        let mut reader = ConfigReader::new(&source);
        assert_eq!(reader.get::<u16>(&PORT), Err(Invalid));

        let errors = reader.finish::<()>(Err(Invalid)).unwrap_err();
        assert_eq!(errors.errors()[0].subject, "PORT");
        assert_eq!(errors.errors()[0].problem, "invalid digit found in string (got `eighty`)");
    }

    #[test]
    fn gom_moi_loi_roi_moi_bao() {
        let source = source(&[("PORT", "x"), ("TOKEN", "short")]);
        let mut reader = ConfigReader::new(&source);
        let port = reader.get::<u16>(&PORT);
        let name = reader.get::<String>(&NAME);
        let token = reader.get_with(&TOKEN, |raw| {
            if raw.len() >= 32 {
                Ok(raw.to_owned())
            } else {
                Err("must be at least 32 characters".to_owned())
            }
        });
        let built = (|| Ok((port?, name?, token?)))();

        let errors = reader.finish(built).unwrap_err();

        let subjects: Vec<&str> = errors.errors().iter().map(|e| e.subject.as_str()).collect();
        assert_eq!(subjects, ["PORT", "NAME", "TOKEN"]);
        assert!(errors.to_string().starts_with("invalid configuration (3 problems):"));
    }

    #[test]
    fn secret_khong_bao_gio_bi_in_ra() {
        let source = source(&[("TOKEN", "hunter2")]);
        let mut reader = ConfigReader::new(&source);
        let token =
            reader.get_with::<String>(&TOKEN, |_| Err("must be at least 32 characters".to_owned()));
        assert_eq!(reader.values()[0].display_value(), "(set, hidden)");

        let message = reader.finish(token).unwrap_err().to_string();

        assert!(message.contains("TOKEN: must be at least 32 characters"), "{message}");
        assert!(!message.contains("hunter2"), "{message}");
    }

    #[test]
    fn bien_tuy_chon_khong_dat_thi_la_none() {
        let source = source(&[]);
        let mut reader = ConfigReader::new(&source);
        assert_eq!(reader.optional::<u32>(&LIMIT), Ok(None));
        let value = &reader.values()[0];
        assert!(value.optional);
        assert_eq!((value.origin, value.display_value()), (Origin::Unset, "(not set)"));
        assert!(reader.finish(Ok(())).is_ok());
    }

    #[test]
    fn bien_tuy_chon_da_dat_thi_van_duoc_kiem() {
        let good = source(&[("LIMIT", "10")]);
        assert_eq!(ConfigReader::new(&good).optional::<u32>(&LIMIT), Ok(Some(10)));

        let bad = source(&[("LIMIT", "-1")]);
        let mut reader = ConfigReader::new(&bad);
        assert_eq!(reader.optional::<u32>(&LIMIT), Err(Invalid));
        assert!(reader.finish(Ok(())).is_err());
    }

    #[test]
    fn hop_le_thi_tra_gia_tri_va_danh_sach_bien_da_doc() {
        let source = source(&[("NAME", "autoxide")]);
        let mut reader = ConfigReader::new(&source);
        let port = reader.get::<u16>(&PORT);
        let name = reader.get::<String>(&NAME);
        let built = (|| Ok((port?, name?)))();

        let ((port, name), values) = reader.finish(built).unwrap();

        assert_eq!((port, name.as_str()), (8080, "autoxide"));
        let names: Vec<&str> = values.iter().map(|value| value.setting.name).collect();
        assert_eq!(names, ["PORT", "NAME"]);
    }

    #[test]
    fn invalid_khong_kem_loi_van_bi_tu_choi() {
        let source = source(&[]);
        let reader = ConfigReader::new(&source);
        let errors = reader.finish::<()>(Err(Invalid)).unwrap_err();
        assert!(errors.to_string().contains("this is a bug"));
    }
}
