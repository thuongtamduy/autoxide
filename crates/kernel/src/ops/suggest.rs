//! Gợi ý tìm cho website (D-201): khi người mua gõ vào ô tìm, website hỏi một endpoint và nhận gợi
//! ý gom từ nhiều module — sản phẩm, danh mục (`catalog`), bài viết (`content`), cửa hàng
//! (`organizations`). Module góp một [`SuggestSource`] bằng
//! [`StartContext::suggest_source`](crate::module::StartContext::suggest_source); module phục vụ
//! endpoint giữ [`SuggestSources`] lấy bằng
//! [`StartContext::suggest_sources`](crate::module::StartContext::suggest_sources).
//!
//! Xếp hạng hai tầng tính trong ứng dụng (D-202): gợi ý khớp đúng dấu (`exact`) trước, khớp không
//! dấu sau — tiếng Việt "dấu / dầu / dâu" khác nghĩa. Một nguồn lỗi không làm hỏng câu trả lời:
//! ghi log và trả gợi ý của các nguồn còn lại.

use std::sync::{Arc, PoisonError, RwLock};

use uuid::Uuid;

use crate::error::Error;
use crate::tenancy::{Site, TenantContext};

/// Số gợi ý tối đa của một lần hỏi, gom mọi nguồn.
pub const MAX_SUGGESTIONS: u16 = 10;

/// Một gợi ý tìm.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Suggestion {
    /// Loại, `<module>.<loại>`, vd `catalog.product`, `content.entry`.
    pub kind: String,
    /// Chữ hiện cho người dùng.
    pub label: String,
    /// Đường dẫn trên website (không có tên miền, không có tiền tố ngôn ngữ); `None`: gợi ý là một
    /// câu tìm (bấm vào thì tìm `label`).
    pub path: Option<String>,
    /// ID bản ghi, khi gợi ý là một bản ghi.
    pub id: Option<Uuid>,
    /// Khớp đúng dấu (xếp trước); `false`: chỉ khớp khi bỏ dấu.
    pub exact: bool,
    /// Ngữ cảnh ngắn, vd "trong Áo" của gợi ý danh mục; rỗng nếu không có.
    pub context: String,
}

impl Suggestion {
    pub fn new(kind: impl Into<String>, label: impl Into<String>, exact: bool) -> Self {
        Suggestion {
            kind: kind.into(),
            label: label.into(),
            path: None,
            id: None,
            exact,
            context: String::new(),
        }
    }

    #[must_use]
    pub fn path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    #[must_use]
    pub fn id(mut self, id: Uuid) -> Self {
        self.id = Some(id);
        self
    }

    #[must_use]
    pub fn context(mut self, context: impl Into<String>) -> Self {
        self.context = context.into();
        self
    }
}

/// Nguồn gợi ý của một module. Nguồn chỉ đọc dữ liệu công khai của khách `tenant`, thấy được ở
/// website `site` (như đọc công khai của chính module đó) — không bao giờ của khách khác.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait SuggestSource: Send + Sync {
    /// Loại gợi ý, `<module>.<loại>`.
    fn kind(&self) -> &'static str;

    /// Tối đa `limit` gợi ý cho câu gõ `q` (đã cắt khoảng trắng, không rỗng) ở ngôn ngữ `locale`.
    async fn suggest(
        &self,
        tenant: &TenantContext,
        site: &Site,
        locale: &str,
        q: &str,
        limit: u16,
    ) -> Result<Vec<Suggestion>, Error>;
}

type Entry = (&'static str, Arc<dyn SuggestSource>);

/// Sổ nguồn gợi ý. Dùng chung (`Arc`): nguồn đăng ký sau lúc module phục vụ lấy sổ vẫn có mặt.
#[derive(Clone, Default)]
pub struct SuggestSources {
    entries: Arc<RwLock<Vec<Entry>>>,
}

impl SuggestSources {
    pub(crate) fn register(
        &self,
        module: &'static str,
        source: Arc<dyn SuggestSource>,
    ) -> Result<(), Error> {
        let kind = source.kind();
        if !kind.starts_with(&format!("{module}.")) {
            return Err(Error::internal(format!(
                "suggest source `{kind}` of module `{module}` must be named `{module}.<type>`"
            )));
        }
        let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        if entries.iter().any(|(_, known)| known.kind() == kind) {
            return Err(Error::internal(format!("suggest source `{kind}` is registered twice")));
        }
        entries.push((module, source));
        Ok(())
    }

    /// Gợi ý của mọi nguồn cho `q`: khớp đúng dấu trước, rồi theo thứ tự đăng ký nguồn; tối đa
    /// `limit` (kẹp vào `1..=`[`MAX_SUGGESTIONS`]). Nguồn lỗi bị bỏ qua (ghi log).
    pub async fn suggest(
        &self,
        tenant: &TenantContext,
        site: &Site,
        locale: &str,
        q: &str,
        limit: u16,
    ) -> Vec<Suggestion> {
        let limit = limit.clamp(1, MAX_SUGGESTIONS);
        let q = q.trim();
        if q.is_empty() {
            return Vec::new();
        }
        // Chép danh sách ra trước khi `await`: không giữ lock qua điểm chờ.
        let entries: Vec<Entry> =
            self.entries.read().unwrap_or_else(PoisonError::into_inner).clone();
        let mut found = Vec::new();
        for (module, source) in entries {
            match source.suggest(tenant, site, locale, q, limit).await {
                Ok(items) => found.extend(items.into_iter().take(usize::from(limit))),
                Err(error) => {
                    tracing::warn!(module, kind = source.kind(), %error, "suggest source failed; left out");
                }
            }
        }
        rank(found, limit)
    }

    /// Có nguồn nào chưa.
    pub fn is_empty(&self) -> bool {
        self.entries.read().unwrap_or_else(PoisonError::into_inner).is_empty()
    }
}

/// Xếp hai tầng: khớp đúng dấu trước, giữ thứ tự trong mỗi tầng; bỏ gợi ý trùng (cùng loại và
/// chữ); cắt ở `limit`.
fn rank(found: Vec<Suggestion>, limit: u16) -> Vec<Suggestion> {
    let (mut exact, loose): (Vec<_>, Vec<_>) = found.into_iter().partition(|item| item.exact);
    exact.extend(loose);
    let mut out: Vec<Suggestion> = Vec::new();
    for item in exact {
        if out.iter().any(|seen| seen.kind == item.kind && seen.label == item.label) {
            continue;
        }
        out.push(item);
        if out.len() >= usize::from(limit) {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Suggestion, rank};

    #[test]
    fn khop_dung_dau_xep_truoc_va_bo_trung() {
        let found = vec![
            Suggestion::new("catalog.product", "dầu gội", false),
            Suggestion::new("catalog.product", "dấu", true),
            Suggestion::new("content.entry", "dâu tây", false),
            Suggestion::new("catalog.product", "dấu", true),
        ];
        let ranked = rank(found, 3);
        let labels: Vec<&str> = ranked.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(labels, vec!["dấu", "dầu gội", "dâu tây"]);
    }
}
