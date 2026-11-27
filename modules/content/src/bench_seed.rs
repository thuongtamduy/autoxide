//! `autoxide task content.bench_seed` (bước 39 B1): sinh bài viết đã đăng, nhiều ngôn ngữ, có
//! danh mục, thẻ, tác giả và ảnh đại diện, cho bài đo đường đọc công khai trên dữ liệu cỡ thật.
//!
//! Chỉ biên dịch khi bật cargo feature `bench-seed` (và trong test). Danh mục, thẻ, tác giả tạo
//! qua service (ít, đi đúng đường thật). Bài viết thì ghi thẳng bảng của module theo lô, mỗi dòng
//! dựng bằng chính các hàm của đường tạo / đăng (`blank`, `body::check`, `Snapshot`, đường dẫn
//! `/<tiền tố>/<slug>`) để giống hệt bài đăng qua API: dòng nháp đã đăng, một revision `published`,
//! dòng `content_published`, term và tác giả. Không phát event, không xếp job lập index: dữ liệu
//! giả không cần audit; bài đo tìm theo ý nghĩa tự lập index khi cần.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::bench_seed::{self, Pseudo, SeedArgs};
use autoxide_kernel::error::Error;
use autoxide_kernel::task::{Task, TaskArgs, TaskRun};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{EntityTrait, IntoActiveModel, PaginatorTrait, Set};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{entry, entry_author, entry_term, published, revision};
use crate::entries::revisions::{Reason, Snapshot};
use crate::entries::{Entry, Kind, Status, blank, body};
use crate::service::ContentService;
use crate::taxonomy::authors::AuthorInput;
use crate::taxonomy::terms::{Taxonomy, TermInput};

/// Khoá trong file ID.
pub(crate) const IDS_KEY: &str = bench_seed::ENTRIES_KEY;

/// Số danh mục, thẻ, tác giả (cố định: bài đo lọc theo danh mục cần mỗi danh mục nhiều bài).
const CATEGORIES: usize = 20;
const TAGS: usize = 200;
const AUTHORS: usize = 50;
/// Số đoạn văn mỗi bài (mỗi đoạn ~40 từ): bài dài cỡ thật, ~500 từ.
const PARAGRAPHS: usize = 12;

pub(crate) fn task(service: Arc<ContentService>) -> Task {
    bench_seed::arguments().into_iter().fold(
        Task::new(
            "bench_seed",
            "Make `count` made-up published posts (default 100000) for load tests: every other \
             post also has a translation in the second language of AUTOXIDE_CONTENT_LOCALES, and \
             each has a category, tags, an author and a featured image from the `media` ids of the \
             `ids` file (when present). Writes the ids of the posts to the `ids` file. Refuses to \
             run when the customer already has entries.",
            Arc::new(Seed { service }),
        ),
        Task::argument,
    )
}

struct Seed {
    service: Arc<ContentService>,
}

/// Danh mục, thẻ, tác giả, ảnh để gắn vào bài.
struct Pool {
    categories: Vec<Uuid>,
    tags: Vec<Uuid>,
    authors: Vec<Uuid>,
    media: Vec<Uuid>,
    users: Vec<Uuid>,
}

/// Các dòng của một lô bài.
#[derive(Default)]
struct Batch {
    entries: Vec<entry::ActiveModel>,
    revisions: Vec<revision::ActiveModel>,
    published: Vec<published::ActiveModel>,
    terms: Vec<entry_term::ActiveModel>,
    authors: Vec<entry_author::ActiveModel>,
}

