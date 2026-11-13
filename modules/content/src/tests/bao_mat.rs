//! Lượt rà soát bảo mật module `content` (sau bước 18): mỗi test giữ một lỗi không quay lại,
//! chạy trên cả SQLite và Postgres.

use autoxide_kernel::access::Principal;
use autoxide_kernel::event::{Delivered, EventHandler};
use chrono::{Duration, Utc};
use sea_orm::EntityTrait;

use super::*;
use crate::publishing::redirects::{RedirectInput, Resolved};
use crate::publishing::schedules::Action;
use crate::taxonomy::authors::AuthorInput;
use crate::taxonomy::menus::ItemInput;
use crate::taxonomy::terms::{Taxonomy, TermInput};

/// Agent dùng API key `key` của người dùng `user_id`, chỉ có quyền `_own`.
fn agent(user_id: Uuid, key: Uuid) -> Principal {
    Principal::new(
        user_id,
        ["read", "write_own", "publish_own"].iter().map(|p| format!("content.{p}")),
    )
    .via_api_key(key)
}

fn bot() -> Principal {
    agent(Uuid::now_v7(), Uuid::now_v7())
}

fn field_codes(error: &autoxide_kernel::error::Error) -> Vec<(String, String)> {
    error.fields().iter().map(|field| (field.field.clone(), field.code.clone())).collect()
}

/// Gỡ hay lưu trữ huỷ lịch đăng đang chờ (lưu trữ huỷ cả lịch gỡ): lịch đặt từ trước không
/// đăng lại mục vừa bị gỡ.
#[tokio::test]
async fn go_va_luu_tru_huy_lich_dang() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let ada = editor();
        let at = Utc::now() + Duration::hours(1);

        let entry = content.create(&ada, post("Taken down")).await.unwrap();
        content.publish(&ada, entry.id, None, None, false).await.unwrap();
        let planned =
            content.create_schedule(&ada, entry.id, Action::Publish, at, None).await.unwrap();
        content.unpublish(&ada, entry.id, None).await.unwrap();
        content.run_schedule(planned.id, Utc::now()).await.unwrap();
        assert_eq!(content.get(&ada, entry.id).await.unwrap().status, "draft");
        assert_eq!(
            content.resolve("en", "/blog/taken-down").await.unwrap_err().code(),
            "content.not_found"
        );
        let rows = content.list_schedules(&ada, entry.id).await.unwrap();
        assert_eq!(rows[0].status, "canceled");

        let archived = content.create(&ada, post("Archived")).await.unwrap();
        content.publish(&ada, archived.id, None, None, false).await.unwrap();
        let up =
            content.create_schedule(&ada, archived.id, Action::Publish, at, None).await.unwrap();
        let down =
            content.create_schedule(&ada, archived.id, Action::Unpublish, at, None).await.unwrap();
        content.archive(&ada, archived.id, None).await.unwrap();
        for id in [up.id, down.id] {
            content.run_schedule(id, Utc::now()).await.unwrap();
        }
        assert_eq!(content.get(&ada, archived.id).await.unwrap().status, "archived");
        assert!(matches!(content.resolve("en", "/blog/archived").await.unwrap(), Resolved::Gone));
        let statuses: Vec<String> = content
            .list_schedules(&ada, archived.id)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.status)
            .collect();
        assert_eq!(statuses, ["canceled", "canceled"]);
        let canceled = events(&database)
            .await
            .iter()
            .filter(|name| *name == "content.schedule_canceled")
            .count();
        assert_eq!(canceled, 3, "one event per canceled schedule");
    })
    .await
    .unwrap();
}

