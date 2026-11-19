//! Kiểm banner trước khi lưu (bước 55, D-236): mọi trường, ảnh (kích thước `media` đã lưu, không
//! tải lại tệp), link và trang được chọn (sổ đối tượng, mỗi loại một lần). Gom mọi lỗi để báo một
//! lần (D-083). Chạy TRƯỚC transaction ghi: gọi module khác.

use std::collections::{HashMap, HashSet};

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_media::api::MediaVisibility;
use uuid::Uuid;

use super::items::{Banner, BannerFields, Link, STATUSES};
use super::placements::Placement;
use super::{
    MAX_PAGE_TARGETS, TONES, check_choice, check_page_kinds, image_fits, invalid, strings,
};
use crate::service::{ContentService, check_length, now};

/// Ảnh cần kiểm: (trường, ID, tỉ lệ, cỡ tối thiểu rộng × cao).
type ImageRule<'a> = (&'a str, Uuid, Option<(i32, i32)>, (Option<i32>, Option<i32>));

impl ContentService {
    /// Kiểm mọi trường của banner trên vị trí `placement` và dựng dòng (chưa có ID, nút, thứ tự).
    /// Gom mọi lỗi để báo một lần (D-083).
    pub(crate) async fn check_banner(
        &self,
        tenant: &TenantContext,
        placement: &Placement,
        fields: BannerFields,
        before: Option<&Banner>,
    ) -> Result<Banner, Error> {
        let mut problems = Vec::new();
        // Ảnh: có, công khai, đủ cỡ và đúng tỉ lệ theo kích thước `media` đã lưu.
        self.check_images(tenant, placement, &fields, &mut problems).await?;
        let languages = self.languages(tenant).await?;
        let locale = match (fields.locale.as_deref(), before) {
            (None, Some(before)) => before.locale.clone(),
            (raw, _) => self.check_locale(&languages, raw, "locale", &mut problems),
        };
        let kind = fields.kind.unwrap_or_else(|| "image".to_owned());
        let key = &placement.key;
        if check_choice("kind", &kind, &["image", "text"], &mut problems)
            && placement.accepts != "any"
            && placement.accepts != kind
        {
            problems.push(
                FieldError::new(
                    "kind",
                    "kind_not_allowed",
                    "placement `{key}` only takes {accepts}",
                )
                .with_param("key", key)
                .with_param("accepts", &placement.accepts),
            );
        }
        let title = fields.title.trim().to_owned();
        check_length("title", Some(&title), 120, &mut problems);
        check_length("subtitle", Some(&fields.subtitle), 300, &mut problems);
        check_length("cta_label", Some(&fields.cta_label), 60, &mut problems);
        check_length("alt", Some(&fields.alt), 300, &mut problems);
        let status = fields.status.unwrap_or_else(|| "draft".to_owned());
        check_choice("status", &status, &STATUSES, &mut problems);
        let mut tone = fields.tone;
        if kind == "text" {
            if title.is_empty() {
                problems.push(FieldError::new("title", "required", "a text banner needs a title"));
            }
            for (field, value) in
                [("media_id", fields.media_id), ("mobile_media_id", fields.mobile_media_id)]
            {
                if value.is_some() {
                    problems.push(FieldError::new(
                        field,
                        "invalid",
                        "a text banner has no picture",
                    ));
                }
            }
            let chosen = tone.get_or_insert_with(|| "neutral".to_owned());
            check_choice("tone", chosen, &TONES, &mut problems);
        } else {
            if fields.media_id.is_none() {
                problems.push(FieldError::new(
                    "media_id",
                    "required",
                    "an image banner needs a picture",
                ));
            }
            if tone.is_some() {
                problems.push(FieldError::new("tone", "invalid", "only text banners have a tone"));
            }
            tone = None;
        }
        if let (Some(starts), Some(ends)) = (fields.starts_at, fields.ends_at)
            && ends <= starts
        {
            problems.push(FieldError::new(
                "ends_at",
                "window_invalid",
                "must be after `starts_at`",
            ));
        }
        // Loại trang: tập con của loại trang của vị trí.
        let allowed = strings(&placement.page_kinds);
        let page_kinds = check_page_kinds("page_kinds", &fields.page_kinds, true, &mut problems);
        for (index, page_kind) in page_kinds.iter().enumerate() {
            if !allowed.iter().any(|kind| kind == "any" || kind == page_kind) {
                problems.push(
                    FieldError::new(
                        format!("page_kinds[{index}]"),
                        "page_kind_not_allowed",
                        "placement `{key}` is not used on `{page_kind}` pages",
                    )
                    .with_param("key", key)
                    .with_param("page_kind", page_kind),
                );
            }
        }
        if fields.page_targets.len() > MAX_PAGE_TARGETS {
            problems.push(
                FieldError::new("page_targets", "length", "must list at most {max} pages")
                    .with_param("max", MAX_PAGE_TARGETS),
            );
        }
        // Link và trang cụ thể: hỏi sổ đối tượng, mỗi loại một lần.
        let mut wanted: Vec<(String, String, String, &str)> = Vec::new();
        let (link_kind, link_url, link_id) = match &fields.link {
            Link::None => ("none".to_owned(), None, None),
            Link::Url(url) => {
                check_link_url(url, &mut problems);
                ("url".to_owned(), Some(url.trim().to_owned()), None)
            }
            Link::Ref { kind, id } => {
                wanted.push(("link".to_owned(), kind.clone(), id.clone(), "link_target_missing"));
                (kind.clone(), None, Some(id.clone()))
            }
        };
        for (index, page) in fields.page_targets.iter().enumerate() {
            wanted.push((
                format!("page_targets[{index}]"),
                page.kind.clone(),
                page.id.clone(),
                "page_target_not_found",
            ));
        }
        self.check_refs(tenant, &locale, &wanted, &mut problems).await?;
        if !problems.is_empty() {
            return Err(invalid(problems));
        }
        Ok(Banner {
            id: Uuid::nil(),
            tenant_id: Uuid::nil(),
            placement_id: placement.id,
            org_id: Uuid::nil(),
            locale,
            kind,
            title,
            subtitle: fields.subtitle.trim().to_owned(),
            cta_label: fields.cta_label.trim().to_owned(),
            tone,
            media_id: fields.media_id,
            mobile_media_id: fields.mobile_media_id,
            alt: fields.alt.trim().to_owned(),
            page_kinds: serde_json::to_string(&page_kinds).unwrap_or_else(|_| "[]".to_owned()),
            page_targets: serde_json::to_string(&fields.page_targets)
                .unwrap_or_else(|_| "[]".to_owned()),
            link_kind,
            link_url,
            link_id,
            starts_at: fields.starts_at,
            ends_at: fields.ends_at,
            position: 0,
            status,
            ai_assisted: fields.ai_assisted,
            created_by: None,
            created_at: before.map_or_else(now, |row| row.created_at),
            updated_at: now(),
        })
    }

