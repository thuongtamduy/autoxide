//! Gọi điện trong chat (bước 61 mục 4, D-232): đường dây của nút, người mua gọi từ trình duyệt hay
//! xin gọi lại, nhân viên gọi lại, nhật ký cuộc gọi trong dòng thời gian, ghi âm tắt mặc định. Mọi
//! lời gọi nhà cung cấp (`VoiceProvider` của kernel) đi NGOÀI transaction ghi.

pub(crate) mod callbacks;
pub(crate) mod calls;
pub(crate) mod hours;
pub(crate) mod lines;
pub(crate) mod recordings;
pub(crate) mod work;
