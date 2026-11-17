//! "Có gì mới" (đề bài giao diện, nút "?"): các mục của `CHANGELOG.md` theo phiên bản, đọc từ bản
//! nhúng lúc biên dịch — đúng những gì máy chủ đang chạy có, không phải bản trên mạng.
//!
//! Mọi module là crate của cùng một workspace, phát hành cùng nhau, nên một CHANGELOG là đủ.
//! Mục `Unreleased` là thay đổi đã có trong bản build nhưng chưa gắn số phiên bản.

use axum::extract::Query;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::{Error, FieldError};
use crate::response::{Data, Problem};

/// CHANGELOG của workspace, nhúng lúc biên dịch.
const CHANGELOG: &str = include_str!("../../../../CHANGELOG.md");

/// Tối đa số phiên bản trả về khi không nêu `since`.
const DEFAULT_RELEASES: usize = 5;

/// The changes of one version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct Release {
    /// `0.14.0`, or `unreleased` for changes in this build that have no version yet.
    #[schema(example = "0.14.0")]
    pub version: String,
    /// Release day (`YYYY-MM-DD`); `null` for `unreleased`.
    #[schema(example = "2026-11-15")]
    pub date: Option<String>,
    /// The changes by kind, in the order of the changelog: `added`, `changed`, `deprecated`,
    /// `removed`, `fixed`, `security`. Each item is Markdown (in English).
    pub sections: Vec<ReleaseSection>,
}

/// One kind of change in a release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct ReleaseSection {
    /// `added`, `changed`, `deprecated`, `removed`, `fixed` or `security`.
    #[schema(example = "added")]
    pub kind: String,
    /// One Markdown paragraph per change.
    pub items: Vec<String>,
}

/// Which versions to list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct WhatsNewQuery {
    /// Only versions newer than this one (e.g. the last version a user saw), and `unreleased`.
    /// Omitted: the five newest.
    #[param(example = "0.13.0")]
    since: Option<String>,
}

/// Đọc CHANGELOG thành danh sách phiên bản, mới nhất trước (đúng thứ tự trong file).
pub(crate) fn parse(text: &str) -> Vec<Release> {
    let mut releases: Vec<Release> = Vec::new();
    let mut in_item = false;
    for line in text.lines() {
        if let Some(head) = line.strip_prefix("## [") {
            let (version, rest) = head.split_once(']').unwrap_or((head, ""));
            let date = rest.trim().trim_start_matches('-').trim();
            let unreleased = version.eq_ignore_ascii_case("unreleased");
            releases.push(Release {
                version: if unreleased { "unreleased".to_owned() } else { version.to_owned() },
                date: (!unreleased && !date.is_empty()).then(|| date.to_owned()),
                sections: Vec::new(),
            });
            in_item = false;
            continue;
        }
        let Some(release) = releases.last_mut() else { continue };
        if let Some(kind) = line.strip_prefix("### ") {
            release
                .sections
                .push(ReleaseSection { kind: kind.trim().to_lowercase(), items: Vec::new() });
            in_item = false;
            continue;
        }
        let Some(section) = release.sections.last_mut() else { continue };
        if let Some(item) = line.strip_prefix("- ") {
            section.items.push(item.trim().to_owned());
            in_item = true;
        } else if line.trim().is_empty() {
            in_item = false;
        } else if in_item && let Some(last) = section.items.last_mut() {
            // Dòng nối của cùng một mục (CHANGELOG ngắt dòng ở 120 cột).
            last.push(' ');
            last.push_str(line.trim());
        }
    }
    for release in &mut releases {
        release.sections.retain(|section| !section.items.is_empty());
    }
    releases.retain(|release| !release.sections.is_empty());
    releases
}

/// `1.2.3` thành bộ số để so; `None` khi sai dạng.
fn version_key(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.trim().trim_start_matches('v').split('.');
    let mut next = || parts.next()?.parse::<u64>().ok();
    let key = (next()?, next()?, next()?);
    Some(key)
}

/// Các phiên bản mới hơn `since` (kèm `unreleased`); không nêu `since`: năm bản mới nhất.
pub(crate) fn select(releases: Vec<Release>, since: Option<&str>) -> Result<Vec<Release>, Error> {
    let Some(since) = since.map(str::trim).filter(|since| !since.is_empty()) else {
        return Ok(releases.into_iter().take(DEFAULT_RELEASES).collect());
    };
    let floor = version_key(since).ok_or_else(|| {
        Error::validation(vec![FieldError::new(
            "since",
            "invalid",
            "must be a version such as `0.14.0`",
        )])
    })?;
    Ok(releases
        .into_iter()
        .filter(|release| version_key(&release.version).is_none_or(|key| key > floor))
        .collect())
}

pub(crate) fn routes() -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(whats_new))
}

/// List what is new in this server.
///
/// No sign-in needed. The changes of each version of the server that is running, newest first,
/// from its changelog (in English): what was added, changed, fixed… With `since` (e.g. the last
/// version a user saw), only the newer versions; `unreleased` lists changes in this build that
/// have no version number yet. All modules share the server's version.
#[utoipa::path(
    get,
    path = "/whats-new",
    extensions(("x-public" = json!(true))),
    operation_id = "get_whats_new",
    security((), ("bearer" = [])),
    params(WhatsNewQuery),
    responses(
        (status = 200, description = "The versions, newest first.", body = Data<Vec<Release>>),
        (status = 422, description = "`since` is not a version; see `errors`.", body = Problem),
    )
)]
async fn whats_new(Query(query): Query<WhatsNewQuery>) -> Result<Data<Vec<Release>>, Error> {
    Ok(Data { data: select(parse(CHANGELOG), query.since.as_deref())? })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Changelog\n\nIntro.\n\n## [Unreleased]\n\n### Added\n\n- New thing\n  over two lines.\n- Other.\n\n## [0.2.0] - 2026-01-02\n\n### Fixed\n\n- A bug.\n\n### Removed\n\n## [0.1.0] - 2026-01-01\n\n### Added\n\n- First.\n";

    #[test]
    fn doc_changelog_theo_phien_ban() {
        let releases = parse(SAMPLE);
        assert_eq!(releases.len(), 3);
        assert_eq!(
            (releases[0].version.as_str(), releases[0].date.as_deref()),
            ("unreleased", None)
        );
        assert_eq!(releases[0].sections[0].kind, "added");
        assert_eq!(releases[0].sections[0].items, ["New thing over two lines.", "Other."]);
        assert_eq!(releases[1].date.as_deref(), Some("2026-01-02"));
        assert_eq!(releases[1].sections.len(), 1, "mục rỗng bị bỏ");
    }

    #[test]
    fn loc_theo_since() {
        let newer = select(parse(SAMPLE), Some("0.1.0")).unwrap();
        let versions: Vec<_> = newer.iter().map(|release| release.version.as_str()).collect();
        assert_eq!(versions, ["unreleased", "0.2.0"]);
        assert!(select(parse(SAMPLE), Some("abc")).is_err());
        assert_eq!(select(parse(SAMPLE), None).unwrap().len(), 3);
    }

    #[test]
    fn changelog_that_doc_duoc() {
        let releases = parse(CHANGELOG);
        assert!(releases.len() > 3, "CHANGELOG của repo phải đọc ra nhiều phiên bản");
        assert!(releases.iter().all(|release| !release.sections.is_empty()));
    }
}
