//! Mã hoá secret TOTP có sẵn (bước 33 B12, D-257): bản cũ lưu secret base32 chữ thường. Chạy ngay
//! sau `m0027_widen_totp_secret` (đã có `tenant_id` — một phần dữ liệu đi kèm khi mã hoá — và cột
//! đủ dài). Chạy lại được: giá trị đã mã hoá thì bỏ qua. `down` đưa về chữ thường để bản cũ đọc
//! được.

use autoxide_kernel::error::Error;
use autoxide_kernel::migration::{DataUpgrade, UpgradeContext};
use autoxide_kernel::secret::is_sealed;
use autoxide_kernel::tenancy::TenantId;
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set};

use crate::mfa::entity::totp;
use crate::mfa::service::SECRET_PURPOSE;

pub(crate) struct SealTotp;

#[async_trait::async_trait]
impl DataUpgrade for SealTotp {
    fn name(&self) -> &'static str {
        "u0001_seal_totp"
    }

    fn after(&self) -> &'static str {
        "m0027_widen_totp_secret"
    }

    async fn up(&self, context: &UpgradeContext) -> Result<(), Error> {
        let sealer = context.secret().sealer().await?;
        let database = context.database();
        for row in totp::Entity::find().all(database.write()).await? {
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
        for row in totp::Entity::find().all(database.write()).await? {
            if !is_sealed(&row.secret) {
                continue;
            }
            let tenant = TenantId::from_uuid(row.tenant_id);
            let plain = sealer.open(tenant, SECRET_PURPOSE, &row.id.to_string(), &row.secret)?;
            let plain = String::from_utf8(plain.to_vec())
                .map_err(|_| Error::internal("a stored TOTP secret is not text"))?;
            let mut active = row.into_active_model();
            active.secret = Set(plain);
            active.update(database.write()).await?;
        }
        Ok(())
    }
}
