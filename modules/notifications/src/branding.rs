//! Thương hiệu của mail HTML theo khách (D-206, D-365): tên, logo, màu chủ đạo, màu nền, chân thư.
//! Chân thư thêm tên và địa chỉ chủ sở hữu từ hồ sơ pháp lý của `compliance` khi có (thông tin vốn
//! phải công khai). Chưa đặt gì thì dùng khung mặc định.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::TenantContext;
use sea_orm::EntityTrait;
use sea_orm::Set;
use sea_orm::sea_query::OnConflict;
use serde_json::{Value, json};

use crate::custom::require_read;
use crate::entity::branding;
use crate::events;
use crate::layout::{Brand, DEFAULT_BACKGROUND, DEFAULT_PRIMARY, valid_color};
use crate::permissions::BRANDING_MANAGE;
use crate::service::{NotificationsService, now};

/// Thương hiệu gửi lên.
#[derive(Debug, Clone, Default)]
pub(crate) struct BrandingInput {
    pub brand_name: Option<String>,
    pub logo_url: Option<String>,
    pub primary_color: Option<String>,
    pub background_color: Option<String>,
    pub footer_text: Option<String>,
}

/// Dòng mặc định khi khách chưa đặt.
fn default_row(tenant: &TenantContext) -> branding::Model {
    branding::Model {
        tenant_id: tenant.id().as_uuid(),
        brand_name: None,
        logo_url: None,
        primary_color: DEFAULT_PRIMARY.to_owned(),
        background_color: DEFAULT_BACKGROUND.to_owned(),
        footer_text: None,
        updated_by: None,
        updated_at: chrono::DateTime::UNIX_EPOCH,
    }
}

fn blank_to_none(value: Option<String>) -> Option<String> {
    value.map(|text| text.trim().to_owned()).filter(|text| !text.is_empty())
}

impl NotificationsService {
    pub(crate) async fn get_branding(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<branding::Model, Error> {
        if !principal.has(&BRANDING_MANAGE) {
            require_read(principal)?;
        }
        Ok(branding::Entity::find_by_id(tenant.id().as_uuid())
            .one(tenant.database().read())
            .await?
            .unwrap_or_else(|| default_row(tenant)))
    }

    pub(crate) async fn put_branding(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: BrandingInput,
    ) -> Result<branding::Model, Error> {
        principal.require(&BRANDING_MANAGE)?;
        let brand_name = blank_to_none(input.brand_name);
        let logo_url = blank_to_none(input.logo_url);
        let footer_text = blank_to_none(input.footer_text);
        let primary = input.primary_color.unwrap_or_else(|| DEFAULT_PRIMARY.to_owned());
        let background = input.background_color.unwrap_or_else(|| DEFAULT_BACKGROUND.to_owned());
        let mut problems = Vec::new();
        if brand_name.as_ref().is_some_and(|name| name.chars().count() > 100) {
            problems.push(
                FieldError::new("brand_name", "length", "must be 1 to {max} characters")
                    .with_param("max", 100),
            );
        }
        if let Some(url) = &logo_url
            && (url.len() > 2000
                || !url.to_ascii_lowercase().starts_with("https://")
                || url.chars().any(|c| c.is_control() || c.is_whitespace()))
        {
            problems.push(FieldError::new(
                "logo_url",
                "invalid",
                "must be an https URL of at most 2000 characters",
            ));
        }
        for (field, value) in [("primary_color", &primary), ("background_color", &background)] {
            if !valid_color(value) {
                problems.push(FieldError::new(field, "invalid", "must be a color like `#1f6feb`"));
            }
        }
        if footer_text.as_ref().is_some_and(|text| text.chars().count() > 1000) {
            problems.push(
                FieldError::new("footer_text", "length", "must be 1 to {max} characters")
                    .with_param("max", 1000),
            );
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let row = branding::Model {
            tenant_id: tenant.id().as_uuid(),
            brand_name,
            logo_url,
            primary_color: primary.to_ascii_lowercase(),
            background_color: background.to_ascii_lowercase(),
            footer_text,
            updated_by: Some(principal.user_id()),
            updated_at: now(),
        };
        let transaction = tenant.database().begin_write().await?;
        branding::Entity::insert(branding::ActiveModel {
            tenant_id: Set(row.tenant_id),
            brand_name: Set(row.brand_name.clone()),
            logo_url: Set(row.logo_url.clone()),
            primary_color: Set(row.primary_color.clone()),
            background_color: Set(row.background_color.clone()),
            footer_text: Set(row.footer_text.clone()),
            updated_by: Set(row.updated_by),
            updated_at: Set(row.updated_at),
        })
        .on_conflict(
            OnConflict::column(branding::Column::TenantId)
                .update_columns([
                    branding::Column::BrandName,
                    branding::Column::LogoUrl,
                    branding::Column::PrimaryColor,
                    branding::Column::BackgroundColor,
                    branding::Column::FooterText,
                    branding::Column::UpdatedBy,
                    branding::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec_without_returning(&transaction)
        .await?;
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                Some("notifications.branding".to_owned()),
                &events::BrandingUpdated(json!({})),
            )
            .await?;
        transaction.commit().await?;
        Ok(row)
    }

    /// Thương hiệu cho khung mail lúc gửi / xem trước.
    pub(crate) async fn brand(&self, tenant: &TenantContext) -> Result<Brand, Error> {
        let row = branding::Entity::find_by_id(tenant.id().as_uuid())
            .one(tenant.database().read())
            .await?
            .unwrap_or_else(|| default_row(tenant));
        let mut footer_lines: Vec<String> = row.footer_text.iter().cloned().collect();
        if let Some(legal) = &self.legal
            && let Some(variables) = legal.template_variables(tenant, None).await?
        {
            let text =
                |value: &Value| value.as_str().map(str::to_owned).filter(|text| !text.is_empty());
            let owner = &variables["owner"];
            let line: Vec<String> = [text(&owner["legal_name"]), text(&owner["address"])]
                .into_iter()
                .flatten()
                .collect();
            if !line.is_empty() {
                footer_lines.push(line.join(" · "));
            }
        }
        Ok(Brand {
            name: row.brand_name,
            logo_url: row.logo_url,
            primary_color: row.primary_color,
            background_color: row.background_color,
            footer_lines,
        })
    }
}
