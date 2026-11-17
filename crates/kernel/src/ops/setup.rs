//! Việc thiết lập (bước 51 P10, D-201): module khai **bước thiết lập** (tạo trang giới thiệu, nối
//! cổng thanh toán, khai thông tin người bán…) bằng
//! [`StartContext::setup_step`](crate::module::StartContext). Bước nào xong tính từ dữ liệu
//! (`done`), không có nút "đánh dấu xong". `GET /api/v1/kernel/setup` trả các bước người gọi có
//! permission, hợp với mô hình kinh doanh của khách (`kernel.business_model`), kèm tiến độ.

use std::sync::{Arc, PoisonError, RwLock};

use axum::extract::State;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::access::{Permission, Principal};
use crate::entitlement::EntitlementsHandle;
use crate::error::Error;
use crate::i18n::{Language, Translations};
use crate::response::{Data, Problem};
use crate::tenancy::TenantContext;
use crate::tenancy::settings::{BUSINESS_MODEL, TenantSettings};

/// Phần khai của một bước thiết lập.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SetupInfo {
    /// `<module>.<bước>`, vd `content.about_page`.
    pub id: &'static str,
    /// Câu tiếng Anh (dịch qua `text:` của module).
    pub label: &'static str,
    /// Màn của sổ điều hướng làm bước này (`NavItem::id`).
    pub screen: &'static str,
    /// Mô hình kinh doanh áp dụng (`independent`, `multi_store`, `chain`, `marketplace`, `saas`);
    /// rỗng: mọi mô hình.
    pub models: &'static [&'static str],
    /// Chưa xong thì chưa bán được.
    pub blocking: bool,
    /// Thứ tự hiện (nhỏ trước).
    pub order: u16,
}

impl SetupInfo {
    pub const fn new(
        id: &'static str,
        label: &'static str,
        screen: &'static str,
        order: u16,
    ) -> Self {
        SetupInfo { id, label, screen, models: &[], blocking: false, order }
    }

    pub const fn models(mut self, models: &'static [&'static str]) -> Self {
        self.models = models;
        self
    }

    pub const fn blocking(mut self) -> Self {
        self.blocking = true;
        self
    }
}

/// Một bước thiết lập.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait SetupStep: Send + Sync {
    fn info(&self) -> SetupInfo;
    /// Permission cần để thấy (và làm) bước này.
    fn permission(&self) -> Permission;
    /// Đã xong chưa, tính từ dữ liệu của khách.
    async fn done(&self, tenant: &TenantContext) -> Result<bool, Error>;
}

type Entry = (&'static str, Arc<dyn SetupStep>);

/// Sổ bước thiết lập của mọi module. Clone rẻ.
#[derive(Clone, Default)]
pub struct SetupSteps {
    entries: Arc<RwLock<Vec<Entry>>>,
}

impl SetupSteps {
    pub(crate) fn register(
        &self,
        module: &'static str,
        step: Arc<dyn SetupStep>,
    ) -> Result<(), Error> {
        let id = step.info().id;
        if !id.starts_with(&format!("{module}.")) {
            return Err(Error::internal(format!(
                "setup step `{id}` of module `{module}` must be named `{module}.<step>`"
            )));
        }
        let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        if entries.iter().any(|(_, known)| known.info().id == id) {
            return Err(Error::internal(format!("setup step `{id}` is registered twice")));
        }
        entries.push((module, step));
        Ok(())
    }

    fn all(&self) -> Vec<Entry> {
        let mut all = self.entries.read().unwrap_or_else(PoisonError::into_inner).clone();
        all.sort_by_key(|(_, step)| (step.info().order, step.info().id));
        all
    }
}

/// One setup step.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct SetupStepView {
    #[schema(example = "content.about_page")]
    pub id: String,
    /// What to do, in the language of `Accept-Language`.
    pub label: String,
    /// Screen id of the navigation where it is done.
    pub screen: String,
    /// Selling cannot start before it is done.
    pub blocking: bool,
    /// Computed from the tenant's data.
    pub done: bool,
}

/// The setup checklist.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct SetupView {
    pub steps: Vec<SetupStepView>,
    pub done: u32,
    pub total: u32,
    /// Blocking steps not done yet.
    pub blocking_left: u32,
    /// Steps that did not answer in time or failed (counted as not done).
    pub incomplete: Vec<String>,
}

#[derive(Clone)]
pub(crate) struct SetupState {
    pub steps: SetupSteps,
    pub entitlements: EntitlementsHandle,
    pub translations: Translations,
    pub settings: Option<Arc<dyn TenantSettings>>,
}

/// Get the setup checklist.
///
/// Any signed-in user. The setup steps modules declare for the tenant's business model
/// (`kernel.business_model`), among those you have permission for, each `done` or not as computed
/// from the data — there is nothing to tick by hand.
#[utoipa::path(
    get,
    path = "/setup",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_setup",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The checklist.", body = Data<SetupView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
async fn get_setup(
    State(state): State<Arc<SetupState>>,
    tenant: TenantContext,
    principal: Principal,
    language: Language,
) -> Result<Data<SetupView>, Error> {
    let model = match &state.settings {
        Some(settings) => {
            settings.get(&tenant, &BUSINESS_MODEL).await?.map(|value| value.as_str().to_owned())
        }
        None => None,
    };
    let mut wanted = Vec::new();
    for (module, step) in state.steps.all() {
        let info = step.info();
        let fits = model
            .as_deref()
            .is_none_or(|model| info.models.is_empty() || info.models.contains(&model));
        if fits
            && state.entitlements.module_enabled(tenant.id(), module).await?
            && principal.require(&step.permission()).is_ok()
        {
            wanted.push(step);
        }
    }
    let answers = futures_util::future::join_all(wanted.iter().map(|step| async {
        tokio::time::timeout(crate::ops::work::SOURCE_TIMEOUT, step.done(&tenant)).await
    }))
    .await;
    let mut steps = Vec::new();
    let mut incomplete = Vec::new();
    for (step, answer) in wanted.iter().zip(answers) {
        let info = step.info();
        let done = match answer {
            Ok(Ok(done)) => done,
            Ok(Err(error)) => {
                tracing::warn!(step = info.id, error = %error, "setup step failed");
                incomplete.push(info.id.to_owned());
                false
            }
            Err(_) => {
                tracing::warn!(step = info.id, "setup step timed out");
                incomplete.push(info.id.to_owned());
                false
            }
        };
        steps.push(SetupStepView {
            id: info.id.to_owned(),
            label: state.translations.describe(info.label, language),
            screen: info.screen.to_owned(),
            blocking: info.blocking,
            done,
        });
    }
    let count = |keep: &dyn Fn(&SetupStepView) -> bool| {
        u32::try_from(steps.iter().filter(|step| keep(step)).count()).unwrap_or(u32::MAX)
    };
    let view = SetupView {
        done: count(&|step| step.done),
        total: count(&|_| true),
        blocking_left: count(&|step| step.blocking && !step.done),
        steps: Vec::new(),
        incomplete,
    };
    Ok(Data { data: SetupView { steps, ..view } })
}

pub(crate) fn routes(state: SetupState) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(get_setup)).with_state(Arc::new(state))
}
