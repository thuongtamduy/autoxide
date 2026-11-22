//! Tạo, ghi và đọc bài: create / insert / get / list / published_paths.

use super::*;
use autoxide_kernel::time::Timestamp;

impl ContentService {
    /// Tạo một mục nháp. Cần `content.write` hoặc `content.write_own` (khi đó trang cha phải là
    /// của mình, và mục luôn mang `ai_generated`). Slug để trống thì sinh từ tiêu đề (trùng thì
    /// thêm `-2`, `-3`…).
    pub(crate) async fn create(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewEntry,
    ) -> Result<Entry, Error> {
        permissions::require_create(principal)?;
        if let Some(parent) = input.parent_id {
            // Trang cha không có thì `check_parent` báo bên dưới, cùng các lỗi khác.
            if let Some(row) =
                Entity::find_by_id(parent).scoped(tenant).one(tenant.database().read()).await?
            {
                permissions::require_parent(principal, &row)?;
            }
        }
        let mut problems = Vec::new();
        let locale = self.check_locale(
            &self.languages(tenant).await?,
            input.locale.as_deref(),
            "locale",
            &mut problems,
        );
        if input.fields.title.as_deref().is_none_or(|title| title.trim().is_empty()) {
            problems.push(
                FieldError::new("title", "required", "must be 1 to {title_max} characters")
                    .with_param("title_max", TITLE_MAX),
            );
        }
        if input.parent_id.is_some() && input.kind != Kind::Page {
            problems.push(FieldError::new("parent_id", "not_allowed", "only pages have a parent"));
        }
        // Mục của loại tự định nghĩa (D-235): loại phải có và đang bật; slug không trùng trong loại.
        let content_type = match (input.kind, input.type_id) {
            (Kind::Custom, Some(type_id)) => {
                let found = crate::entity::content_type::Entity::find_by_id(type_id)
                    .scoped(tenant)
                    .one(tenant.database().read())
                    .await?
                    .filter(|row| row.active);
                if found.is_none() {
                    problems.push(FieldError::new(
                        "type_id",
                        "not_found",
                        "is not an active content type",
                    ));
                }
                found
            }
            (Kind::Custom, None) => {
                problems.push(FieldError::new(
                    "type_id",
                    "required",
                    "entries of kind `custom` need a content type",
                ));
                None
            }
            (_, Some(_)) => {
                problems.push(FieldError::new(
                    "type_id",
                    "not_allowed",
                    "only entries of kind `custom` have a content type",
                ));
                None
            }
            (_, None) => None,
        };
        let id = Uuid::now_v7();
        let now = now();
        let mut candidate =
            blank(tenant, id, input.kind, &locale, input.parent_id, principal.user_id(), now);
        candidate.created_by_api_key_id = principal.api_key_id();
        if let Some(row) = &content_type {
            candidate.type_id = Some(row.id);
            candidate.parent_key = row.id;
        }
        let field_target = content_type.as_ref().map_or_else(
            || crate::resource::ENTRY.to_owned(),
            |row| crate::types::field_target(&row.key),
        );
        // Nút: người tạo nêu, hay nút duy nhất họ viết được (bước 34). Kiểm phạm vi trong `insert`.
        let writer = if principal.has(&permissions::CONTENT_WRITE) {
            &permissions::CONTENT_WRITE
        } else {
            &permissions::CONTENT_WRITE_OWN
        };
        match input
            .org_id
            .map(autoxide_kernel::tenancy::OrgId::from_uuid)
            .or_else(|| principal.home_org(tenant.id(), writer))
        {
            Some(org) => candidate.org_id = Some(org.as_uuid()),
            None => problems.push(FieldError::new(
                "org_id",
                "required",
                "you work in several organization units: choose the unit of the entry",
            )),
        }
        let auto_slug = input.fields.slug.is_none();
        let mut fields = input.fields;
        let custom = fields.custom_fields.take();
        candidate.custom_fields = self
            .custom_values(tenant, &field_target, custom.as_ref(), None, true, &mut problems)
            .await?;
        self.apply(tenant, &mut candidate, fields, &mut problems).await?;
        mark_agent_written(principal, &mut candidate);
        if let Some(parent) = input.parent_id {
            self.check_parent(tenant, tenant.database().read(), &candidate, parent, &mut problems)
                .await?;
        }
        if !problems.is_empty() {
            return Err(invalid(problems));
        }
        self.insert(tenant, principal, candidate, auto_slug, None).await
    }

