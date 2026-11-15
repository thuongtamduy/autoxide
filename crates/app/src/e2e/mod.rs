//! Test đầu-cuối của chương trình thật: dựng mọi module như `autoxide serve`, gọi qua HTTP, kiểm
//! cả database, nơi lưu file và audit log. Nằm trong `src/` (không ở `crates/app/tests/`) vì cần
//! phần bên trong của binary, như danh sách module ở `modules.rs`.

pub(crate) mod access;
pub(crate) mod guides;
pub(crate) mod media;
pub(crate) mod migrations;
