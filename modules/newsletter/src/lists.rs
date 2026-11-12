//! Danh sách bản tin: tạo, đọc, liệt kê (kèm số người theo trạng thái), sửa, xoá.

use std::collections::HashMap;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use chrono::{DateTime, Utc};
use sea_orm::sea_query::{Expr, ExprTrait as _};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, PaginatorTrait,
    QueryFilter, QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::campaigns::CampaignStatus;
use crate::entity::{campaign, list, subscription};
use crate::events::{self, Change};
use crate::permissions::{NEWSLETTER_MANAGE, NEWSLETTER_READ};
use crate::service::{
    NewsletterService, from_json, is_unique_violation, list_not_found, now, to_json,
};
use crate::text::{Locale, Localized, is_valid_key};

const NAME_MAX: usize = 200;
const DESCRIPTION_MAX: usize = 1000;

/// What happens when a post is published (by the `content` module).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum NewPosts {
    /// Nothing (the default).
    #[default]
    Off,
    /// A draft campaign is created for each new post, to review and send by hand.
    Draft,
    /// A campaign is created and sent at once.
    Send,
}

impl NewPosts {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            NewPosts::Off => "off",
            NewPosts::Draft => "draft",
            NewPosts::Send => "send",
        }
    }

    fn stored(raw: &str) -> NewPosts {
        match raw {
            "draft" => NewPosts::Draft,
            "send" => NewPosts::Send,
            _ => NewPosts::Off,
        }
    }
}

/// Một danh sách, cột JSON đã đọc ra.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct List {
    pub row: list::Model,
    pub name: Localized,
    pub description: Localized,
}

impl List {
    pub(crate) fn parse(row: list::Model) -> Result<List, Error> {
        Ok(List { name: from_json(&row.name)?, description: from_json(&row.description)?, row })
    }

    pub(crate) fn new_posts(&self) -> NewPosts {
        NewPosts::stored(&self.row.new_posts)
    }

    pub(crate) fn new_post_locale(&self) -> Option<Locale> {
        self.row.new_post_locale.as_deref().and_then(Locale::parse)
    }

    /// Tóm tắt cho event.
    fn summary(&self) -> Value {
        json!({
            "id": self.row.id,
            "key": self.row.key,
            "public": self.row.public,
            "double_opt_in": self.row.double_opt_in,
            "new_posts": self.row.new_posts,
        })
    }
}

/// Subscriptions by status.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub(crate) struct SubscriptionCounts {
    pub unconfirmed: u64,
    pub confirmed: u64,
    pub unsubscribed: u64,
}

/// Các trường sửa được; `None` giữ nguyên (khi tạo: giá trị mặc định).
#[derive(Debug, Clone, Default)]
pub(crate) struct ListInput {
    pub key: Option<String>,
    pub name: Option<Localized>,
    pub description: Option<Localized>,
    pub double_opt_in: Option<bool>,
    pub public: Option<bool>,
    pub new_posts: Option<NewPosts>,
    /// `Some(None)`: mọi ngôn ngữ.
    pub new_post_locale: Option<Option<Locale>>,
}