    /// Ghi `candidate` (đã kiểm) cùng revision `initial` và event. Slug tự sinh mà trùng
    /// (kể cả do một request khác vừa lấy) thì thử hậu tố kế tiếp.
    pub(crate) async fn insert(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        mut candidate: Entry,
        auto_slug: bool,
        source: Option<Uuid>,
    ) -> Result<Entry, Error> {
        let base = candidate.slug.clone();
        let field_target = self.entry_field_target(tenant, &candidate).await?;
        for _ in 0..SLUG_ATTEMPTS {
            let transaction = tenant.database().begin_write().await?;
            // Nút của mục mới phải trong phạm vi viết của người tạo, và đang hoạt động (bước 34).
            permissions::require_write_at(principal, &transaction, tenant, &candidate).await?;
            if let Some(parent) = candidate.parent_id {
                // Khoá cây của ngôn ngữ (cùng khoá với xoá, chuyển, đăng) rồi kiểm lại trang cha
                // trong transaction: không tạo được trang con dưới một trang đang bị xoá, và độ
                // sâu tính trên cây hiện tại (một lệnh `move` vừa xong có thể đã đổi nó; bản dịch
                // lấy cha trong ngôn ngữ khác, có thể đã sâu hơn).
                crate::publishing::paths::lock_tree(&transaction, tenant, &candidate.locale)
                    .await?;
                let mut problems = Vec::new();
                self.check_parent(tenant, &transaction, &candidate, parent, &mut problems).await?;
                if !problems.is_empty() {
                    transaction.rollback().await?;
                    return Err(invalid(problems));
                }
                if !lock(&transaction, tenant, parent).await? {
                    return Err(invalid(vec![FieldError::new(
                        "parent_id",
                        "not_found",
                        "no page has this id",
                    )]));
                }
            }
            if auto_slug {
                candidate.slug = free_slug(&transaction, tenant, &candidate, &base).await?;
            } else if slug_taken(&transaction, tenant, &candidate).await? {
                return Err(slug_conflict(&candidate.slug));
            }
            let inserted = candidate.clone().into_active_model().insert(&transaction).await;
            let entry = match inserted {
                Ok(entry) => entry,
                Err(error) if is_unique_violation(&error) => {
                    transaction.rollback().await?;
                    if self.translation_taken(tenant, &candidate).await? {
                        return Err(Error::conflict(
                            "content.translation_exists",
                            "This entry already has a translation in `{locale}`.",
                        )
                        .with_param("locale", &candidate.locale));
                    }
                    if !auto_slug {
                        return Err(slug_conflict(&candidate.slug));
                    }
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            if let Some(values) = stored_custom(&entry) {
                self.custom_fields
                    .write_index(&transaction, tenant, "content", &field_target, entry.id, &values)
                    .await?;
            }
            revisions::record(
                &transaction,
                tenant,
                &entry,
                Reason::Initial,
                Some(principal.user_id()),
                entry.created_at,
            )
            .await?;
            let mut after = summary(&entry);
            if let (Some(source), Some(object)) = (source, after.as_object_mut()) {
                object.insert("translation_of".to_owned(), json!(source));
            }
            self.enqueue_index(tenant, &transaction, entry.id).await?;
            let event =
                events::EntryCreated(Change { before: None, after: Some(after), published: None });
            self.outbox
                .record(
                    &transaction,
                    tenant,
                    Actor::from(principal),
                    events::entry_subject(entry.id),
                    &event,
                )
                .await?;
            transaction.commit().await?;
            tracing::info!(entry_id = %entry.id, kind = %entry.kind, "content entry created");
            return Ok(entry);
        }
        Err(Error::conflict("content.slug_taken", "Could not find a free slug; give one yourself."))
    }

    /// Mục `id`. Cần `content.read`.
    pub(crate) async fn get(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Entry, Error> {
        principal.require(&CONTENT_READ)?;
        let entry = Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| entry_not_found(id))?;
        permissions::require_org(principal, tenant, &entry, permissions::Need::Read).await?;
        Ok(entry)
    }

    /// Một trang mục theo bộ lọc. Cần `content.read`.
    pub(crate) async fn list(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: &EntryFilter,
        page: &PageRequest,
    ) -> Result<Page<Entry>, Error> {
        principal.require(&CONTENT_READ)?;
        // Theo nút (bước 34): chỉ mục trong phạm vi `content.read`; `own` = mục mình tạo.
        let scope =
            principal.scope_condition(&CONTENT_READ, Column::OrgId, Some(Column::CreatedBy))?;
        let mut select = Entity::find().filter(scope).scoped(tenant);
        if let Some(org) = filter.owner_org {
            select = select.filter(Column::OrgId.eq(org));
        }
        if let Some(org) = filter.site {
            let site = autoxide_kernel::tenancy::Site::of(
                tenant,
                autoxide_kernel::tenancy::OrgId::from_uuid(org),
            )
            .await?;
            select = select.filter(Column::OrgId.is_in(site.lineage_ids()));
        }
        if let Some(kind) = filter.kind {
            select = select.filter(Column::Kind.eq(kind.as_str()));
        }
        if let Some(type_id) = filter.type_id {
            select = select.filter(Column::TypeId.eq(type_id));
        }
        if let Some(locale) = &filter.locale {
            select = select.filter(Column::Locale.eq(locale.as_str()));
        }
        if let Some(status) = filter.status {
            select = select.filter(Column::Status.eq(status.as_str()));
        }
        select = match filter.parent {
            ParentFilter::Any => select,
            ParentFilter::Root => select.filter(Column::ParentKey.eq(ROOT)),
            ParentFilter::Id(id) => select.filter(Column::ParentKey.eq(id)),
        };
        if let Some(search) = filter.search.as_deref().filter(|search| !search.trim().is_empty()) {
            select = select.filter(autoxide_kernel::search::contains(Column::SearchKey, search));
        }
        if let Some(group) = filter.translation_group_id {
            select = select.filter(Column::TranslationGroupId.eq(group));
        }
        if let Some(term) = filter.term_id {
            select = select.filter(crate::taxonomy::terms::has_term(tenant, term));
        }
        if let Some(author) = filter.author_id {
            select = select.filter(crate::taxonomy::authors::has_author(tenant, author));
        }
        select = match filter.review_requested {
            Some(true) => select.filter(Column::ReviewRequestedAt.is_not_null()),
            Some(false) => select.filter(Column::ReviewRequestedAt.is_null()),
            None => select,
        };
        if let Some(scheduled) = filter.scheduled {
            let waiting = sea_orm::sea_query::Query::select()
                .column(crate::entity::schedule::Column::EntryId)
                .from(crate::entity::schedule::Entity)
                .and_where(crate::entity::schedule::Column::TenantId.eq(tenant.id().as_uuid()))
                .and_where(crate::entity::schedule::Column::Status.eq("scheduled"))
                .to_owned();
            select = if scheduled {
                select.filter(Column::Id.in_subquery(waiting))
            } else {
                select.filter(Column::Id.not_in_subquery(waiting))
            };
        }
        if let Some(creator) = filter.created_by {
            select = select.filter(Column::CreatedBy.eq(creator));
        }
        if let Some(ai) = filter.ai_generated {
            select = select.filter(Column::AiGenerated.eq(ai));
        }
        for (key, value) in &filter.custom {
            let condition = self
                .custom_fields
                .condition(tenant, "content", crate::resource::ENTRY, Column::Id, key, value)
                .await?;
            select = select.filter(condition);
        }
        if !filter.tags_any.is_empty() || !filter.tags_all.is_empty() {
            select = select.filter(autoxide_kernel::tags::TagIndex.condition(
                tenant,
                crate::resource::ENTRY,
                Column::Id,
                &filter.tags_any,
                &filter.tags_all,
            ));
        }
        if let Some(missing) = &filter.missing_locale {
            // Nhóm bản dịch đã có mục bằng ngôn ngữ đó thì loại; mục chính nó bằng ngôn ngữ đó
            // cũng bị loại theo (cùng nhóm).
            let translated = sea_orm::sea_query::Query::select()
                .column(Column::TranslationGroupId)
                .from(Entity)
                .and_where(Column::TenantId.eq(tenant.id().as_uuid()))
                .and_where(Column::Locale.eq(missing.as_str()))
                .to_owned();
            select = select.filter(Column::TranslationGroupId.not_in_subquery(translated));
        }
        let read = tenant.database().read();
        match filter.sort {
            EntrySort::CreatedAt => fetch_page(select, Column::Id, |row| row.id, page, read).await,
            EntrySort::UpdatedAt => {
                let key = |row: &Entry| SortValue::Time(row.updated_at);
                fetch_page_by(select, Column::UpdatedAt, key, Column::Id, |row| row.id, page, read)
                    .await
            }
        }
    }

    /// Đường dẫn và lúc đăng của bản công khai của từng mục trong `ids` (một câu cho cả trang).
    pub(crate) async fn published_paths(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, (String, Timestamp)>, Error> {
        if ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        use crate::entity::published;
        let rows: Vec<(Uuid, String, Timestamp)> = published::Entity::find()
            .scoped(tenant)
            .select_only()
            .column(published::Column::EntryId)
            .column(published::Column::Path)
            .column(published::Column::PublishedAt)
            .filter(published::Column::EntryId.is_in(ids.iter().copied()))
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        Ok(rows.into_iter().map(|(id, path, at)| (id, (path, at))).collect())
    }
}
