//! Hỏi đáp trên `comments` (D-202): câu hỏi là bình luận gốc, câu trả lời là trả lời trong luồng.
//! Đích giả `catalog.product` (module `catalog` chưa có, bước 41): bộ tra, bộ tính nhãn và event
//! vòng đời đăng ký vào sổ của kernel như module thật sẽ làm.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use autoxide_kernel::access::{PostingVerifier, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::http::ClientInfo;
use autoxide_kernel::pagination::{Order, PageQuery, PageRequest};
use autoxide_kernel::resource::{
    BadgeResolver, Resource, ResourceChange, ResourceEvent, ResourceResolver,
};
use autoxide_testkit::TestDatabase;
use serde_json::json;
use uuid::Uuid;

use super::{page, visitor};
use crate::config::{CommentsConfig, Mode};
use crate::http::CommentDetail;
use crate::moderation::QueueFilter;
use crate::posting::NewComment;
use crate::service::{CommentsService, Status};
use crate::targets::OwnerEvents;
use crate::testing::{Harness, admin, service};

const PRODUCTS: &str = "catalog.product";
const PRODUCT: &str = "p-1";

/// Bộ tra giả của `catalog.product`: ID bắt đầu bằng `p` là sản phẩm đang bán.
struct Products;

#[async_trait::async_trait]
impl ResourceResolver for Products {
    async fn resolve(&self, id: &str) -> Result<Option<Resource>, Error> {
        Ok(id.starts_with('p').then(|| Resource::new("Bếp từ").public(true)))
    }
}

/// `seller`: người có `catalog.manage` (bước 41 xét thêm nút tổ chức sở hữu sản phẩm).
struct Sellers;

#[async_trait::async_trait]
impl BadgeResolver for Sellers {
    async fn badges(&self, _: &str, author: &Principal) -> Result<Vec<&'static str>, Error> {
        Ok(if author.allows("catalog.manage") { vec!["seller"] } else { Vec::new() })
    }
}

/// `verified_buyer`: người trong danh sách đã nhận hàng (bước 45 hỏi `orders`).
#[derive(Default)]
struct Buyers(Mutex<HashSet<Uuid>>);

#[async_trait::async_trait]
impl BadgeResolver for Buyers {
    async fn badges(&self, _: &str, author: &Principal) -> Result<Vec<&'static str>, Error> {
        let bought = self.0.lock().unwrap().contains(&author.user_id());
        Ok(if bought { vec!["verified_buyer"] } else { Vec::new() })
    }
}

/// Bộ hỏi xác thực giả: chỉ người trong danh sách đã xác thực số điện thoại.
#[derive(Default)]
struct Verified(Mutex<HashSet<Uuid>>);

#[async_trait::async_trait]
impl PostingVerifier for Verified {
    async fn is_verified(&self, user_id: Uuid) -> Result<bool, Error> {
        Ok(self.0.lock().unwrap().contains(&user_id))
    }
}

/// Cấu hình hỏi đáp: `catalog.product` nhận bình luận, chỉ tài khoản đã xác thực.
fn questions_config(user_mode: Mode) -> CommentsConfig {
    CommentsConfig {
        target_types: vec![PRODUCTS.to_owned()],
        verified_only: vec![PRODUCTS.to_owned()],
        user_mode,
        ..crate::testing::config()
    }
}

struct Shop {
    h: Harness,
    buyers: Arc<Buyers>,
    verified: Arc<Verified>,
}

async fn shop(test: &TestDatabase, user_mode: Mode) -> Shop {
    let h = service(test, questions_config(user_mode)).await;
    let buyers = Arc::new(Buyers::default());
    let verified = Arc::new(Verified::default());
    let resources = &h.service.resources;
    resources.register("catalog", PRODUCTS, Arc::new(Products)).unwrap();
    resources.register_badges("catalog", PRODUCTS, &["seller"], Arc::new(Sellers)).unwrap();
    resources.register_badges("orders", PRODUCTS, &["verified_buyer"], buyers.clone()).unwrap();
    h.service.verification.register("auth", verified.clone()).unwrap();
    Shop { h, buyers, verified }
}

/// Người dùng đã xác thực, có tên hiển thị `name`, `comments.post` và `extra`.
async fn person(shop: &Shop, name: &str, extra: &[&str]) -> Principal {
    let mut permissions = vec!["comments.post".to_owned()];
    permissions.extend(extra.iter().map(|permission| (*permission).to_owned()));
    let principal = Principal::new(Uuid::now_v7(), permissions);
    shop.h.service.set_profile(&principal, name).await.unwrap();
    shop.verified.0.lock().unwrap().insert(principal.user_id());
    principal
}