/// Người chỉ có `content.publish_own` không đăng lại mục biên tập viên đã lưu trữ, không tự
/// đặt `published_at`, không đăng đè chuyển hướng làm tay; người có `content.publish` thì
/// được.
#[tokio::test]
async fn publish_own_bi_gioi_han() {
    TestDatabase::run(async |test| {
        let (content, _, _) = service(test, config(&[])).await;
        let ada = editor();
        let bot = bot();
        let mine = content.create(&bot, post("Mine")).await.unwrap();
        content.publish(&bot, mine.id, None, None, false).await.unwrap();
        content.archive(&ada, mine.id, None).await.unwrap();
        let error = content.publish(&bot, mine.id, None, None, false).await.unwrap_err();
        assert_eq!((error.kind(), error.code()), (ErrorKind::Forbidden, "content.archived"));
        content.publish(&ada, mine.id, None, None, false).await.unwrap();

        let dated = content.create(&bot, post("Dated")).await.unwrap();
        let when = Some(Utc::now() + Duration::days(30));
        let error = content.publish(&bot, dated.id, None, when, false).await.unwrap_err();
        assert_eq!(error.code(), "validation_failed");
        assert_eq!(field_codes(&error), [("published_at".into(), "not_allowed".into())]);
        content.publish(&bot, dated.id, None, None, false).await.unwrap();
        let theirs = content.create(&ada, post("Editor dated")).await.unwrap();
        content.publish(&ada, theirs.id, None, when, false).await.unwrap();

        let target = content.create(&ada, post("Target")).await.unwrap();
        let redirect = RedirectInput {
            locale: Some("en".into()),
            from_path: "/blog/campaign".into(),
            to_entry_id: Some(target.id),
            to_url: None,
            status_code: 302,
        };
        content.create_redirect(&ada, redirect).await.unwrap();
        let campaign = content.create(&bot, post("Campaign")).await.unwrap();
        let error = content.publish(&bot, campaign.id, None, None, false).await.unwrap_err();
        assert_eq!(
            (error.kind(), error.code()),
            (ErrorKind::Conflict, "content.path_has_redirect")
        );
        assert_eq!(content.get(&ada, campaign.id).await.unwrap().status, "draft");
        content.publish(&ada, campaign.id, None, None, false).await.unwrap();
    })
    .await
    .unwrap();
}

/// Người chỉ có `content.write_own`: trang cha (khi tạo, chuyển, dịch) phải là của mình;
/// `ai_generated` luôn bật; chỉ thêm hồ sơ tác giả của chính mình.
#[tokio::test]
async fn write_own_bi_gioi_han() {
    TestDatabase::run(async |test| {
        let (content, _, _) = service(test, config(&[])).await;
        let ada = editor();
        let bot = bot();

        // Trang cha.
        let theirs = content.create(&ada, page("Theirs", None)).await.unwrap();
        let error = content.create(&bot, page("Child", Some(theirs.id))).await.unwrap_err();
        assert_eq!((error.kind(), error.code()), (ErrorKind::Forbidden, "content.not_owner"));
        let mine = content.create(&bot, page("Mine", None)).await.unwrap();
        let child = content.create(&bot, page("Child", Some(mine.id))).await.unwrap();
        let error = content.move_page(&bot, child.id, Some(theirs.id), None).await.unwrap_err();
        assert_eq!(error.code(), "content.not_owner");
        content.move_page(&bot, child.id, None, None).await.unwrap();
        content.move_page(&bot, child.id, Some(mine.id), None).await.unwrap();
        // Bản dịch của trang cha do người khác tạo: bản dịch trang con không nằm dưới nó.
        content.create_translation(&ada, mine.id, "vi", fields("Của tôi")).await.unwrap();
        let error =
            content.create_translation(&bot, child.id, "vi", fields("Con")).await.unwrap_err();
        assert_eq!(error.code(), "content.not_owner");
        // Người có `content.write` đặt được dưới trang của người khác.
        content.create(&ada, page("Editor child", Some(mine.id))).await.unwrap();

        // `ai_generated` không tự khai được.
        let mut input = post("Bot post");
        input.fields.ai_generated = Some(false);
        let entry = content.create(&bot, input).await.unwrap();
        assert!(entry.ai_generated);
        let no = EntryFields { ai_generated: Some(false), ..Default::default() };
        let entry = content.update(&bot, entry.id, no.clone(), false, None).await.unwrap();
        assert!(entry.ai_generated);
        let vi = content.create_translation(&bot, entry.id, "vi", fields("Bài")).await;
        assert!(vi.unwrap().ai_generated);
        let entry = content.update(&ada, entry.id, no, false, None).await.unwrap();
        assert!(!entry.ai_generated, "an editor may correct it");
        let human = content.create(&ada, post("Human post")).await.unwrap();
        assert!(!human.ai_generated);

        // Tác giả.
        let profile = |name: &str, user: Option<Uuid>| AuthorInput {
            display_name: Some(name.into()),
            user_id: Some(user),
            ..Default::default()
        };
        let own = content.create_author(&ada, profile("Bot", Some(bot.user_id()))).await;
        let own = own.unwrap();
        let other = content.create_author(&ada, profile("Ada", Some(ada.user_id()))).await;
        let other = other.unwrap();
        let error = content.set_entry_authors(&bot, entry.id, vec![other.id]).await.unwrap_err();
        assert_eq!(field_codes(&error), [("author_ids".into(), "not_allowed".into())]);
        content.set_entry_authors(&bot, entry.id, vec![own.id]).await.unwrap();
        // Hồ sơ biên tập viên đã gắn thì giữ được.
        content.set_entry_authors(&ada, entry.id, vec![own.id, other.id]).await.unwrap();
        let kept = content.set_entry_authors(&bot, entry.id, vec![other.id, own.id]).await;
        assert_eq!(kept.unwrap(), [other.id, own.id]);
    })
    .await
    .unwrap();
}

