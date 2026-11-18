//! Bộ máy tìm kiếm thay được (bước 40 P7, D-220): module khai **loại tài liệu tìm được**
//! ([`SearchKind`] + [`SearchSource`]), kernel giữ chỉ mục qua một [`SearchBackend`]. Mặc định là
//! [`DatabaseSearch`](super::DatabaseSearch) — bảng `kernel_search_docs` trong chính database của khách,
//! khớp đúng dấu xếp trước khớp không dấu (D-202), chạy như nhau trên SQLite và Postgres. Bộ tìm
//! chuyên dụng (Meilisearch, Typesense, OpenSearch…) là module riêng cài trait này và đăng ký bằng
//! `StartContext::search_backend`; chọn bằng `AUTOXIDE_SEARCH_BACKEND` — module nghiệp vụ không sửa gì.
//!
//! Khác ô tìm "chứa chuỗi" trên cột khoá tìm của từng module ([`fold`](super::fold),
//! [`contains`](super::contains)) và tìm xuyên phân hệ (`autoxide_kernel::find`): hai thứ đó giữ
//! nguyên (không phá API); module chuyển sang chỉ mục khi bảng của nó lớn tới mức "chứa" quét chậm.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::access::Permission;
use crate::error::Error;
use crate::pagination::{Page, PageRequest};
use crate::tenancy::{OrgId, TenantContext};

/// Một tài liệu trong chỉ mục: bản chụp phần tìm được của một bản ghi.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchDoc {
    /// Loại, `<module>.<loại>` (như sổ đối tượng), vd `catalog.product`.
    pub kind: String,
    /// ID bản ghi.
    pub id: Uuid,
    /// Nút sở hữu bản ghi (lọc theo phạm vi quyền ngay trong backend).
    pub org_id: OrgId,
    /// Ngôn ngữ của nội dung (`vi`, `en`…); rỗng: không theo ngôn ngữ.
    pub locale: String,
    /// Trường tìm được, theo tên đã khai trong [`SearchKind::fields`].
    pub fields: BTreeMap<String, String>,
    /// Giá trị lọc, theo tên đã khai trong [`SearchKind::filters`] (chuỗi, số hay bool).
    pub filters: BTreeMap<String, Value>,
}