#[async_trait::async_trait]
impl TaskRun for Seed {
    async fn run(&self, args: &TaskArgs) -> Result<String, Error> {
        let args = SeedArgs::parse(args, 100_000)?;
        let tenant = args.tenant(&self.service.databases).await?;
        let existing =
            entry::Entity::find().scoped(&tenant).count(tenant.database().read()).await?;
        if existing > 0 {
            return Err(bench_seed::refused(&format!(
                "the customer already has {existing} entries; seed only a fresh database"
            )));
        }
        // Ảnh và người dùng là tuỳ chọn: thiếu thì bài không có ảnh / người tạo.
        let media = bench_seed::read_ids(&args.ids, bench_seed::MEDIA_KEY).unwrap_or_default();
        let users = bench_seed::read_ids(&args.ids, bench_seed::USERS_KEY).unwrap_or_default();
        let languages = self.service.languages(&tenant).await?;
        let principal = Principal::with_all_permissions(users.first().copied().unwrap_or_default())
            .in_tenant(tenant.id());
        let pool = Pool {
            categories: self.terms(&tenant, &principal, Taxonomy::Category, CATEGORIES).await?,
            tags: self.terms(&tenant, &principal, Taxonomy::Tag, TAGS).await?,
            authors: self.authors(&tenant, &principal).await?,
            media,
            users,
        };
        let second = languages.locales.iter().find(|locale| **locale != languages.default).cloned();
        let ids =
            self.posts(&tenant, &pool, &languages.default, second.as_deref(), args.count).await?;
        bench_seed::write_ids(&args.ids, IDS_KEY, &ids)?;
        Ok(format!(
            "Made {} published entries ({CATEGORIES} categories, {TAGS} tags, {AUTHORS} authors); \
             ids written to `{}`.",
            ids.len(),
            args.ids.display()
        ))
    }
}

impl Seed {
    async fn terms(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        taxonomy: Taxonomy,
        count: usize,
    ) -> Result<Vec<Uuid>, Error> {
        let mut ids = Vec::with_capacity(count);
        for index in 0..count {
            let input = TermInput {
                name: Some(format!("{} {index}", taxonomy.as_str())),
                ..TermInput::default()
            };
            ids.push(self.service.create_term(tenant, principal, taxonomy, input).await?.term.id);
        }
        Ok(ids)
    }