    async fn check_images(
        &self,
        tenant: &TenantContext,
        placement: &Placement,
        fields: &BannerFields,
        problems: &mut Vec<FieldError>,
    ) -> Result<(), Error> {
        let ratio = placement.ratio_w.zip(placement.ratio_h);
        let mobile_ratio = placement.mobile_ratio_w.zip(placement.mobile_ratio_h).or(ratio);
        let wanted: Vec<ImageRule<'_>> = [
            ("media_id", fields.media_id, ratio, (placement.min_width, placement.min_height)),
            ("mobile_media_id", fields.mobile_media_id, mobile_ratio, (None, None)),
        ]
        .into_iter()
        .filter_map(|(field, id, ratio, min)| Some((field, id?, ratio, min)))
        .collect();
        if wanted.is_empty() {
            return Ok(());
        }
        let ids: Vec<Uuid> = wanted.iter().map(|(_, id, _, _)| *id).collect();
        let found: HashMap<Uuid, autoxide_media::api::MediaRef> = self
            .media
            .find_many(tenant, &ids)
            .await?
            .into_iter()
            .map(|item| (item.id, item))
            .collect();
        for (field, id, ratio, min) in wanted {
            match found.get(&id) {
                None => problems.push(FieldError::new(field, "media_not_found", "does not exist")),
                Some(item) if item.visibility != MediaVisibility::Public => {
                    problems.push(FieldError::new(
                        field,
                        "media_not_public",
                        "is not public; make it public first",
                    ))
                }
                Some(item) if !image_fits(item.width, item.height, ratio, min) => {
                    let ratio_text =
                        ratio.map_or_else(|| "any".to_owned(), |(w, h)| format!("{w}:{h}"));
                    problems.push(
                        FieldError::new(
                            field,
                            "image_size_invalid",
                            "placement `{key}` needs ratio {ratio} and at least {min_width}×{min_height} px",
                        )
                        .with_param("key", &placement.key)
                        .with_param("ratio", ratio_text)
                        .with_param("min_width", min.0.unwrap_or(0))
                        .with_param("min_height", min.1.unwrap_or(0)),
                    );
                }
                Some(_) => {}
            }
        }
        Ok(())
    }

    /// Đối tượng `(chỗ trong JSON, loại, ID, mã lỗi khi không có)`: loại đã đăng ký, đối tượng có.
    async fn check_refs(
        &self,
        tenant: &TenantContext,
        locale: &str,
        wanted: &[(String, String, String, &str)],
        problems: &mut Vec<FieldError>,
    ) -> Result<(), Error> {
        let mut by_kind: HashMap<&str, Vec<String>> = HashMap::new();
        for (path, kind, id, _) in wanted {
            if self.resources.knows(kind) {
                by_kind.entry(kind).or_default().push(id.clone());
            } else {
                problems.push(
                    FieldError::new(
                        format!("{path}.kind"),
                        "link_kind_unavailable",
                        "no installed module provides `{kind}`",
                    )
                    .with_param("kind", kind),
                );
            }
        }
        let mut found: HashSet<(String, String)> = HashSet::new();
        for (kind, ids) in by_kind {
            let resolved = self.resources.resolve_many(tenant, kind, &ids, Some(locale)).await?;
            found.extend(resolved.into_keys().map(|id| (kind.to_owned(), id)));
        }
        for (path, kind, id, code) in wanted {
            if self.resources.knows(kind) && !found.contains(&(kind.clone(), id.clone())) {
                problems.push(
                    FieldError::new(format!("{path}.id"), *code, "no `{kind}` has the id `{id}`")
                        .with_param("kind", kind)
                        .with_param("id", id),
                );
            }
        }
        Ok(())
    }
}

/// Địa chỉ của link: `https://`, `http://` hay đường dẫn `/…` (không `//`).
fn check_link_url(url: &str, problems: &mut Vec<FieldError>) {
    let url = url.trim();
    let path = url.starts_with('/') && !url.starts_with("//");
    let lower = url.to_ascii_lowercase();
    let web = lower.starts_with("https://") || lower.starts_with("http://");
    if !(path || web)
        || url.len() > 2048
        || url.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        problems.push(FieldError::new(
            "link.url",
            "invalid",
            "must be an `https://` or `http://` address, or a path starting with `/`",
        ));
    }
}
