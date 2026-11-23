//! Search suggestions for the website (D-201): when a shopper types in the search box, the website asks one
//! endpoint and gets suggestions collected from several modules — products, categories (`catalog`), articles
//! (`content`), stores (`organizations`). A module contributes a [`SuggestSource`] with
//! [`StartContext::suggest_source`](crate::module::StartContext::suggest_source); the module that serves the
//! endpoint holds the [`SuggestSources`] obtained with
//! [`StartContext::suggest_sources`](crate::module::StartContext::suggest_sources).
//!
//! Two-tier ranking computed in the application (D-202): suggestions that match with diacritics (`exact`)
//! come first, then those matching without diacritics — in Vietnamese `dấu` / `dầu` / `dâu` mean different
//! things. One failing source does not break the answer: it is logged and the suggestions of the remaining
//! sources are returned.

use std::sync::{Arc, PoisonError, RwLock};

use uuid::Uuid;

use crate::error::Error;
use crate::tenancy::{Site, TenantContext};

/// Maximum number of suggestions per query, across all sources.
pub const MAX_SUGGESTIONS: u16 = 10;

/// One search suggestion.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Suggestion {
    /// Kind, `<module>.<kind>`, e.g. `catalog.product`, `content.entry`.
    pub kind: String,
    /// Text shown to the user.
    pub label: String,
    /// Path on the website (no domain, no language prefix); `None`: the suggestion is a
    /// search phrase (clicking it searches for `label`).
    pub path: Option<String>,
    /// Record ID, when the suggestion is a record.
    pub id: Option<Uuid>,
    /// Matches with diacritics (ranked first); `false`: matches only when diacritics are removed.
    pub exact: bool,
    /// Short context, e.g. "in Shirts" for a category suggestion; empty if none.
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

/// The suggestion source of one module. A source only reads the public data of the tenant `tenant`, visible on
/// the website `site` (like the module's own public reads) — never another tenant's.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait SuggestSource: Send + Sync {
    /// Suggestion kind, `<module>.<kind>`.
    fn kind(&self) -> &'static str;

    /// At most `limit` suggestions for the typed phrase `q` (trimmed, non-empty) in the language `locale`.
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

/// Registry of suggestion sources. Shared (`Arc`): a source registered after the serving module obtained the registry is still present.
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

    /// Suggestions from all sources for `q`: diacritic-exact matches first, then in source registration
    /// order; at most `limit` (clamped to `1..=`[`MAX_SUGGESTIONS`]). Failing sources are skipped (logged).
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

    /// Whether there are any sources yet.
    pub fn is_empty(&self) -> bool {
        self.entries.read().unwrap_or_else(PoisonError::into_inner).is_empty()
    }
}

/// Two-tier ordering: diacritic-exact matches first, keeping the order within each tier; drops duplicate
/// suggestions (same kind and text); cuts at `limit`.
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