impl NewsletterService {
    /// Tạo danh sách. Cần `newsletter.manage`.
    pub(crate) async fn create_list(
        &self,
        principal: &Principal,
        input: ListInput,
    ) -> Result<List, Error> {
        principal.require(&NEWSLETTER_MANAGE)?;
        let mut problems = Vec::new();
        for (name, missing) in [("key", input.key.is_none()), ("name", input.name.is_none())] {
            if missing {
                problems.push(FieldError::new(name, "required", "is required"));
            }
        }
        let now = now();
        let blank = list::Model {
            id: Uuid::now_v7(),
            key: String::new(),
            name: "{}".to_owned(),
            description: "{}".to_owned(),
            double_opt_in: true,
            public: false,
            new_posts: NewPosts::Off.as_str().to_owned(),
            new_post_locale: None,
            created_by: Some(principal.user_id()),
            created_at: now,
            updated_at: now,
        };
        let mut candidate = List::parse(blank)?;
        apply(&mut candidate, input, &mut problems)?;
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let transaction = self.database.begin_write().await?;
        if key_taken(&transaction, &candidate.row.key, None).await? {
            return Err(key_conflict(&candidate.row.key));
        }
        let row = match candidate.row.clone().into_active_model().insert(&transaction).await {
            Ok(row) => row,
            Err(error) if is_unique_violation(&error) => {
                return Err(key_conflict(&candidate.row.key));
            }
            Err(error) => return Err(error.into()),
        };
        let created = List::parse(row)?;
        let event = events::ListCreated(Change { before: None, after: Some(created.summary()) });
        self.outbox
            .record(
                &transaction,
                Actor::from(principal),
                events::list_subject(created.row.id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(list_id = %created.row.id, "newsletter list created");
        Ok(created)
    }

    /// Danh sách `id`. Cần `newsletter.read`.
    pub(crate) async fn get_list(&self, principal: &Principal, id: Uuid) -> Result<List, Error> {
        principal.require(&NEWSLETTER_READ)?;
        self.find_list(id).await
    }

    pub(crate) async fn find_list(&self, id: Uuid) -> Result<List, Error> {
        let row = list::Entity::find_by_id(id)
            .one(self.database.read())
            .await?
            .ok_or_else(list_not_found)?;
        List::parse(row)
    }

    /// Mọi danh sách có id trong `ids` (đọc cùng lúc), theo id.
    pub(crate) async fn lists_by_id<C: ConnectionTrait>(
        &self,
        connection: &C,
        ids: impl IntoIterator<Item = Uuid>,
    ) -> Result<HashMap<Uuid, List>, Error> {
        let ids: Vec<Uuid> = ids.into_iter().collect();
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows = list::Entity::find().filter(list::Column::Id.is_in(ids)).all(connection).await?;
        rows.into_iter().map(|row| List::parse(row).map(|list| (list.row.id, list))).collect()
    }

    /// Một trang danh sách, mới trước (mặc định). Cần `newsletter.read`.
    pub(crate) async fn list_lists(
        &self,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<List>, Error> {
        principal.require(&NEWSLETTER_READ)?;
        let found = fetch_page(
            list::Entity::find(),
            list::Column::Id,
            |row| row.id,
            page,
            self.database.read(),
        )
        .await?;
        let Page { data, pagination } = found;
        let data = data.into_iter().map(List::parse).collect::<Result<Vec<_>, _>>()?;
        Ok(Page { data, pagination })
    }

    /// Mọi danh sách công khai, theo `key` (cho website).
    pub(crate) async fn public_lists(&self) -> Result<Vec<List>, Error> {
        let rows = list::Entity::find()
            .filter(list::Column::Public.eq(true))
            .all(self.database.read())
            .await?;
        let mut lists = rows.into_iter().map(List::parse).collect::<Result<Vec<_>, _>>()?;
        lists.sort_by(|a, b| a.row.key.cmp(&b.row.key));
        Ok(lists)
    }

    /// Số đăng ký theo trạng thái của từng danh sách trong `ids`.
    pub(crate) async fn subscription_counts(
        &self,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, SubscriptionCounts>, Error> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows: Vec<(Uuid, String, i64)> = subscription::Entity::find()
            .select_only()
            .column(subscription::Column::ListId)
            .column(subscription::Column::Status)
            .column_as(Expr::col(subscription::Column::SubscriberId).count(), "count")
            .filter(subscription::Column::ListId.is_in(ids.to_vec()))
            .group_by(subscription::Column::ListId)
            .group_by(subscription::Column::Status)
            .into_tuple()
            .all(self.database.read())
            .await?;
        let mut counts: HashMap<Uuid, SubscriptionCounts> = HashMap::new();
        for (list_id, status, count) in rows {
            let entry = counts.entry(list_id).or_default();
            let count = u64::try_from(count).unwrap_or_default();
            match status.as_str() {
                "unconfirmed" => entry.unconfirmed = count,
                "confirmed" => entry.confirmed = count,
                _ => entry.unsubscribed = count,
            }
        }
        Ok(counts)
    }

    /// Sửa danh sách `id`. Cần `newsletter.manage`.
    pub(crate) async fn update_list(
        &self,
        principal: &Principal,
        id: Uuid,
        input: ListInput,
        expected: Option<DateTime<Utc>>,
    ) -> Result<List, Error> {
        principal.require(&NEWSLETTER_MANAGE)?;
        let before = self.find_list(id).await?;
        if expected.is_some_and(|expected| expected != before.row.updated_at) {
            return Err(autoxide_kernel::version::changed());
        }
        let mut problems = Vec::new();
        let mut candidate = before.clone();
        apply(&mut candidate, input, &mut problems)?;
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let now = now();
        let transaction = self.database.begin_write().await?;
        if candidate.row.key != before.row.key
            && key_taken(&transaction, &candidate.row.key, Some(id)).await?
        {
            return Err(key_conflict(&candidate.row.key));
        }
        let next = candidate.row.clone();
        let mut row = before.row.clone().into_active_model();
        row.key = Set(next.key);
        row.name = Set(next.name);
        row.description = Set(next.description);
        row.double_opt_in = Set(next.double_opt_in);
        row.public = Set(next.public);
        row.new_posts = Set(next.new_posts);
        row.new_post_locale = Set(next.new_post_locale);
        row.updated_at = Set(now);
        let update = list::Entity::update(row)
            .validate()?
            .filter(list::Column::UpdatedAt.eq(expected.unwrap_or(before.row.updated_at)));
        let after = match update.exec(&transaction).await {
            Ok(after) => after,
            Err(sea_orm::DbErr::RecordNotUpdated) => {
                return Err(autoxide_kernel::version::changed());
            }
            Err(error) if is_unique_violation(&error) => {
                return Err(key_conflict(&candidate.row.key));
            }
            Err(error) => return Err(error.into()),
        };
        let after = List::parse(after)?;
        let event = events::ListUpdated(Change {
            before: Some(before.summary()),
            after: Some(after.summary()),
        });
        self.outbox
            .record(&transaction, Actor::from(principal), events::list_subject(id), &event)
            .await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Xoá danh sách `id` cùng mọi đăng ký của nó. Cần `newsletter.manage`. Còn chiến dịch chưa
    /// xong (nháp, hẹn giờ, đang gửi, tạm dừng) thì từ chối: xoá hay huỷ chúng trước.
    pub(crate) async fn delete_list(
        &self,
        principal: &Principal,
        id: Uuid,
        expected: Option<DateTime<Utc>>,
    ) -> Result<(), Error> {
        principal.require(&NEWSLETTER_MANAGE)?;
        let before = self.find_list(id).await?;
        if expected.is_some_and(|expected| expected != before.row.updated_at) {
            return Err(autoxide_kernel::version::changed());
        }
        let transaction = self.database.begin_write().await?;
        let active = campaign::Entity::find()
            .filter(campaign::Column::ListId.eq(id))
            .filter(
                campaign::Column::Status.is_in(
                    [
                        CampaignStatus::Draft,
                        CampaignStatus::Scheduled,
                        CampaignStatus::Sending,
                        CampaignStatus::Paused,
                    ]
                    .map(CampaignStatus::as_str),
                ),
            )
            .count(&transaction)
            .await?;
        if active > 0 {
            return Err(Error::conflict(
                "newsletter.list_in_use",
                "This list has campaigns that are not finished (draft, scheduled, sending or \
                 paused). Delete or cancel them first.",
            ));
        }
        let deleted = list::Entity::delete_many()
            .filter(list::Column::Id.eq(id))
            .filter(list::Column::UpdatedAt.eq(expected.unwrap_or(before.row.updated_at)))
            .exec(&transaction)
            .await?;
        if deleted.rows_affected == 0 {
            return Err(autoxide_kernel::version::changed());
        }
        let subscriptions = subscription::Entity::delete_many()
            .filter(subscription::Column::ListId.eq(id))
            .exec(&transaction)
            .await?;
        let mut summary = before.summary();
        if let Some(object) = summary.as_object_mut() {
            object.insert("subscriptions_deleted".to_owned(), json!(subscriptions.rows_affected));
        }
        let event = events::ListDeleted(Change { before: Some(summary), after: None });
        self.outbox
            .record(&transaction, Actor::from(principal), events::list_subject(id), &event)
            .await?;
        transaction.commit().await?;
        tracing::info!(list_id = %id, "newsletter list deleted");
        Ok(())
    }
}

/// Áp `input` lên `candidate` rồi kiểm toàn bộ (mọi lỗi vào `problems`).
fn apply(
    candidate: &mut List,
    input: ListInput,
    problems: &mut Vec<FieldError>,
) -> Result<(), Error> {
    if let Some(key) = input.key {
        let key = key.trim().to_owned();
        if !is_valid_key(&key) {
            problems.push(FieldError::new(
                "key",
                "invalid",
                "must be lower-case letters and digits in groups joined by `-`, at most 50 \
                 characters",
            ));
        }
        candidate.row.key = key;
    }
    if let Some(name) = input.name {
        let name = name.normalized("name", NAME_MAX, problems);
        if name.is_empty() {
            problems.push(FieldError::new("name", "required", "needs `en` or `vi`"));
        }
        candidate.name = name;
    }
    if let Some(description) = input.description {
        candidate.description = description.normalized("description", DESCRIPTION_MAX, problems);
    }
    if let Some(double_opt_in) = input.double_opt_in {
        candidate.row.double_opt_in = double_opt_in;
    }
    if let Some(public) = input.public {
        candidate.row.public = public;
    }
    if let Some(new_posts) = input.new_posts {
        candidate.row.new_posts = new_posts.as_str().to_owned();
    }
    if let Some(locale) = input.new_post_locale {
        candidate.row.new_post_locale = locale.map(|locale| locale.as_str().to_owned());
    }
    candidate.row.name = to_json(&candidate.name)?;
    candidate.row.description = to_json(&candidate.description)?;
    Ok(())
}

async fn key_taken<C: ConnectionTrait>(
    connection: &C,
    key: &str,
    except: Option<Uuid>,
) -> Result<bool, Error> {
    let mut select = list::Entity::find().filter(list::Column::Key.eq(key));
    if let Some(id) = except {
        select = select.filter(list::Column::Id.ne(id));
    }
    Ok(select.count(connection).await? > 0)
}

fn key_conflict(key: &str) -> Error {
    Error::conflict(
        "newsletter.list_key_taken",
        "Another list already uses the key `{key}`. Choose another.",
    )
    .with_param("key", key)
}
