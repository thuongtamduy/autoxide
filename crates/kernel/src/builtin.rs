//! Module nội bộ `kernel`: luôn được đăng ký đầu tiên (trong [`crate::module::Modules::new`]),
//! giữ những bảng mà cả chương trình dùng chung, không thuộc module nghiệp vụ nào — hiện là
//! `kernel_secrets` (khoá ký, [`crate::secret`]). Là một module như mọi module khác nên luật
//! sở hữu bảng, `autoxide backup` / `copy` và `migrate` áp cho nó y như vậy.

use std::sync::Arc;

use crate::config::{ConfigReader, Invalid};
use crate::error::Error;
use crate::migration::prelude::*;
use crate::module::{Module, StartContext};
use crate::secret::SecretKey;

/// Tên của module nội bộ; module khác không được dùng tên này.
pub(crate) const NAME: &str = "kernel";

#[derive(Default)]
pub(crate) struct KernelModule {
    /// Giá trị `AUTOXIDE_SECRET_KEY`; có sau `configure`.
    secret: Option<Vec<u8>>,
}

impl Module for KernelModule {
    fn name(&self) -> &'static str {
        NAME
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(CreateSecrets)]
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.secret = crate::secret::read_setting(reader)?;
        Ok(())
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let key = SecretKey::new(context.database().clone(), self.secret.clone());
        context.provide::<SecretKey>(Arc::new(key))
    }
}

/// Bảng `kernel_secrets`: mỗi dòng một khoá chương trình tự sinh (tên → giá trị).
struct CreateSecrets;

impl MigrationName for CreateSecrets {
    fn name(&self) -> &str {
        "m0001_create_secrets"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateSecrets {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("kernel_secrets")
                    .col(schema::string_len("name", 64).primary_key())
                    .col(schema::text("value"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("kernel_secrets").to_owned()).await
    }
}