fn question(body: &str) -> NewComment {
    NewComment {
        target_type: PRODUCTS.to_owned(),
        target_id: PRODUCT.to_owned(),
        body: body.to_owned(),
        ..NewComment::default()
    }
}

fn answer_to(id: Uuid, body: &str) -> NewComment {
    NewComment { parent_id: Some(id), ..question(body) }
}

async fn ask(service: &CommentsService, who: &Principal, body: &str) -> Uuid {
    service.post(Some(who), &ClientInfo::default(), question(body)).await.unwrap().comment.row.id
}

/// Nội dung các câu hỏi đang hiện của sản phẩm, theo thứ tự trả về.
async fn shown(service: &CommentsService, search: Option<&str>) -> Vec<String> {
    let found = service.thread(PRODUCTS, PRODUCT, &page(Order::Desc), 3, search).await.unwrap();
    found.data.into_iter().map(|thread| thread.root.row.body).collect()
}

/// (số câu hỏi đang hiện, số câu hỏi chờ duyệt) của sản phẩm.
async fn question_counts(service: &CommentsService) -> (i32, i32) {
    let target = service.target(PRODUCTS, PRODUCT).await.unwrap();
    (target.published_threads, target.pending_threads)
}

#[tokio::test]
async fn cau_hoi_chi_hien_sau_khi_duyet() {
    TestDatabase::run(async |test| {
        let shop = shop(test, Mode::Pre).await;
        let service = &shop.h.service;
        let buyer = person(&shop, "Lan", &[]).await;
        let posted = service
            .post(Some(&buyer), &ClientInfo::default(), question("Bếp có kèm nồi không?"))
            .await
            .unwrap();
        assert_eq!(posted.shown, Status::Pending);
        assert!(shown(service, None).await.is_empty(), "waits for approval");
        assert_eq!(question_counts(service).await, (0, 1));
        assert_eq!(
            service.counts(PRODUCTS, &[PRODUCT.to_owned()]).await.unwrap(),
            [(PRODUCT.to_owned(), 0, 0)]
        );

        service.set_status(&admin(), posted.comment.row.id, Status::Published, None).await.unwrap();
        assert_eq!(shown(service, None).await, ["Bếp có kèm nồi không?"]);
        assert_eq!(question_counts(service).await, (1, 0));
        assert_eq!(
            service.counts(PRODUCTS, &[PRODUCT.to_owned()]).await.unwrap(),
            [(PRODUCT.to_owned(), 1, 1)]
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tra_loi_cua_nguoi_ban_co_nhan_seller() {
    TestDatabase::run(async |test| {
        let shop = shop(test, Mode::Post).await;
        let service = &shop.h.service;
        let buyer = person(&shop, "Lan", &[]).await;
        let seller = person(&shop, "Bếp Mộc", &["catalog.manage"]).await;
        let id = ask(service, &buyer, "Bảo hành bao lâu?").await;
        let answer = service
            .post(Some(&seller), &ClientInfo::default(), answer_to(id, "Bảo hành 24 tháng."))
            .await
            .unwrap();
        assert_eq!(answer.comment.row.badges, "seller");
        let thread = service.thread(PRODUCTS, PRODUCT, &page(Order::Desc), 3, None).await.unwrap();
        let reply = crate::http::PublicComment::from(thread.data[0].replies[0].clone());
        assert_eq!(reply.badges, ["seller"]);
        assert_eq!(reply.author.name.as_deref(), Some("Bếp Mộc"));
        let asked = crate::http::PublicComment::from(thread.data[0].root.clone());
        assert!(asked.badges.is_empty(), "the buyer did not buy yet");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nguoi_chua_mua_khong_co_nhan_verified_buyer() {
    TestDatabase::run(async |test| {
        let shop = shop(test, Mode::Post).await;
        let service = &shop.h.service;
        let asker = person(&shop, "Lan", &[]).await;
        let (stranger, owner) = (person(&shop, "Minh", &[]).await, person(&shop, "Hoa", &[]).await);
        shop.buyers.0.lock().unwrap().insert(owner.user_id());
        let id = ask(service, &asker, "Bếp có ồn không?").await;
        let guess = service
            .post(Some(&stranger), &ClientInfo::default(), answer_to(id, "Chắc là không."))
            .await
            .unwrap();
        let used = service
            .post(Some(&owner), &ClientInfo::default(), answer_to(id, "Rất êm, mình dùng rồi."))
            .await
            .unwrap();
        assert_eq!(guess.comment.row.badges, "");
        assert_eq!(used.comment.row.badges, "verified_buyer");
        let detail = CommentDetail::from(service.find_named(used.comment.row.id).await.unwrap());
        assert_eq!(detail.badges, ["verified_buyer"]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tim_cau_hoi_khong_phan_biet_dau() {
    TestDatabase::run(async |test| {
        let shop = shop(test, Mode::Post).await;
        let service = &shop.h.service;
        let asker = person(&shop, "Lan", &[]).await;
        for body in
            ["Bảo hành bao lâu?", "BAO HANH ở đâu?", "Giao hàng mấy ngày?", "Có bảo hành tận nhà?"]
        {
            ask(service, &asker, body).await;
        }
        // Khớp đúng dấu trước (mới trước trong tầng), rồi khớp không dấu.
        assert_eq!(
            shown(service, Some("bảo hành")).await,
            ["Có bảo hành tận nhà?", "Bảo hành bao lâu?", "BAO HANH ở đâu?"]
        );
        assert_eq!(
            shown(service, Some("bao hanh")).await,
            ["BAO HANH ở đâu?", "Có bảo hành tận nhà?", "Bảo hành bao lâu?"]
        );
        assert!(shown(service, Some("đổi trả")).await.is_empty());
        // `%` gõ vào là chữ, không phải ký tự đại diện.
        assert!(shown(service, Some("%")).await.is_empty());

        // Lật từng trang một dòng: không trùng, không sót, giữ thứ tự tầng.
        let (mut cursor, mut seen) = (None::<String>, Vec::new());
        loop {
            let query = PageQuery::new(Some("1".to_owned()), cursor.clone(), None);
            let request = PageRequest::try_from(query).unwrap();
            let found =
                service.thread(PRODUCTS, PRODUCT, &request, 0, Some("bảo hành")).await.unwrap();
            seen.extend(found.data.into_iter().map(|thread| thread.root.row.body));
            match found.pagination.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        assert_eq!(seen, ["Có bảo hành tận nhà?", "Bảo hành bao lâu?", "BAO HANH ở đâu?"]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn luu_tru_dich_thi_dong_va_an_hoi_dap() {
    TestDatabase::run(async |test| {
        let shop = shop(test, Mode::Post).await;
        let service = &shop.h.service;
        let asker = person(&shop, "Lan", &[]).await;
        ask(service, &asker, "Còn hàng không?").await;
        // `catalog` khai: lưu trữ sản phẩm là gỡ — đóng và ẩn hỏi đáp (đặc tả bước 41).
        let declared =
            [ResourceEvent::new("catalog.product_archived", PRODUCTS, ResourceChange::Removed)];
        let handler = OwnerEvents {
            service: Arc::clone(service),
            events: crate::targets::owner_events(&declared, &service.config.target_types),
        };
        let event = Delivered::new("catalog", "catalog.product_archived", json!({}))
            .subject(Some(format!("{PRODUCTS}:{PRODUCT}")));
        handler.handle(&event).await.unwrap();
        handler.handle(&event).await.unwrap();

        let target = service.target(PRODUCTS, PRODUCT).await.unwrap();
        assert!(target.hidden);
        assert_eq!(target.effective_state(), "closed");
        assert!(shown(service, None).await.is_empty());
        assert_eq!(
            service.counts(PRODUCTS, &[PRODUCT.to_owned()]).await.unwrap(),
            [(PRODUCT.to_owned(), 0, 0)]
        );
        let refused = service
            .post(Some(&asker), &ClientInfo::default(), question("Bao giờ có lại?"))
            .await
            .unwrap_err();
        assert_eq!(refused.code(), "comments.target_closed");
        // Đích loại khác, hay event không khai: không đụng tới.
        let other = Delivered::new("catalog", "catalog.product_archived", json!({}))
            .subject(Some("content.entry:public-1".to_owned()));
        handler.handle(&other).await.unwrap();
        assert!(service.target("content.entry", "public-1").await.is_err(), "kind not enabled");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tai_khoan_chua_xac_thuc_khong_hoi_duoc() {
    TestDatabase::run(async |test| {
        let shop = shop(test, Mode::Post).await;
        let service = &shop.h.service;
        let unverified = Principal::new(Uuid::now_v7(), ["comments.post".to_owned()]);
        service.set_profile(&unverified, "Nam").await.unwrap();
        let refused = service
            .post(Some(&unverified), &ClientInfo::default(), question("Có giao Đà Lạt không?"))
            .await
            .unwrap_err();
        assert_eq!(refused.code(), "comments.verification_required");

        // Khách không có tài khoản để xác thực.
        let mut as_guest = question("Có giao Đà Lạt không?");
        as_guest.author_name = Some("Khách".to_owned());
        as_guest.form_token = Some(super::token(service, PRODUCTS).await);
        let refused = service.post(None, &visitor("203.0.113.40"), as_guest).await.unwrap_err();
        assert_eq!(refused.code(), "comments.verification_required");

        // Xác thực xong thì hỏi được, và trả lời cũng cần xác thực.
        shop.verified.0.lock().unwrap().insert(unverified.user_id());
        let id = ask(service, &unverified, "Có giao Đà Lạt không?").await;
        let late = Principal::new(Uuid::now_v7(), ["comments.post".to_owned()]);
        service.set_profile(&late, "Tú").await.unwrap();
        let refused = service
            .post(Some(&late), &ClientInfo::default(), answer_to(id, "Có."))
            .await
            .unwrap_err();
        assert_eq!(refused.code(), "comments.verification_required");
    })
    .await
    .unwrap();
}

/// Chưa module nào giữ trạng thái xác thực (trước bước 37): tài khoản đăng nhập hỏi được như
/// trước D-202, khách thì không.
#[tokio::test]
async fn chua_co_bo_xac_thuc_thi_tai_khoan_dang_nhap_hoi_duoc() {
    TestDatabase::run(async |test| {
        let h = service(test, questions_config(Mode::Post)).await;
        h.service.resources.register("catalog", PRODUCTS, Arc::new(Products)).unwrap();
        let user = Principal::new(Uuid::now_v7(), ["comments.post".to_owned()]);
        h.service.set_profile(&user, "Lan").await.unwrap();
        let posted =
            h.service.post(Some(&user), &ClientInfo::default(), question("Hỏi")).await.unwrap();
        assert_eq!(posted.shown, Status::Published);
        assert_eq!(posted.comment.row.badges, "", "no badge source registered");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cau_hoi_chua_tra_loi() {
    TestDatabase::run(async |test| {
        let shop = shop(test, Mode::Post).await;
        let service = &shop.h.service;
        let asker = person(&shop, "Lan", &[]).await;
        let seller = person(&shop, "Bếp Mộc", &["catalog.manage"]).await;
        let first = ask(service, &asker, "Câu một").await;
        let second = ask(service, &asker, "Câu hai").await;
        service
            .post(Some(&seller), &ClientInfo::default(), answer_to(first, "Đã trả lời"))
            .await
            .unwrap();
        let filter = QueueFilter {
            status: Some(Status::Published),
            target_type: Some(PRODUCTS.to_owned()),
            unanswered: true,
            ..QueueFilter::default()
        };
        let found = service.queue(&admin(), &filter, &page(Order::Asc)).await.unwrap();
        let ids: Vec<Uuid> = found.data.iter().map(|comment| comment.row.id).collect();
        assert_eq!(ids, [second]);
    })
    .await
    .unwrap();
}

/// Nhiều câu trả lời cùng lúc: `reply_count` của câu hỏi và số đếm của đích (mọi bình luận, câu
/// hỏi) khớp số dòng thật — `UPDATE … SET n = n + 1` trong transaction, không đọc rồi ghi.
#[tokio::test]
async fn tra_loi_dong_thoi_khong_lam_lech_so_dem() {
    TestDatabase::run(async |test| {
        let shop = shop(test, Mode::Post).await;
        let service = &shop.h.service;
        let asker = person(&shop, "Lan", &[]).await;
        let id = ask(service, &asker, "Pin dùng bao lâu?").await;
        let mut answerers = Vec::new();
        for index in 0..6 {
            answerers.push(person(&shop, &format!("Người {index}"), &[]).await);
        }
        let mut tasks = Vec::new();
        for principal in answerers {
            let service = Arc::clone(service);
            tasks.push(tokio::spawn(async move {
                let input = answer_to(id, &format!("Trả lời của {}", principal.user_id()));
                service.post(Some(&principal), &ClientInfo::default(), input).await.map(|_| ())
            }));
        }
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        assert_eq!(service.find(id).await.unwrap().reply_count, 6);
        let target = service.target(PRODUCTS, PRODUCT).await.unwrap();
        assert_eq!((target.published_count, target.pending_count), (7, 0));
        assert_eq!((target.published_threads, target.pending_threads), (1, 0));
    })
    .await
    .unwrap();
}
