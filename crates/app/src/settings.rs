//! Cấu hình của cả chương trình: kernel cộng (về sau) từng module.
//!
//! Đây là chỗ duy nhất quyết định "chương trình này đọc những biến nào". Mọi lệnh
//! của CLI đi qua [`load`], nên cấu hình sai bị chặn như nhau ở mọi lệnh.

use std::path::Path;

use autoxide_kernel::config::{
    Config, ConfigErrors, ConfigReader, ConfigSource, Invalid, SettingValue,
};

/// Tên file cấu hình cục bộ, tìm trong thư mục đang đứng.
pub const DOTENV_FILE: &str = ".env";

/// Cấu hình đã kiểm, kèm danh sách mọi biến đã đọc (để in ra).
#[derive(Debug)]
pub struct Settings {
    pub config: Config,
    pub values: Vec<SettingValue>,
}

/// Đọc mọi nhóm cấu hình của chương trình bằng `reader`.
fn read_all(reader: &mut ConfigReader) -> Result<Config, Invalid> {
    // Khi có module: đọc cấu hình của từng module ở đây, cùng `reader`.
    Config::read(reader)
}

/// Đọc và kiểm toàn bộ cấu hình từ `source`.
pub fn read(source: &ConfigSource) -> Result<Settings, ConfigErrors> {
    let mut reader = ConfigReader::new(source);
    let config = read_all(&mut reader);
    let (config, values) = reader.finish(config)?;
    Ok(Settings { config, values })
}

/// Danh sách mọi biến chương trình đọc, với giá trị mặc định — không phụ thuộc
/// cấu hình hiện tại (kể cả khi nó đang sai). Dùng để sinh `.env.example`.
pub fn describe() -> Vec<SettingValue> {
    let source = ConfigSource::default();
    let mut reader = ConfigReader::new(&source);
    // Bỏ qua kết quả: biến bắt buộc đương nhiên "thiếu" ở đây; chỉ cần danh sách.
    let _ = read_all(&mut reader);
    reader.values().to_vec()
}

/// Đọc cấu hình từ môi trường của tiến trình và file `.env` trong thư mục hiện tại.
pub fn load() -> Result<Settings, ConfigErrors> {
    read(&ConfigSource::load(Path::new(DOTENV_FILE))?)
}
