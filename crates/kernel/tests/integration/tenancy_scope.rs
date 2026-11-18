//! `ScopeExt` (bước 33 A5): mọi kiểu truy vấn của entity cấp khách hàng chỉ chạm dòng của khách
//! được chỉ — đọc, đọc kèm entity khác, sửa / xoá nhiều dòng, sửa / xoá một dòng. Chạy trên cả
//! SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::migration::prelude::*;
use autoxide_kernel::migration::{Migrator, ModuleMigrations};
use autoxide_kernel::tenancy::{
    MAIN_PLACEMENT, ScopeExt, TenantContext, TenantId, TenantScoped, TenantStatus,
};
use autoxide_testkit::TestDatabase;
use sea_orm::entity::prelude::*;
use sea_orm::{ActiveValue::Set, IntoActiveModel, QueryOrder};

mod note {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "demo_note")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub title: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(has_many = "super::tag::Entity")]
        Tag,
    }

    impl Related<super::tag::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Tag.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

mod tag {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "demo_tag")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub note_id: Uuid,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "super::note::Entity",
            from = "Column::NoteId",
            to = "super::note::Column::Id"
        )]
        Note,
    }

    impl Related<super::note::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Note.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

impl TenantScoped for note::Entity {
    fn tenant_column() -> Self::Column {
        note::Column::TenantId
    }
}

struct CreateDemo;

impl MigrationName for CreateDemo {
    fn name(&self) -> &str {
        "m0001_create_demo"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateDemo {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("demo_note")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("title", 50))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("demo_tag")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("note_id"))
                    .col(schema::string_len("name", 50))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("demo_tag").to_owned()).await?;
        manager.drop_table(Table::drop().table("demo_note").to_owned()).await
    }
}

async fn setup(test: &TestDatabase) -> (Database, TenantContext, TenantContext) {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    let database = Database::connect(&reader.finish(config).unwrap().0).await.unwrap();
    Migrator::new(vec![ModuleMigrations::new("demo", vec![Box::new(CreateDemo)])])
        .unwrap()
        .up(&database)
        .await
        .unwrap();
    let ours = TenantContext::default_on(database.clone());
    let theirs = TenantContext::new(
        TenantId::from_uuid(Uuid::now_v7()),
        "other",
        "Other",
        TenantStatus::Active,
        MAIN_PLACEMENT,
        database.clone(),
    );
    (database, ours, theirs)
}

async fn add(database: &Database, tenant: &TenantContext, title: &str) -> Uuid {
    let id = Uuid::now_v7();
    note::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant.id().as_uuid()),
        title: Set(title.to_owned()),
    }
    .insert(database.write())
    .await
    .unwrap();
    tag::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant.id().as_uuid()),
        note_id: Set(id),
        name: Set(format!("{title}-tag")),
    }
    .insert(database.write())
    .await
    .unwrap();
    id
}

async fn titles(database: &Database, tenant: &TenantContext) -> Vec<String> {
    note::Entity::find()
        .scoped(tenant)
        .order_by_asc(note::Column::Title)
        .all(database.read())
        .await
        .unwrap()
        .into_iter()
        .map(|note| note.title)
        .collect()
}

#[tokio::test]
async fn doc_va_doc_kem_chi_thay_dong_cua_khach() {
    TestDatabase::run(async |test| {
        let (database, ours, theirs) = setup(test).await;
        let mine = add(&database, &ours, "a").await;
        add(&database, &theirs, "b").await;

        assert_eq!(titles(&database, &ours).await, ["a"]);
        assert_eq!(titles(&database, &theirs).await, ["b"]);
        let read = database.read();
        // Theo ID của khách khác: như không có.
        assert!(note::Entity::find_by_id(mine).scoped(&theirs).one(read).await.unwrap().is_none());
        assert!(note::Entity::find_by_id(mine).scoped(&ours).one(read).await.unwrap().is_some());
        let with_tags =
            note::Entity::find().find_also_related(tag::Entity).scoped(&ours).all(read).await;
        assert_eq!(with_tags.unwrap().len(), 1);
        let with_all = note::Entity::find().find_with_related(tag::Entity).scoped(&theirs);
        let with_all = with_all.all(read).await.unwrap();
        assert_eq!((with_all.len(), with_all[0].0.title.as_str()), (1, "b"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn sua_xoa_khong_cham_khach_khac() {
    TestDatabase::run(async |test| {
        let (database, ours, theirs) = setup(test).await;
        let mine = add(&database, &ours, "a").await;
        let other = add(&database, &theirs, "b").await;
        let write = database.write();

        let changed = note::Entity::update_many()
            .col_expr(note::Column::Title, Expr::value("changed"))
            .scoped(&ours)
            .exec(write)
            .await
            .unwrap();
        assert_eq!(changed.rows_affected, 1);
        assert_eq!(titles(&database, &theirs).await, ["b"]);

        // Sửa một dòng theo ID của khách khác: không dòng nào đổi.
        let row = note::Entity::find_by_id(other).one(write).await.unwrap().unwrap();
        let mut row = row.into_active_model();
        row.title = Set("stolen".to_owned());
        let refused = note::Entity::update(row.clone()).validate().unwrap().scoped(&ours);
        assert!(refused.exec(write).await.is_err(), "no row of ours has this id");
        assert_eq!(titles(&database, &theirs).await, ["b"]);
        note::Entity::update(row).validate().unwrap().scoped(&theirs).exec(write).await.unwrap();
        assert_eq!(titles(&database, &theirs).await, ["stolen"]);

        let gone = note::Entity::delete_by_id(mine).scoped(&theirs);
        assert_eq!(gone.exec(write).await.unwrap().rows_affected, 0);
        let gone = note::Entity::delete_many().scoped(&theirs).exec(write).await.unwrap();
        assert_eq!(gone.rows_affected, 1);
        assert_eq!(titles(&database, &ours).await, ["changed"]);
    })
    .await
    .unwrap();
}