impl SearchDoc {
    pub fn new(kind: impl Into<String>, id: Uuid, org_id: OrgId) -> Self {
        SearchDoc {
            kind: kind.into(),
            id,
            org_id,
            locale: String::new(),
            fields: BTreeMap::new(),
            filters: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn locale(mut self, locale: impl Into<String>) -> Self {
        self.locale = locale.into();
        self
    }

    #[must_use]
    pub fn field(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.fields.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn filter(mut self, name: impl Into<String>, value: impl Into<Value>) -> Self {
        self.filters.insert(name.into(), value.into());
        self
    }
}

/// Khai báo một loại tài liệu tìm được.
#[derive(Debug, Clone)]
pub struct SearchKind {
    /// `<module>.<loại>`.
    pub kind: &'static str,
    /// Trường tìm được và trọng số (lớn hơn xếp trước; trọng số cao nhất là "tiêu đề").
    pub fields: Vec<(&'static str, u8)>,
    /// Tên các bộ lọc so bằng.
    pub filters: Vec<&'static str>,
    /// Permission để đọc kết quả; phạm vi nút của nó giới hạn kết quả. `None`: công khai.
    pub permission: Option<Permission>,
}

impl SearchKind {
    pub fn new(kind: &'static str) -> Self {
        SearchKind { kind, fields: Vec::new(), filters: Vec::new(), permission: None }
    }

    #[must_use]
    pub fn field(mut self, name: &'static str, weight: u8) -> Self {
        self.fields.push((name, weight));
        self
    }

    #[must_use]
    pub fn filter(mut self, name: &'static str) -> Self {
        self.filters.push(name);
        self
    }

    #[must_use]
    pub fn permission(mut self, permission: Permission) -> Self {
        self.permission = Some(permission);
        self
    }

    /// Trường theo trọng số giảm dần (cùng trọng số: theo thứ tự khai).
    pub fn ranked_fields(&self) -> Vec<&'static str> {
        let mut fields = self.fields.clone();
        fields.sort_by_key(|(_, weight)| std::cmp::Reverse(*weight));
        fields.into_iter().filter(|(_, weight)| *weight > 0).map(|(name, _)| name).collect()
    }
}

/// Phạm vi nút của một truy vấn, lấy từ grant của người gọi (như `Principal::scope_condition`).
/// Backend phải lọc NGAY trong chỉ mục, không lọc sau (trang kết quả mới đúng).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrgScope {
    /// Bản ghi ở các nút này hay nút con của chúng.
    pub subtrees: Vec<OrgId>,
    /// Bản ghi ở đúng các nút này.
    pub units: Vec<OrgId>,
    /// Hàng rào: chỉ trong cây con này (thêm vào điều kiện trên).
    pub fence: Option<OrgId>,
}

/// Một truy vấn trên một loại tài liệu.
#[derive(Debug, Clone)]
pub struct SearchRequest {
    pub kind: String,
    /// Chữ người dùng gõ; rỗng: mọi tài liệu (theo bộ lọc).
    pub text: String,
    /// Bộ lọc so bằng (tên đã khai → giá trị dạng chuỗi).
    pub filters: BTreeMap<String, String>,
    pub locale: Option<String>,
    /// `None`: cả khách.
    pub scope: Option<OrgScope>,
    pub page: PageRequest,
}

/// Một kết quả.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchHit {
    pub kind: String,
    pub id: Uuid,
    pub org_id: OrgId,
    pub locale: String,
    pub fields: BTreeMap<String, String>,
}

/// Một trang kết quả (`limit` / `cursor` như mọi danh sách).
pub type SearchHits = Page<SearchHit>;

/// Bộ máy giữ chỉ mục. Mọi hàm nhận khách: tài liệu của khách này không bao giờ ra ở khách khác.
/// Cài đặt phải chịu được gọi lại (job đồng bộ chạy lại sau lỗi): `index` là upsert, `remove`
/// tài liệu không có là thành công.
#[async_trait::async_trait]
pub trait SearchBackend: Send + Sync {
    /// Thêm hay thay tài liệu.
    async fn index(&self, tenant: &TenantContext, docs: &[SearchDoc]) -> Result<(), Error>;
    /// Bỏ tài liệu `ids` của loại `kind`.
    async fn remove(&self, tenant: &TenantContext, kind: &str, ids: &[Uuid]) -> Result<(), Error>;
    /// Bỏ mọi tài liệu của loại `kind` (trước khi dựng lại chỉ mục).
    async fn clear(&self, tenant: &TenantContext, kind: &str) -> Result<(), Error>;
    /// Tìm. `kind` của truy vấn đã được kiểm là loại có khai; bộ lọc chỉ gồm tên đã khai.
    async fn query(
        &self,
        tenant: &TenantContext,
        kind: &SearchKind,
        request: &SearchRequest,
    ) -> Result<SearchHits, Error>;
}

/// Nơi module cung cấp tài liệu của một loại (đọc từ bảng của chính module).
#[async_trait::async_trait]
pub trait SearchSource: Send + Sync {
    /// Tài liệu hiện tại của các bản ghi `ids`; bản ghi đã xoá thì vắng (bị bỏ khỏi chỉ mục).
    async fn load(&self, tenant: &TenantContext, ids: &[Uuid]) -> Result<Vec<SearchDoc>, Error>;
    /// Tối đa `limit` tài liệu có ID lớn hơn `after`, theo ID tăng dần (dựng lại chỉ mục).
    async fn scan(
        &self,
        tenant: &TenantContext,
        after: Option<Uuid>,
        limit: u64,
    ) -> Result<Vec<SearchDoc>, Error>;
}