    async fn authors(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<Vec<Uuid>, Error> {
        let mut random = Pseudo::new(0xA07);
        let mut ids = Vec::with_capacity(AUTHORS);
        for index in 0..AUTHORS {
            let input = AuthorInput {
                display_name: Some(format!("{} {index}", random.sentence(2))),
                bio: Some(random.sentence(20)),
                ..AuthorInput::default()
            };
            ids.push(self.service.create_author(tenant, principal, input).await?.id);
        }
        Ok(ids)
    }

    /// Ghi `count` bài theo lô; bài chẵn có thêm bản dịch sang `second` (nếu có, tính trong
    /// `count`). Trả ID mọi bài.
    async fn posts(
        &self,
        tenant: &TenantContext,
        pool: &Pool,
        default: &str,
        second: Option<&str>,
        count: u64,
    ) -> Result<Vec<Uuid>, Error> {
        let mut random = Pseudo::new(0xC0DE);
        let now = Timestamp::now();
        let mut batch = Batch::default();
        let mut ids = Vec::new();
        let mut group = None;
        let mut index = 0;
        while index < count {
            // Bài kế là bản dịch của bài trước khi bài trước là bài chẵn (và có ngôn ngữ thứ hai).
            let (locale, translation_of) = match (group.take(), second) {
                (Some(source), Some(second)) => (second, Some(source)),
                _ => (default, None),
            };
            let row = self.post(tenant, pool, &mut random, index, locale, translation_of, now)?;
            if translation_of.is_none() && index.is_multiple_of(2) {
                group = Some(row.translation_group_id);
            }
            ids.push(row.id);
            self.add(tenant, pool, &mut random, &mut batch, row)?;
            index += 1;
            if batch.entries.len() >= bench_seed::ROWS_PER_TRANSACTION {
                flush(tenant, std::mem::take(&mut batch)).await?;
            }
        }
        flush(tenant, batch).await?;
        Ok(ids)
    }

    /// Dòng nháp đã đăng của bài thứ `index`.
    #[allow(clippy::too_many_arguments)]
    fn post(
        &self,
        tenant: &TenantContext,
        pool: &Pool,
        random: &mut Pseudo,
        index: u64,
        locale: &str,
        translation_of: Option<Uuid>,
        now: Timestamp,
    ) -> Result<Entry, Error> {
        let id = Uuid::now_v7();
        let at =
            now - SignedDuration::from_mins(i64::try_from(random.below(1_576_800)).unwrap_or(0));
        let by = random.pick(&pool.users).copied().unwrap_or_default();
        let mut row = blank(tenant, id, Kind::Post, locale, None, by, at);
        if let Some(group) = translation_of {
            row.translation_group_id = group;
        }
        row.title = format!("{} {index}", random.sentence(6));
        row.slug = format!(
            "{}-{index}",
            autoxide_kernel::slug::from_title(&random.sentence(4), id)
                .chars()
                .take(80)
                .collect::<String>()
                .trim_end_matches('-')
        );
        row.excerpt = random.sentence(25);
        let paragraphs: Vec<Value> = (0..PARAGRAPHS)
            .map(|_| json!({"type": "paragraph", "content": [{"type": "text", "text": random.sentence(40)}]}))
            .collect();
        let content = json!({"type": "doc", "content": paragraphs});
        let config = &self.service.config;
        let info = body::check(&content, config.max_body_bytes, &config.embed_hosts)
            .map_err(|_| Error::internal("bench seed made an invalid body"))?;
        row.body = content.to_string();
        row.word_count = info.word_count;
        row.reading_time_minutes = body::reading_minutes(info.word_count, config.words_per_minute);
        row.search_key = autoxide_kernel::search::fold(&format!(
            "{}\n{}\n{}",
            row.title, row.excerpt, info.plain_text
        ));
        row.plain_text = info.plain_text;
        row.featured_media_id = random.pick(&pool.media).copied();
        row.status = Status::Published.as_str().to_owned();
        Ok(row)
    }

    /// Thêm bài `row` (cùng revision, bản công khai, term, tác giả) vào lô.
    fn add(
        &self,
        tenant: &TenantContext,
        pool: &Pool,
        random: &mut Pseudo,
        batch: &mut Batch,
        row: Entry,
    ) -> Result<(), Error> {
        let tenant_id = tenant.id().as_uuid();
        let revision_id = Uuid::now_v7();
        let at = row.created_at;
        batch.revisions.push(revision::ActiveModel {
            id: Set(revision_id),
            tenant_id: Set(tenant_id),
            entry_id: Set(row.id),
            snapshot: Set(serde_json::to_string(&Snapshot::of(&row)).map_err(Error::internal)?),
            reason: Set(Reason::Published.as_str().to_owned()),
            created_by: Set(row.created_by),
            created_at: Set(at),
        });
        let path = format!("/{}/{}", self.service.config.post_path_prefix, row.slug);
        batch.published.push(
            published::Model {
                entry_id: row.id,
                tenant_id,
                org_id: row.org_id,
                kind: row.kind.clone(),
                type_id: None,
                locale: row.locale.clone(),
                translation_group_id: row.translation_group_id,
                parent_id: None,
                revision_id,
                path,
                title: row.title.clone(),
                slug: row.slug.clone(),
                excerpt: row.excerpt.clone(),
                body: row.body.clone(),
                body_schema_version: row.body_schema_version,
                plain_text: row.plain_text.clone(),
                search_key: row.search_key.clone(),
                search_words: crate::publishing::words::words(tenant_id, &row.search_key),
                word_count: row.word_count,
                reading_time_minutes: row.reading_time_minutes,
                featured_media_id: row.featured_media_id,
                featured_media_alt: String::new(),
                meta_title: String::new(),
                meta_description: String::new(),
                og_media_id: None,
                canonical_url: String::new(),
                noindex: false,
                ai_generated: false,
                ai_assisted: false,
                restricted_at: None,
                ai_reviewed_by: None,
                ai_reviewed_at: None,
                published_at: at,
                first_published_at: at,
                content_modified_at: at,
                updated_at: at,
                custom_fields: None,
            }
            .into_active_model(),
        );
        // Một danh mục, ba thẻ (khác nhau), một tác giả.
        let mut terms = vec![random.pick(&pool.categories).copied()];
        while terms.len() < 4 {
            let tag = random.pick(&pool.tags).copied();
            if !terms.contains(&tag) {
                terms.push(tag);
            }
        }
        for (order, term) in terms.into_iter().flatten().enumerate() {
            batch.terms.push(entry_term::ActiveModel {
                tenant_id: Set(tenant_id),
                entry_id: Set(row.id),
                term_id: Set(term),
                sort_order: Set(i32::try_from(order).map_err(Error::internal)?),
            });
        }
        if let Some(author) = random.pick(&pool.authors) {
            batch.authors.push(entry_author::ActiveModel {
                tenant_id: Set(tenant_id),
                entry_id: Set(row.id),
                author_id: Set(*author),
                sort_order: Set(0),
            });
        }
        batch.entries.push(row.into_active_model());
        Ok(())
    }
}

/// Ghi một lô: bài trước, rồi các bảng trỏ tới bài.
async fn flush(tenant: &TenantContext, batch: Batch) -> Result<(), Error> {
    let database = tenant.database();
    bench_seed::insert_all(database, batch.entries).await?;
    bench_seed::insert_all(database, batch.revisions).await?;
    bench_seed::insert_all(database, batch.published).await?;
    bench_seed::insert_all(database, batch.terms).await?;
    bench_seed::insert_all(database, batch.authors).await
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::bench_seed::CONFIRM;
    use autoxide_kernel::pagination::PageRequest;
    use autoxide_testkit::TestDatabase;

    use super::*;
    use crate::testing::{config, service};

    #[tokio::test]
    async fn sinh_bai_da_dang_nhieu_ngon_ngu_doc_cong_khai_duoc() {
        TestDatabase::run(async |test| {
            let (service, media, database) =
                service(test, config(&[("AUTOXIDE_CONTENT_LOCALES", "en,vi")])).await;
            let tenant = TenantContext::default_on(database.clone());
            let ids = std::env::temp_dir().join(format!("autoxide-bench-{}.json", Uuid::now_v7()));
            let images: Vec<Uuid> = (0..5).map(|_| media.add(true)).collect();
            bench_seed::write_ids(&ids, bench_seed::MEDIA_KEY, &images).unwrap();
            let task = task(service.clone());
            let raw = [
                format!("confirm={CONFIRM}"),
                "count=1201".to_owned(),
                format!("ids={}", ids.display()),
            ];
            let report = task.run(&task.parse(&raw).unwrap()).await.unwrap();
            assert!(report.starts_with("Made 1201 published entries"), "{report}");
            let made = bench_seed::read_ids(&ids, IDS_KEY).unwrap();
            assert_eq!(made.len(), 1201);
            let read = database.read();
            assert_eq!(published::Entity::find().count(read).await.unwrap(), 1201);
            // Bài chẵn có bản dịch: 601 bài `en`, 600 bài `vi`.
            let first = published::Entity::find_by_id((tenant.id().as_uuid(), made[0]));
            let first = first.one(read).await.unwrap().unwrap();
            let second = published::Entity::find_by_id((tenant.id().as_uuid(), made[1]));
            let second = second.one(read).await.unwrap().unwrap();
            assert_eq!((first.locale.as_str(), second.locale.as_str()), ("en", "vi"));
            assert_eq!(first.translation_group_id, second.translation_group_id);
            // Đọc công khai đi qua đúng đường thật: danh sách và một bài theo đường dẫn.
            let filter = crate::publishing::reading::PostFilter {
                locale: "en".to_owned(),
                site: vec![autoxide_kernel::tenancy::root_of(tenant.id()).as_uuid()],
                term_id: None,
                author_id: None,
                search: None,
            };
            let page = service.public_posts(&tenant, &filter, &PageRequest::default()).await;
            assert_eq!(
                page.unwrap().data.len(),
                usize::try_from(PageRequest::default().limit()).unwrap()
            );
            assert!(images.contains(&first.featured_media_id.unwrap()));
            // Lần hai: đã có bài → từ chối.
            let error = task.run(&task.parse(&raw).unwrap()).await.unwrap_err();
            assert_eq!(error.code(), "bench_seed_refused");
            std::fs::remove_file(ids).unwrap();
        })
        .await
        .unwrap();
    }
}
