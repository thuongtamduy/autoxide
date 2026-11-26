//! Tính năng dùng AI của `content` (D-109): dịch bài (`translate`, `segments`), tìm theo ý nghĩa
//! (`semantic`), SEO (`seo`). Mô hình gọi qua trait của kernel; kết quả chỉ là gợi ý hay bản nháp.

pub(crate) mod campaign;
pub(crate) mod pages;
pub(crate) mod public_limits;
pub(crate) mod segments;
pub(crate) mod semantic;
pub(crate) mod seo;
pub(crate) mod translate;
