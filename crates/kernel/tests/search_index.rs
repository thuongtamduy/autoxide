//! Bộ máy tìm kiếm thay được (bước 40 P7, D-220): chỉ mục mặc định trong database xếp khớp đúng dấu
//! trước, không lộ sang khách khác, lọc theo phạm vi nút trong bộ máy, bản ghi xoá thì biến khỏi kết
//! quả, job đồng bộ chịu chạy lại, và bộ máy giả thay được mà module không sửa gì. Cả hai database.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use autoxide_kernel::access::{Grant, Permission, Principal, Scope};
use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::pagination::{Order, Page, PageQuery, PageRequest, Pagination};
use autoxide_kernel::search::{
    SearchBackend, SearchDoc, SearchHit, SearchHits, SearchIndex, SearchKind, SearchRequest,
    SearchSource,
};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, TenantContext, root_of};
use autoxide_testkit::{TestApp, TestDatabase};
use uuid::Uuid;

const SHOP_READ: Permission = Permission::new("shop.read", "See the shop.").scoped();
const KIND: &str = "shop.product";

/// Nguồn tài liệu của module mẫu: bản ghi giữ trong bộ nhớ, theo khách.
#[derive(Default)]
struct Products {
    rows: Mutex<BTreeMap<(Uuid, Uuid), SearchDoc>>,
}

impl Products {
    fn put(&self, tenant: &TenantContext, doc: SearchDoc) {
        self.rows.lock().unwrap().insert((tenant.id().as_uuid(), doc.id), doc);
    }

    fn delete(&self, tenant: &TenantContext, id: Uuid) {
        self.rows.lock().unwrap().remove(&(tenant.id().as_uuid(), id));
    }
}

#[async_trait::async_trait]
impl SearchSource for Products {
    async fn load(&self, tenant: &TenantContext, ids: &[Uuid]) -> Result<Vec<SearchDoc>, Error> {
        let rows = self.rows.lock().unwrap();
        Ok(ids.iter().filter_map(|id| rows.get(&(tenant.id().as_uuid(), *id)).cloned()).collect())
    }

    async fn scan(
        &self,
        tenant: &TenantContext,
        after: Option<Uuid>,
        limit: u64,
    ) -> Result<Vec<SearchDoc>, Error> {
        let rows = self.rows.lock().unwrap();
        Ok(rows
            .iter()
            .filter(|((owner, id), _)| {
                *owner == tenant.id().as_uuid() && after.is_none_or(|after| *id > after)
            })
            .map(|(_, doc)| doc.clone())
            .take(usize::try_from(limit).unwrap())
            .collect())
    }
}

/// Module mẫu: khai loại `shop.product`; không biết bộ máy nào đang chạy.
struct Shop(Arc<Products>);

impl Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![SHOP_READ]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let kind = SearchKind::new(KIND)
            .field("name", 10)
            .field("description", 1)
            .filter("status")
            .permission(SHOP_READ);
        context.search_documents(kind, self.0.clone())
    }
}

/// Bộ máy giả: giữ tài liệu trong bộ nhớ, ghi lại mọi lần gọi.
#[derive(Default)]
struct FakeBackend {
    docs: Mutex<BTreeMap<Uuid, SearchDoc>>,
    queries: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl SearchBackend for FakeBackend {
    async fn index(&self, _: &TenantContext, docs: &[SearchDoc]) -> Result<(), Error> {
        let mut stored = self.docs.lock().unwrap();
        for doc in docs {
            stored.insert(doc.id, doc.clone());
        }
        Ok(())
    }

    async fn remove(&self, _: &TenantContext, _: &str, ids: &[Uuid]) -> Result<(), Error> {
        let mut stored = self.docs.lock().unwrap();
        for id in ids {
            stored.remove(id);
        }
        Ok(())
    }

    async fn clear(&self, _: &TenantContext, _: &str) -> Result<(), Error> {
        self.docs.lock().unwrap().clear();
        Ok(())
    }

    async fn query(
        &self,
        _: &TenantContext,
        _: &SearchKind,
        request: &SearchRequest,
    ) -> Result<SearchHits, Error> {
        self.queries.lock().unwrap().push(request.text.clone());
        let data = self
            .docs
            .lock()
            .unwrap()
            .values()
            .map(|doc| SearchHit {
                kind: doc.kind.clone(),
                id: doc.id,
                org_id: doc.org_id,
                locale: doc.locale.clone(),
                fields: doc.fields.clone(),
            })
            .collect();
        Ok(Page { data, pagination: Pagination::new(request.page.limit(), None) })
    }
}

struct FakeSearch(Arc<FakeBackend>);

impl Module for FakeSearch {
    fn name(&self) -> &'static str {
        "fakesearch"
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.search_backend("fake", self.0.clone())
    }
}

