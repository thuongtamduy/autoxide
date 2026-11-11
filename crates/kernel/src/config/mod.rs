//! Cấu hình: đọc từ biến môi trường (và file `.env`), kiểm lúc khởi động.
//!
//! - Mỗi biến khai một lần bằng [`Setting`] ở cạnh code dùng nó.
//! - [`ConfigSource`] gom giá trị thô; [`ConfigReader`] đổi kiểu, kiểm, và gom
//!   mọi lỗi để báo một lượt.
//! - [`Config`] là cấu hình của kernel. Module đọc cấu hình riêng của mình bằng
//!   cùng một [`ConfigReader`], nên lỗi của kernel và của mọi module hiện chung
//!   trong một thông báo.
//!
//! Quy ước tên biến: tên chuẩn ngành nếu có (`DATABASE_URL`), còn lại mang tiền
//! tố `AUTOXIDE_` (docs/DECISIONS.md D-034).

mod database;
mod env_example;
mod reader;
mod setting;
mod source;

pub use database::{DatabaseBackend, DatabaseConfig, DatabaseUrl, redact_password};
pub use env_example::render_env_example;
pub use reader::{
    ConfigError, ConfigErrors, ConfigReader, Invalid, Origin, SettingValue, parse_in_range,
};
pub use setting::Setting;
pub use source::ConfigSource;

/// Cấu hình của kernel. Mỗi mục của lộ trình thêm nhóm của mình vào đây.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub database: DatabaseConfig,
}

impl Config {
    /// Đọc cấu hình của kernel. Lỗi được ghi vào `reader`; gọi
    /// [`ConfigReader::finish`] sau khi mọi phần (kernel và module) đã đọc xong.
    pub fn read(reader: &mut ConfigReader) -> Result<Self, Invalid> {
        let database = DatabaseConfig::read(reader);
        Ok(Config { database: database? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn khong_dat_gi_thi_cau_hinh_mac_dinh_hop_le() {
        // Người lạ clone về chạy ngay được: mọi biến đều có mặc định dùng được.
        let source = ConfigSource::default();
        let mut reader = ConfigReader::new(&source);
        let config = Config::read(&mut reader);

        let (config, values) = reader.finish(config).unwrap();

        assert_eq!(config.database.url.backend(), DatabaseBackend::Sqlite);
        assert!(values.iter().all(|value| value.origin != Origin::Configured));
    }
}
