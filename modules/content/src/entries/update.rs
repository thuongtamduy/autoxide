//! Sửa bài: update, update_inner và apply (áp thay đổi lên bản ghi).

use super::*;
use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;

impl ContentService {
    /// Sửa bản nháp của mục `id`. Cần `content.write`. Mục đang công khai thì bật
    /// `has_unpublished_changes`; bản công khai không đổi tới khi đăng lại.
    pub(crate) async fn update(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        fields: EntryFields,
        save_revision: bool,
        expected: Option<Timestamp>,
    ) -> Result<Entry, Error> {
        let revise =
            Revise { forced: save_revision.then_some(Reason::ExplicitSave), restored_from: None };
        self.update_inner(tenant, principal, id, fields, revise, expected).await
    }

    /// Sửa bản nháp; `revise`: revision bắt buộc ghi và có phải khôi phục không.
    pub(crate) async fn update_inner(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        fields: EntryFields,
        revise: Revise,
        expected: Option<Timestamp>,
    ) -> Result<Entry, Error> {
        let Revise { forced, restored_from } = revise;
        permissions::require_create(principal)?;
        let before = Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| entry_not_found(id))?;
        permissions::require_write(principal, &before)?;
        permissions::require_org(principal, tenant, &before, permissions::Need::Write).await?;
        if expected.is_some_and(|expected| expected != before.updated_at) {
            return Err(autoxide_kernel::version::changed());
        }
        let mut problems = Vec::new();
        let mut candidate = before.clone();
        let slug_changed = fields.slug.as_ref().is_some_and(|slug| slug.trim() != before.slug);
        // Bỏ nhãn "viết từ gợi ý của AI" là quyết định biên tập: chỉ người được đăng (D-362).
        if before.ai_assisted && fields.ai_assisted == Some(false) {
            principal.require(&permissions::CONTENT_PUBLISH).map_err(|_| ai_label_locked())?;
        }
        let mut fields = fields;
        if let Some(custom) = fields.custom_fields.take() {
            let current = stored_custom(&before);
            candidate.custom_fields = self
                .custom_values(tenant, Some(&custom), current.as_ref(), false, &mut problems)
                .await?;
        }
        self.apply(tenant, &mut candidate, fields, &mut problems).await?;
        mark_agent_written(principal, &mut candidate);
        if !problems.is_empty() {
            return Err(invalid(problems));
        }
        let now = now();
        candidate.updated_at = now;
        if Status::of(&before) == Status::Published {
            candidate.has_unpublished_changes = true;
        }
        let reason = if forced.is_some() {
            forced
        } else if now - before.revised_at > SignedDuration::from_mins(BACKGROUND_SAVE_MINUTES) {
            Some(Reason::BackgroundSave)
        } else {
            None
        };
        if reason.is_some() {
            candidate.revised_at = now;
        }

