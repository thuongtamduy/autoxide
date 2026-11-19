//! Rào quyền tối đa cho khách con (bước 54 P4, như SCP của AWS; D-20261119-step54-guardrails).
//!
//! - Khách cấp trên (hay nền tảng) đặt cho một khách cấp dưới danh sách permission tối đa. Không role
//!   nào ở khách đó hay khách cháu của nó — kể cả `admin` — vượt được; người từ khách cha vào bằng
//!   uỷ quyền cũng bị rào. Quyền thật = quyền theo role ∩ mọi rào trên đường từ gốc tới khách.
//! - Khách con không nới được rào cha đặt: nó chỉ đặt rào cho khách của chính nó ở dưới, và mọi rào
//!   giao nhau.
//! - Lưu ở database nền tảng (`auth_tenant_guardrails`, cạnh lần vào hỗ trợ): rào của cả chuỗi tổ
//!   tiên đọc bằng một câu. Kết quả cache theo khách [`CACHE_SECONDS`] giây và xoá ngay khi đổi
//!   rào trên máy này — đường nóng không thêm truy vấn mỗi request.
//! - Khách bị xoá hẳn (`kernel.tenant_deleted`): rào đặt cho nó và rào nó đặt bị xoá.

pub(crate) mod entity;
mod service;

pub(crate) use service::{GuardrailService, GuardrailSwitch, GuardrailView, subscriptions};