/// Đích chuyển hướng và link menu dạng `//host` (địa chỉ không scheme của site khác) bị từ
/// chối, như link trong thân bài.
#[tokio::test]
async fn duong_dan_hai_gach_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let (content, _, _) = service(test, config(&[])).await;
        let ada = editor();
        for to in ["//evil.com", "//evil.com/path", "/\\evil.com"] {
            let input = RedirectInput {
                locale: Some("en".into()),
                from_path: "/old".into(),
                to_entry_id: None,
                to_url: Some(to.into()),
                status_code: 301,
            };
            let error = content.create_redirect(&ada, input).await.unwrap_err();
            assert_eq!(field_codes(&error), [("to_url".into(), "invalid".into())], "{to}");
        }
        let fine = RedirectInput {
            locale: Some("en".into()),
            from_path: "/old".into(),
            to_entry_id: None,
            to_url: Some("/new".into()),
            status_code: 301,
        };
        content.create_redirect(&ada, fine).await.unwrap();

        let item = |url: &str| ItemInput {
            label: "Link".into(),
            entry_id: None,
            term_id: None,
            url: Some(url.into()),
            children: vec![],
        };
        let error = content.put_menu(&ada, "main", None, vec![item("//evil.com")], None).await;
        let error = error.unwrap_err();
        assert_eq!(field_codes(&error), [("items[0].url".into(), "invalid".into())]);
        content.put_menu(&ada, "main", None, vec![item("/about")], None).await.unwrap();
    })
    .await
    .unwrap();
}

/// Gắn term / tác giả cùng lúc cho một mục (và cùng lúc với xoá mục): không lỗi 500 vì trùng
/// khoá, không liên kết mồ côi — trên cả hai database.
#[tokio::test]
async fn gan_the_tac_gia_dong_thoi() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let ada = editor();
        let mut tags = Vec::new();
        for name in ["A", "B", "C"] {
            let input = TermInput {
                locale: Some("en".into()),
                name: Some(name.into()),
                ..Default::default()
            };
            tags.push(content.create_term(&ada, Taxonomy::Tag, input).await.unwrap().term.id);
        }
        let mut authors = Vec::new();
        for name in ["X", "Y"] {
            let input = AuthorInput { display_name: Some(name.into()), ..Default::default() };
            authors.push(content.create_author(&ada, input).await.unwrap().id);
        }
        let entry = content.create(&ada, post("Busy")).await.unwrap();
        let sets: Vec<Vec<Uuid>> = vec![
            tags.clone(),
            vec![tags[1], tags[0]],
            vec![tags[2]],
            tags.iter().rev().copied().collect(),
        ];
        // Nhiều vòng, mỗi vòng mọi bộ cùng lúc: đủ để các transaction chồng lên nhau.
        for _ in 0..5 {
            let tasks: Vec<_> = sets
                .iter()
                .chain(sets.iter())
                .cloned()
                .enumerate()
                .map(|(n, set)| {
                    let content = content.clone();
                    let authors = if n % 2 == 0 { authors.clone() } else { vec![authors[1]] };
                    tokio::spawn(async move {
                        let ada = editor();
                        let (terms, names) = tokio::join!(
                            content.set_entry_terms(&ada, entry.id, set),
                            content.set_entry_authors(&ada, entry.id, authors),
                        );
                        (terms, names)
                    })
                })
                .collect();
            for task in tasks {
                let (terms, names) = task.await.unwrap();
                terms.unwrap();
                names.unwrap();
            }
        }
        let now = crate::taxonomy::terms::term_ids_of(database.read(), &[entry.id]).await.unwrap();
        assert!(sets.contains(&now[&entry.id]), "one of the sets won whole: {now:?}");
        let now =
            crate::taxonomy::authors::author_ids_of(database.read(), &[entry.id]).await.unwrap();
        assert!(now[&entry.id] == authors || now[&entry.id] == [authors[1]]);

        for _ in 0..4 {
            let entry = content.create(&ada, post("Doomed")).await.unwrap();
            let (deleted, terms, names) = tokio::join!(
                content.delete(&ada, entry.id, None),
                content.set_entry_terms(&ada, entry.id, tags.clone()),
                content.set_entry_authors(&ada, entry.id, authors.clone()),
            );
            deleted.unwrap();
            for result in [terms, names] {
                if let Err(error) = result {
                    assert_eq!(error.code(), "content.not_found");
                }
            }
        }
        for table in ["content_entry_terms", "content_entry_authors"] {
            let orphans = count(
                &database,
                &format!(
                    "SELECT COUNT(*) FROM {table} l WHERE NOT EXISTS \
                     (SELECT 1 FROM content_entries e WHERE e.id = l.entry_id)"
                ),
            )
            .await;
            assert_eq!(orphans, 0, "{table}");
        }
    })
    .await
    .unwrap();
}

