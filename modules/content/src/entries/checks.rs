//! Kiểm tham chiếu (bản ghi, media, bài cha) và tạo bản dịch.

use super::*;

impl ContentService {
    /// Bản ghi nhúng trong bài (bước 51 P6): loại phải có module đăng ký trong sổ đối tượng
    /// (D-123) và bản ghi phải có thật. Không đòi công khai lúc lưu: đọc công khai tự bỏ bản ghi
    /// chưa (hay không còn) công khai. Tra theo lô, mỗi loại một lần, ngoài transaction.
    pub(crate) async fn check_records(
        &self,
        tenant: &TenantContext,
        records: &BTreeSet<(String, String)>,
        problems: &mut Vec<FieldError>,
    ) -> Result<(), Error> {
        let mut by_kind: std::collections::BTreeMap<&str, Vec<String>> = Default::default();
        for (kind, id) in records {
            by_kind.entry(kind.as_str()).or_default().push(id.clone());
        }
        for (kind, ids) in by_kind {
            // Khu dùng chung là của chính `content`, không qua sổ đối tượng.
            if kind == crate::shared::KIND {
                self.check_shared_ids(tenant, &ids, problems).await?;
                continue;
            }
            if !self.resources.knows(kind) {
                problems.push(
                    FieldError::new(
                        "body",
                        "embed_kind_unknown",
                        "no installed module provides records of type `{kind}`",
                    )
                    .with_param("kind", kind),
                );
                continue;
            }
            let found = self.resources.resolve_many(tenant, kind, &ids, None).await?;
            for id in ids.iter().filter(|id| !found.contains_key(*id)) {
                problems.push(
                    FieldError::new("body", "embed_not_found", "there is no `{kind}` `{id}`")
                        .with_param("kind", kind)
                        .with_param("id", id),
                );
            }
        }
        Ok(())
    }

    /// Ảnh mục dùng (ảnh đại diện, ảnh chia sẻ, ảnh trong bài) có tồn tại — và, khi
    /// `public`, có công khai — không. Gọi `media` ngoài transaction.
    pub(crate) async fn check_media(
        &self,
        tenant: &TenantContext,
        entry: &Entry,
        in_body: &BTreeSet<Uuid>,
        public: bool,
        problems: &mut Vec<FieldError>,
    ) -> Result<(), Error> {
        let mut wanted: Vec<(String, Uuid)> = Vec::new();
        if let Some(id) = entry.featured_media_id {
            wanted.push(("featured_media_id".to_owned(), id));
        }
        if let Some(id) = entry.og_media_id {
            wanted.push(("og_media_id".to_owned(), id));
        }
        wanted.extend(in_body.iter().map(|id| ("body".to_owned(), *id)));
        self.check_media_ids(tenant, wanted, public, problems).await
    }

    /// Ảnh `wanted` (trường báo lỗi, ID) có tồn tại — và, khi `public`, có công khai — không.
    pub(crate) async fn check_media_ids(
        &self,
        tenant: &TenantContext,
        wanted: Vec<(String, Uuid)>,
        public: bool,
        problems: &mut Vec<FieldError>,
    ) -> Result<(), Error> {
        if wanted.is_empty() {
            return Ok(());
        }
        let ids: Vec<Uuid> =
            wanted.iter().map(|(_, id)| *id).collect::<BTreeSet<_>>().into_iter().collect();
        let mut found = std::collections::HashMap::new();
        for chunk in ids.chunks(autoxide_media::api::MAX_IDS) {
            for item in self.media.find_many(tenant, chunk).await? {
                found.insert(item.id, item);
            }
        }
        for (field, id) in wanted {
            match found.get(&id) {
                None => problems.push(
                    FieldError::new(field, "media_not_found", "media item {id} does not exist")
                        .with_param("id", id),
                ),
                Some(item)
                    if public
                        && item.visibility != autoxide_media::api::MediaVisibility::Public =>
                {
                    problems.push(
                        FieldError::new(field, "media_not_public", "media item {id} is not public")
                            .with_param("id", id),
                    );
                }
                Some(_) => {}
            }
        }
        Ok(())
    }

    /// Kiểm trang cha `parent` cho `entry`: là trang, cùng ngôn ngữ, cây không quá sâu.
    pub(crate) async fn check_parent<C: ConnectionTrait>(
        &self,
        tenant: &TenantContext,
        connection: &C,
        entry: &Entry,
        parent: Uuid,
        problems: &mut Vec<FieldError>,
    ) -> Result<(), Error> {
        let Some(row) = Entity::find_by_id(parent).scoped(tenant).one(connection).await? else {
            problems.push(FieldError::new("parent_id", "not_found", "no page has this id"));
            return Ok(());
        };
        if row.kind != Kind::Page.as_str() || row.locale != entry.locale {
            problems.push(FieldError::new(
                "parent_id",
                "invalid",
                "must be a page in the same language",
            ));
            return Ok(());
        }
        let below = subtree_height(connection, tenant, entry.id).await?;
        if depth(connection, tenant, parent).await? + below > MAX_TREE_DEPTH {
            problems.push(
                FieldError::new(
                    "parent_id",
                    "tree_too_deep",
                    "pages may nest at most {max_tree_depth} levels",
                )
                .with_param("max_tree_depth", MAX_TREE_DEPTH),
            );
        }
        Ok(())
    }