        let transaction = tenant.database().begin_write().await?;
        if slug_changed && slug_taken(&transaction, tenant, &candidate).await? {
            return Err(slug_conflict(&candidate.slug));
        }
        let mut update =
            Entity::update(changed_columns(&before, &candidate)).validate()?.scoped(tenant);
        // Phiên bản đã đọc (`If-Match`), hay ít nhất "chưa ai sửa từ lúc đọc ở trên".
        update = update.filter(Column::UpdatedAt.eq(expected.unwrap_or(before.updated_at)));
        let after = match update.exec(&transaction).await {
            Ok(after) => after,
            Err(sea_orm::DbErr::RecordNotUpdated) => {
                return Err(autoxide_kernel::version::changed());
            }
            Err(error) if is_unique_violation(&error) => {
                return Err(slug_conflict(&candidate.slug));
            }
            Err(error) => return Err(error.into()),
        };
        if after.custom_fields != before.custom_fields {
            self.custom_fields
                .write_index(
                    &transaction,
                    tenant,
                    "content",
                    crate::resource::ENTRY,
                    after.id,
                    &stored_custom(&after).unwrap_or_default(),
                )
                .await?;
        }
        if let Some(reason) = reason {
            revisions::record(&transaction, tenant, &after, reason, Some(principal.user_id()), now)
                .await?;
        }
        let mut after_summary = summary(&after);
        if let (Some(revision), Some(object)) = (restored_from, after_summary.as_object_mut()) {
            object.insert("restored_from".to_owned(), json!(revision));
        }
        let change =
            Change { before: Some(summary(&before)), after: Some(after_summary), published: None };
        self.enqueue_index(tenant, &transaction, id).await?;
        let actor = Actor::from(principal);
        let subject = events::entry_subject(id);
        if restored_from.is_some() {
            self.outbox
                .record(&transaction, tenant, actor, subject, &events::EntryRestored(change))
                .await?;
        } else {
            self.outbox
                .record(&transaction, tenant, actor, subject, &events::EntryUpdated(change))
                .await?;
        }
        transaction.commit().await?;
        Ok(after)
    }

    /// Kiểm giá trị trường tự định nghĩa (bước 51 P19) theo định nghĩa của khách; lỗi vào
    /// `problems` (đường dẫn `custom_fields.<key>`). Trả JSON để lưu; `None` khi không có gì.
    pub(crate) async fn custom_values(
        &self,
        tenant: &TenantContext,
        incoming: Option<&serde_json::Map<String, Value>>,
        current: Option<&serde_json::Map<String, Value>>,
        creating: bool,
        problems: &mut Vec<FieldError>,
    ) -> Result<Option<String>, Error> {
        match self
            .custom_fields
            .validate(tenant, crate::resource::ENTRY, incoming, current, creating)
            .await
        {
            Ok(values) if values.is_empty() => Ok(None),
            Ok(values) => Ok(Some(Value::Object(values).to_string())),
            Err(error) if error.code() == "kernel.custom_field_invalid" => {
                problems.extend(error.fields().iter().cloned());
                Ok(current.map(|current| Value::Object(current.clone()).to_string()))
            }
            Err(error) => Err(error),
        }
    }

    /// Áp `fields` lên `candidate` rồi kiểm toàn bộ (mọi lỗi vào `problems`); tính văn bản
    /// thuần, số chữ, thời gian đọc, khoá tìm; kiểm ảnh có tồn tại (qua `media`).
    pub(crate) async fn apply(
        &self,
        tenant: &TenantContext,
        candidate: &mut Entry,
        fields: EntryFields,
        problems: &mut Vec<FieldError>,
    ) -> Result<(), Error> {
        let trim = |value: Option<String>| value.map(|value| value.trim().to_owned());
        if let Some(title) = trim(fields.title) {
            candidate.title = title;
        }
        if (candidate.title.is_empty() || candidate.title.chars().count() > TITLE_MAX)
            && !problems.iter().any(|p| p.field == "title")
        {
            problems.push(
                FieldError::new("title", "length", "must be 1 to {title_max} characters")
                    .with_param("title_max", TITLE_MAX),
            );
        }
        match trim(fields.slug) {
            Some(slug) if slug.is_empty() => {
                candidate.slug = autoxide_kernel::slug::from_title(&candidate.title, candidate.id);
            }
            Some(slug) => {
                if !autoxide_kernel::slug::is_valid(&slug) {
                    problems.push(FieldError::new(
                        "slug",
                        "invalid",
                        "must be lower-case letters and digits in groups joined by `-`, at most 100 characters",
                    ));
                }
                candidate.slug = slug;
            }
            None if candidate.slug.is_empty() => {
                candidate.slug = autoxide_kernel::slug::from_title(&candidate.title, candidate.id);
            }
            None => {}
        }
        for (field, value, target, max) in [
            ("excerpt", fields.excerpt, &mut candidate.excerpt, EXCERPT_MAX),
            (
                "featured_media_alt",
                fields.featured_media_alt,
                &mut candidate.featured_media_alt,
                ALT_MAX,
            ),
            ("meta_title", fields.meta_title, &mut candidate.meta_title, META_TITLE_MAX),
            (
                "meta_description",
                fields.meta_description,
                &mut candidate.meta_description,
                META_DESCRIPTION_MAX,
            ),
        ] {
            check_length(field, value.as_deref(), max, problems);
            if let Some(value) = value {
                *target = value.trim().to_owned();
            }
        }
        check_url("canonical_url", fields.canonical_url.as_deref(), problems);
        if let Some(url) = fields.canonical_url {
            candidate.canonical_url = url.trim().to_owned();
        }
        if let Some(noindex) = fields.noindex {
            candidate.noindex = noindex;
        }
        if let Some(ai_generated) = fields.ai_generated {
            candidate.ai_generated = ai_generated;
        }
        if let Some(ai_assisted) = fields.ai_assisted {
            candidate.ai_assisted = ai_assisted;
        }
        if let Some(revision) = fields.translated_from {
            candidate.translated_from_revision_id = Some(revision);
        }
        if let Some(featured) = fields.featured_media_id {
            candidate.featured_media_id = featured;
        }
        if let Some(og) = fields.og_media_id {
            candidate.og_media_id = og;
        }
        let body = match fields.body {
            Some(body) => body,
            None => serde_json::from_str(&candidate.body).unwrap_or_else(|_| body::empty()),
        };
        let info = match body::check(&body, self.config.max_body_bytes, &self.config.embed_hosts) {
            Ok(info) => info,
            Err(errors) => {
                problems.extend(errors);
                body::BodyInfo::default()
            }
        };
        candidate.body = serde_json::to_string(&body).map_err(Error::internal)?;
        candidate.body_schema_version = body::SCHEMA_VERSION;
        candidate.word_count = info.word_count;
        candidate.reading_time_minutes =
            body::reading_minutes(info.word_count, self.config.words_per_minute);
        candidate.search_key = autoxide_kernel::search::fold(&format!(
            "{}\n{}\n{}",
            candidate.title, candidate.excerpt, info.plain_text
        ));
        candidate.plain_text = info.plain_text;
        self.check_media(tenant, candidate, &info.media_ids, false, problems).await?;
        self.check_records(tenant, &info.records, problems).await
    }
}