fn product(id: Uuid, org: OrgId, name: &str, description: &str) -> SearchDoc {
    SearchDoc::new(KIND, id, org)
        .locale("vi")
        .field("name", name)
        .field("description", description)
        .filter("status", "active")
}

fn request(text: &str) -> SearchRequest {
    SearchRequest {
        kind: KIND.to_owned(),
        text: text.to_owned(),
        filters: BTreeMap::new(),
        locale: None,
        scope: None,
        page: PageRequest::first(20, Order::Asc),
    }
}

fn admin() -> Principal {
    Principal::with_all_permissions(Uuid::now_v7())
}

fn names(hits: &SearchHits) -> Vec<String> {
    hits.data.iter().map(|hit| hit.fields["name"].clone()).collect()
}

async fn app(test: &TestDatabase, products: &Arc<Products>) -> (TestApp, SearchIndex) {
    let app = TestApp::builder(test).module(Shop(products.clone())).build().await.unwrap();
    let index = app.modules().api::<SearchIndex>().unwrap();
    (app, index.as_ref().clone())
}

#[tokio::test]
async fn tim_khong_dau_dung_dau_xep_truoc() {
    TestDatabase::run(async |test| {
        let products = Arc::new(Products::default());
        let (app, index) = app(test, &products).await;
        let tenant = app.default_tenant();
        let root = root_of(DEFAULT_TENANT);
        let (oil, strawberry, note) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        products.put(&tenant, product(strawberry, root, "Dâu tây Đà Lạt", "Trái cây"));
        products.put(&tenant, product(note, root, "Chảo chống dính", "Dùng ít dầu"));
        products.put(&tenant, product(oil, root, "Dầu ăn", "Chai 1 lít"));
        index.sync(&tenant, KIND, &[oil, strawberry, note]).await.unwrap();

        // Đúng dấu ở tên trước, rồi đúng dấu ở mô tả, rồi không dấu.
        let hits = index.search(&tenant, Some(&admin()), request("dầu")).await.unwrap();
        assert_eq!(names(&hits), ["Dầu ăn", "Chảo chống dính", "Dâu tây Đà Lạt"]);
        let hits = index.search(&tenant, Some(&admin()), request("da lat")).await.unwrap();
        assert_eq!(names(&hits), ["Dâu tây Đà Lạt"]);
        // Bộ lọc đã khai; bộ lọc lạ bị từ chối.
        let mut filtered = request("");
        filtered.filters.insert("status".to_owned(), "archived".to_owned());
        assert!(index.search(&tenant, Some(&admin()), filtered).await.unwrap().data.is_empty());
        let mut unknown = request("");
        unknown.filters.insert("price".to_owned(), "1".to_owned());
        let error = index.search(&tenant, Some(&admin()), unknown).await.unwrap_err();
        assert_eq!(error.code(), "validation_failed");
        // Trang theo con trỏ không lặp, không sót.
        let mut first = request("d");
        first.page = PageRequest::first(2, Order::Asc);
        let page = index.search(&tenant, Some(&admin()), first).await.unwrap();
        assert_eq!(page.data.len(), 2);
        let cursor = page.pagination.next_cursor.clone().unwrap();
        let mut next = request("d");
        let query = PageQuery::new(Some("2".to_owned()), Some(cursor), None);
        next.page = PageRequest::try_from(query).unwrap();
        let rest = index.search(&tenant, Some(&admin()), next).await.unwrap();
        assert_eq!(rest.data.len(), 1);
        // Không có permission: từ chối.
        let nobody = Principal::new(Uuid::now_v7(), Vec::<String>::new());
        let error = index.search(&tenant, Some(&nobody), request("dầu")).await.unwrap_err();
        assert_eq!(error.code(), "permission_denied");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ket_qua_khong_lo_sang_khach_khac() {
    TestDatabase::run(async |test| {
        let products = Arc::new(Products::default());
        let app = TestApp::builder(test)
            .tenant("a")
            .tenant("b")
            .module(Shop(products.clone()))
            .build()
            .await
            .unwrap();
        let index = app.modules().api::<SearchIndex>().unwrap();
        let (a, b) = (app.tenant("a").await.unwrap(), app.tenant("b").await.unwrap());
        let id = Uuid::now_v7();
        products.put(&a, product(id, root_of(a.id()), "Dầu ăn", ""));
        index.sync(&a, KIND, &[id]).await.unwrap();
        // Đồng bộ ở khách `b` không thấy bản ghi của `a` (nguồn theo khách): không đưa gì vào.
        index.sync(&b, KIND, &[id]).await.unwrap();
        let in_a = index.search(&a, Some(&admin()), request("dầu")).await.unwrap();
        assert_eq!(in_a.data.len(), 1);
        let in_b = index.search(&b, Some(&admin()), request("dầu")).await.unwrap();
        assert!(in_b.data.is_empty());
        assert_eq!(index.rebuild(&b, None).await.unwrap(), 0);
        assert_eq!(index.search(&a, Some(&admin()), request("")).await.unwrap().data.len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ket_qua_loc_theo_pham_vi_nut() {
    TestDatabase::run(async |test| {
        let products = Arc::new(Products::default());
        let (app, index) = app(test, &products).await;
        let tenant = app.default_tenant();
        // Cây: gốc → A, B; A → A1.
        let transaction = tenant.database().begin_write().await.unwrap();
        let root = OrgTree::insert_root(&transaction, DEFAULT_TENANT).await.unwrap();
        let (a, b, a1) = (
            OrgId::from_uuid(Uuid::now_v7()),
            OrgId::from_uuid(Uuid::now_v7()),
            OrgId::from_uuid(Uuid::now_v7()),
        );
        OrgTree::insert(&transaction, DEFAULT_TENANT, a, root, 8).await.unwrap();
        OrgTree::insert(&transaction, DEFAULT_TENANT, b, root, 8).await.unwrap();
        OrgTree::insert(&transaction, DEFAULT_TENANT, a1, a, 8).await.unwrap();
        transaction.commit().await.unwrap();
        let mut ids = Vec::new();
        for (org, name) in [(a, "Hàng A"), (a1, "Hàng A1"), (b, "Hàng B")] {
            let id = Uuid::now_v7();
            products.put(&tenant, product(id, org, name, ""));
            ids.push(id);
        }
        index.sync(&tenant, KIND, &ids).await.unwrap();
        let member = |org: OrgId, scope: Scope| {
            Principal::with_grants(
                Uuid::now_v7(),
                [(SHOP_READ.name().to_owned(), Grant::new(org, scope))],
            )
        };
        let principal = member(a, Scope::Subtree);
        let subtree = index.search(&tenant, Some(&principal), request("hàng")).await.unwrap();
        assert_eq!(names(&subtree), ["Hàng A", "Hàng A1"]);
        let principal = member(a, Scope::Unit);
        let unit = index.search(&tenant, Some(&principal), request("hàng")).await.unwrap();
        assert_eq!(names(&unit), ["Hàng A"]);
        let principal = member(b, Scope::Subtree);
        let sibling = index.search(&tenant, Some(&principal), request("hàng")).await.unwrap();
        assert_eq!(names(&sibling), ["Hàng B"]);
        let everything = index.search(&tenant, Some(&admin()), request("hàng")).await.unwrap();
        assert_eq!(everything.data.len(), 3);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn xoa_ban_ghi_thi_bien_khoi_ket_qua() {
    TestDatabase::run(async |test| {
        let products = Arc::new(Products::default());
        let (app, index) = app(test, &products).await;
        let tenant = app.default_tenant();
        let id = Uuid::now_v7();
        products.put(&tenant, product(id, root_of(DEFAULT_TENANT), "Dầu ăn", ""));
        index.sync(&tenant, KIND, &[id]).await.unwrap();
        assert_eq!(
            index.search(&tenant, Some(&admin()), request("dầu")).await.unwrap().data.len(),
            1
        );
        products.delete(&tenant, id);
        index.sync(&tenant, KIND, &[id]).await.unwrap();
        assert!(
            index.search(&tenant, Some(&admin()), request("dầu")).await.unwrap().data.is_empty()
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dong_bo_chiu_nhan_lai_event() {
    TestDatabase::run(async |test| {
        let products = Arc::new(Products::default());
        let (app, index) = app(test, &products).await;
        let tenant = app.default_tenant();
        let id = Uuid::now_v7();
        products.put(&tenant, product(id, root_of(DEFAULT_TENANT), "Dầu ăn", "Chai 1 lít"));
        // Module xếp việc đồng bộ trong transaction của thay đổi; worker chạy sau commit.
        let transaction = tenant.database().begin_write().await.unwrap();
        index.enqueue(&transaction, &tenant, KIND, &[id]).await.unwrap();
        transaction.commit().await.unwrap();
        let worker = app.modules().worker(4).unwrap();
        while worker.run_once().await.unwrap() > 0 {}
        assert_eq!(
            index.search(&tenant, Some(&admin()), request("dầu")).await.unwrap().data.len(),
            1
        );
        // Nhận lại cùng việc (job chạy lại): không trùng, cùng kết quả.
        index.sync(&tenant, KIND, &[id]).await.unwrap();
        index.sync(&tenant, KIND, &[id]).await.unwrap();
        let hits = index.search(&tenant, Some(&admin()), request("")).await.unwrap();
        assert_eq!(names(&hits), ["Dầu ăn"]);
        // Dựng lại chỉ mục cho cùng kết quả.
        assert_eq!(index.rebuild(&tenant, Some(KIND)).await.unwrap(), 1);
        assert_eq!(index.search(&tenant, Some(&admin()), request("")).await.unwrap().data.len(), 1);
        let error = index.rebuild(&tenant, Some("shop.unknown")).await.unwrap_err();
        assert_eq!(error.code(), "kernel.search_kind_unknown");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn backend_gia_thay_duoc_khong_sua_module() {
    TestDatabase::run(async |test| {
        let products = Arc::new(Products::default());
        let fake = Arc::new(FakeBackend::default());
        let app = TestApp::builder(test)
            .setting("AUTOXIDE_SEARCH_BACKEND", "fake")
            .module(FakeSearch(fake.clone()))
            .module(Shop(products.clone()))
            .build()
            .await
            .unwrap();
        let index = app.modules().api::<SearchIndex>().unwrap();
        let tenant = app.default_tenant();
        let id = Uuid::now_v7();
        products.put(&tenant, product(id, root_of(DEFAULT_TENANT), "Dầu ăn", ""));
        index.sync(&tenant, KIND, &[id]).await.unwrap();
        assert!(fake.docs.lock().unwrap().contains_key(&id));
        let hits = index.search(&tenant, Some(&admin()), request("dầu")).await.unwrap();
        assert_eq!(names(&hits), ["Dầu ăn"]);
        assert_eq!(fake.queries.lock().unwrap().as_slice(), ["dầu"]);
        // Bảng của bộ máy mặc định không được dùng.
        let unused = app.modules().api::<SearchIndex>().unwrap();
        assert_eq!(unused.configured(), "fake");
    })
    .await
    .unwrap();
}