    /// Tạo bản dịch của mục `source_id` sang ngôn ngữ `locale`: một mục mới (nháp) cùng nhóm
    /// bản dịch, chép nội dung từ bản gốc rồi áp `fields`. Bản dịch của trang có cha là bản
    /// dịch của trang cha trong ngôn ngữ đó. Cần `content.write`, hoặc `content.write_own` khi
    /// bản gốc là của mình.
    pub(crate) async fn create_translation(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        source_id: Uuid,
        locale: &str,
        fields: EntryFields,
    ) -> Result<Entry, Error> {
        permissions::require_create(principal)?;
        let source = Entity::find_by_id(source_id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| entry_not_found(source_id))?;
        permissions::require_write(principal, &source)?;
        permissions::require_org(principal, tenant, &source, permissions::Need::Write).await?;
        let mut problems = Vec::new();
        let target = self.check_locale(
            &self.languages(tenant).await?,
            Some(locale),
            "locale",
            &mut problems,
        );
        if problems.is_empty() && target == source.locale {
            problems.push(FieldError::new(
                "locale",
                "invalid",
                "must differ from the language of the entry",
            ));
        }
        if !problems.is_empty() {
            return Err(invalid(problems));
        }
        let sibling = Entity::find()
            .scoped(tenant)
            .filter(Column::TranslationGroupId.eq(source.translation_group_id))
            .filter(Column::Locale.eq(target.as_str()))
            .one(tenant.database().read())
            .await?;
        if sibling.is_some() {
            return Err(Error::conflict(
                "content.translation_exists",
                "This entry already has a translation in `{locale}`.",
            )
            .with_param("locale", &target));
        }
        let parent = match source.parent_id {
            None => None,
            Some(parent) => {
                let parent = Entity::find_by_id(parent)
                    .scoped(tenant)
                    .one(tenant.database().read())
                    .await?
                    .ok_or_else(|| entry_not_found(parent))?;
                let translated = Entity::find()
                    .scoped(tenant)
                    .filter(Column::TranslationGroupId.eq(parent.translation_group_id))
                    .filter(Column::Locale.eq(target.as_str()))
                    .one(tenant.database().read())
                    .await?;
                match translated {
                    Some(translated) => {
                        // Bản dịch nằm dưới bản dịch của trang cha: cùng luật "trang cha phải là
                        // của mình" như khi tạo trang con.
                        permissions::require_parent(principal, &translated)?;
                        Some(translated.id)
                    }
                    None => {
                        return Err(Error::conflict(
                            "content.parent_not_translated",
                            "The parent page has no `{locale}` translation yet. Translate it first.",
                        )
                        .with_param("locale", &target));
                    }
                }
            }
        };
        let now = now();
        let mut candidate = source.clone();
        candidate.id = Uuid::now_v7();
        candidate.locale = target;
        candidate.parent_id = parent;
        candidate.parent_key = parent.unwrap_or(ROOT);
        candidate.status = Status::Draft.as_str().to_owned();
        candidate.has_unpublished_changes = false;
        candidate.created_by = Some(principal.user_id());
        candidate.created_by_api_key_id = principal.api_key_id();
        // Bản dịch là mục mới: không mang cờ của bản gốc; ai tạo bằng AI thì tự khai (hay máy
        // chủ đặt khi nó dịch).
        candidate.ai_generated = false;
        candidate.ai_assisted = false;
        candidate.restricted_at = None;
        candidate.review_requested_at = None;
        candidate.review_requested_by = None;
        candidate.translated_from_revision_id = None;
        candidate.created_at = now;
        candidate.updated_at = now;
        candidate.revised_at = now;
        // Không gửi slug: lấy slug của bản gốc; gửi `""`: sinh từ tiêu đề (đã dịch). Cả hai tự
        // thêm `-2`, `-3`… khi trùng.
        let auto_slug = fields.slug.as_deref().is_none_or(|slug| slug.trim().is_empty());
        self.apply(tenant, &mut candidate, fields, &mut problems).await?;
        mark_agent_written(principal, &mut candidate);
        if !problems.is_empty() {
            return Err(invalid(problems));
        }
        self.insert(tenant, principal, candidate, auto_slug, Some(source.id)).await
    }

    pub(super) async fn translation_taken(
        &self,
        tenant: &TenantContext,
        candidate: &Entry,
    ) -> Result<bool, Error> {
        Ok(Entity::find()
            .scoped(tenant)
            .filter(Column::TranslationGroupId.eq(candidate.translation_group_id))
            .filter(Column::Locale.eq(candidate.locale.as_str()))
            .one(tenant.database().write())
            .await?
            .is_some())
    }
}
