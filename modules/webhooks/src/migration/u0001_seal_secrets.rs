//! Mã hoá secret ký của endpoint có sẵn (bước 33 B7, D-257, D-278): bản cũ lưu chữ thường. Chạy
//! ngay sau `m0007_widen_secret` (đã có `tenant_id` — một phần dữ liệu đi kèm khi mã hoá — và cột
//! đủ dài). Chạy lại
//! được: giá trị đã mã hoá thì bỏ qua. `down` đưa về chữ thường để bản cũ đọc được.

use autoxide_kernel::error::Error;
use autoxide_kernel::migration::{DataUpgrade, UpgradeContext};
use autoxide_kernel::secret::is_sealed;
use autoxide_kernel::tenancy::TenantId;
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set};

use crate::entity::endpoint;
use crate::service::SECRET_PURPOSE;

pub(crate) struct SealSecrets;

#[async_trait::async_trait]
impl DataUpgrade for SealSecrets {
    fn name(&self) -> &'static str {
        "u0001_seal_secrets"
    }

    fn after(&self) -> &'static str {
        "m0007_widen_secret"
    }

    async fn up(&self, context: &UpgradeContext) -> Result<(), Error> {
        let sealer = context.secret().sealer().await?;
        let database = context.database();
        for row in endpoint::Entity::find().all(database.write()).await? {
            if is_sealed(&row.secret) {
                continue;
            }
            let tenant = TenantId::from_uuid(row.tenant_id);
            let sealed =
                sealer.seal(tenant, SECRET_PURPOSE, &row.id.to_string(), row.secret.as_bytes())?;
            let mut active = row.into_active_model();
            active.secret = Set(sealed);
            active.update(database.write()).await?;
        }
        Ok(())
    }

    async fn down(&self, context: &UpgradeContext) -> Result<(), Error> {
        let sealer = context.secret().sealer().await?;
        let database = context.database();
        for row in endpoint::Entity::find().all(database.write()).await? {
            if !is_sealed(&row.secret) {
                continue;
            }
            let tenant = TenantId::from_uuid(row.tenant_id);
            let plain = sealer.open(tenant, SECRET_PURPOSE, &row.id.to_string(), &row.secret)?;
            let plain = String::from_utf8(plain.to_vec())
                .map_err(|_| Error::internal("a stored webhook secret is not text"))?;
            let mut active = row.into_active_model();
            active.secret = Set(plain);
            active.update(database.write()).await?;
        }
        Ok(())
    }
}
