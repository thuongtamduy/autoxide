//! "Đang dùng ở đâu" (bước 51, D-267): bản ghi nào của module nào đang trỏ tới một bản ghi (bài
//! viết dùng ảnh, menu trỏ tới bài…). Module khai [`ReferenceSource`] — chính nó đọc bảng của nó
//! (luật kiến trúc 1); kernel gom qua `GET /api/v1/kernel/records/{kind}/{id}/usages`, chỉ trả bản
//! ghi của module mà người gọi có `<module>.read`.

use std::sync::{Arc, PoisonError, RwLock};

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::Error;
use crate::tenancy::TenantContext;

/// Một bản ghi đang trỏ tới bản ghi được hỏi.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Referrer {
    /// Loại của bản ghi trỏ tới, vd `content.entry`.
    #[schema(example = "content.entry")]
    pub kind: String,
    pub id: String,
    /// What staff call it (title, name).
    pub title: String,
    /// How it points: e.g. `featured_media`, `body`, `menu_item`.
    #[schema(example = "featured_media")]
    pub field: String,
}

impl Referrer {
    pub fn new(
        kind: impl Into<String>,
        id: impl Into<String>,
        title: impl Into<String>,
        field: impl Into<String>,
    ) -> Self {
        Referrer { kind: kind.into(), id: id.into(), title: title.into(), field: field.into() }
    }
}

/// Nguồn tham chiếu của một module.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait ReferenceSource: Send + Sync {
    /// Tối đa `limit` bản ghi của module đang trỏ tới bản ghi `kind` / `id` của khách `tenant`.
    /// Loại không trỏ tới được thì trả rỗng.
    async fn referrers(
        &self,
        tenant: &TenantContext,
        kind: &str,
        id: &str,
        limit: usize,
    ) -> Result<Vec<Referrer>, Error>;
}

type Entry = (&'static str, Arc<dyn ReferenceSource>);

/// Sổ nguồn tham chiếu của mọi module. Clone rẻ.
#[derive(Clone, Default)]
pub struct ReferenceSources {
    entries: Arc<RwLock<Vec<Entry>>>,
}

impl ReferenceSources {
    pub(crate) fn register(&self, module: &'static str, source: Arc<dyn ReferenceSource>) {
        self.entries.write().unwrap_or_else(PoisonError::into_inner).push((module, source));
    }

    pub(crate) fn all(&self) -> Vec<Entry> {
        self.entries.read().unwrap_or_else(PoisonError::into_inner).clone()
    }
}
