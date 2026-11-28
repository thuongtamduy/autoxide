//! Module mẫu `notes`: ghi chú cá nhân (D-069).
//!
//! Đây là KHUÔN cho module về sau, dùng đủ mọi phần của hợp đồng module: migration,
//! permission, service, REST + OpenAPI, event (vào audit log), tool MCP, bản dịch, và
//! trait công bố cho module khác. Không đăng ký sẵn trong binary — core trung lập với
//! nghiệp vụ. Cách tạo module mới từ khuôn này: `docs/MODULE_GUIDE.md`.

pub mod api;
mod entity;
mod events;
mod http;
mod migration;
mod permissions;
mod service;
mod tools;

#[cfg(test)]
mod branch_tests;
#[cfg(test)]
mod sync_tests;
#[cfg(test)]
mod tenancy_tests;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Subscription;
use autoxide_kernel::module::{Manifest, Module, StartContext};

use crate::service::NoteService;

/// Module `notes`, để binary đăng ký trong `crates/app/src/modules.rs`.
pub fn module() -> impl Module {
    NotesModule
}

struct NotesModule;

// Hợp đồng module (D-20261126-module-manifest): `manifest` khai mọi thứ biết được khi chưa khởi động, `start` dựng
// service rồi đăng ký những gì cần service (route, tool MCP, đăng ký nhận event, trait cho module khác).
impl Module for NotesModule {
    fn manifest(&self) -> Manifest {
        Manifest::new(
            "notes",
            "Sample module: personal notes, showing every part of the module contract.",
        )
        // `migration::all` là con trỏ hàm: kernel tự gọi khi cần, manifest vẫn rẻ để dựng lại.
        .migrations(migration::all)
        .records([autoxide_kernel::db::RecordInfo::of::<entity::Note>()])
        .permissions(permissions::all())
        .events(events::all())
        .translation("vi", include_str!("../locales/vi.json"))
        .limits([service::NOTES_LIMIT])
        // Ghi chú đồng bộ được với app ngoại tuyến (bước 69 P2): luồng thay đổi ở `GET /api/v1/notes/changes`, ghi
        // qua lô `POST /api/v1/kernel/sync/batch`. Đồng bộ còn ở hạng thử nghiệm nên khai trong `unstable`.
        .unstable(|unstable| unstable.sync_resources([service::NOTES_SYNC]))
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        // `kernel::<…>()`: một cách lấy mọi handle của kernel; kiểu đích cho trình biên dịch biết lấy handle nào.
        let service =
            Arc::new(NoteService::new(context.kernel()?, context.kernel()?, context.kernel()?));
        context.usage_counter(&service::NOTES_LIMIT, Arc::new(service::NoteCounter))?;
        // `Arc<NoteService>` tự đổi thành `Arc<dyn Notes>` / `Arc<dyn PersonalData>`…: bên nhận chỉ thấy trait.
        context.personal_data(service.clone())?;
        // Tài khoản bị xoá: xoá ghi chú của người đó.
        context.subscribe(Subscription::new(
            autoxide_kernel::privacy::USER_DELETED,
            service.clone(),
        ))?;
        context.routes(http::routes(service.clone()))?;
        context.tools(tools::all(&service))?;
        context.provide::<dyn api::Notes>(service)
    }

    // Chế độ "nghỉ" (bước 70 P9, D-20261125-dormant-modules): module có trong chương trình nhưng đang tắt
    // vẫn xuất và xoá ghi chú của người dùng — mẫu cho module khác giữ dữ liệu cá nhân.
    fn dormant(
        &self,
        context: &mut autoxide_kernel::module::DormantContext<'_>,
    ) -> Result<(), Error> {
        let part = Arc::new(service::DormantNotes { sync: context.sync_log() });
        context.personal_data(part.clone())?;
        context.on_user_deleted(part)
    }
}