/// Gắn term / tác giả và tạo link xem trước chỉ cần quyền sửa mục, không cần `content.read`
/// (như khôi phục revision, D-111).
#[tokio::test]
async fn sua_lien_ket_khong_can_quyen_doc() {
    TestDatabase::run(async |test| {
        let (content, _, _) =
            service(test, config(&[("AUTOXIDE_CONTENT_PREVIEW_SECRET", public::SECRET)])).await;
        let ada = editor();
        let writer = user(&["write"]);
        let tag =
            TermInput { locale: Some("en".into()), name: Some("Tag".into()), ..Default::default() };
        let tag = content.create_term(&ada, Taxonomy::Tag, tag).await.unwrap();
        let author = AuthorInput { display_name: Some("Ada".into()), ..Default::default() };
        let author = content.create_author(&ada, author).await.unwrap();
        let entry = content.create(&writer, post("Writer's")).await.unwrap();
        content.set_entry_terms(&writer, entry.id, vec![tag.term.id]).await.unwrap();
        content.set_entry_authors(&writer, entry.id, vec![author.id]).await.unwrap();
        content.preview_token(&writer, entry.id).await.unwrap();
        assert_eq!(
            content.get(&writer, entry.id).await.unwrap_err().code(),
            "permission_denied",
            "reading still needs content.read"
        );
    })
    .await
    .unwrap();
}

/// Xoá tài khoản bỏ trống cả `review_requested_by` và `ai_reviewed_by` (giữ thời điểm);
/// chạy hai lần vẫn vậy.
#[tokio::test]
async fn xoa_tai_khoan_bo_nguoi_duyet() {
    TestDatabase::run(async |test| {
        let (content, _, database) = service(test, config(&[])).await;
        let ada = editor();
        let mut input = post("Reviewed");
        input.fields.ai_generated = Some(true);
        let entry = content.create(&ada, input).await.unwrap();
        content.publish(&ada, entry.id, None, None, true).await.unwrap();
        content.set_review_request(&ada, entry.id, None, false).await.unwrap();

        let deleted = Delivered::new(
            "auth",
            autoxide_kernel::privacy::USER_DELETED.to_owned(),
            json!({"before": {"id": ada.user_id()}}),
        );
        content.handle(&deleted).await.unwrap();
        content.handle(&deleted).await.unwrap();

        let after = content.get(&editor(), entry.id).await.unwrap();
        assert_eq!(after.review_requested_by, None);
        assert!(after.review_requested_at.is_some(), "the request itself stays");
        let published = crate::entity::published::Entity::find_by_id(entry.id)
            .one(database.read())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(published.ai_reviewed_by, None);
        assert!(published.ai_reviewed_at.is_some(), "readers still see ai_reviewed");
    })
    .await
    .unwrap();
}
