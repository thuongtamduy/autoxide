//! Ba chế độ dòng tiền (bước 36 mục 4, D-187, D-216, D-218): tự thu, thu hộ qua đối tác, chia tại
//! cổng. `settle` ghi đúng bút toán cho từng việc (tiền vào, phí sàn, khấu trừ thay, hoàn, phí cổng)
//! theo chế độ của người bán; `ledger_holds` là cơ chế tạm giữ duy nhất — phần của người bán ở chế độ
//! thu hộ nằm ở `payable.seller_held` tới hết hạn đổi trả, job `ledger.release_holds` chuyển sang
//! `payable.seller_available`, chi trả lấy từ đó.
//!
//! Core không giữ tiền: ở chế độ thu hộ tiền nằm ở tài khoản thu hộ của đối tác có giấy phép
//! (`clearing.gateway`), ở chế độ tự thu tiền vào thẳng người bán; COD không bao giờ về tài khoản của
//! nhà vận hành sàn (`cod_remittance`).
//!
//! Thứ tự khoá trong transaction (giống mọi đường ghi, không deadlock): dòng tạm giữ → dòng sổ (chia
//! sẻ) → số dư theo `account_id` tăng dần.

use autoxide_kernel::time::{Date, Timestamp};
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::reference::{Reference, Submitted};
use autoxide_kernel::rows::{self, Bounds};
use autoxide_kernel::tenancy::{OrgId, OrgTree, ScopeExt, TenantContext, TenantId, nearest};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, QuerySelect, Set};
use serde_json::json;
use uuid::Uuid;

use crate::api::{
    AccountKey, CodRemittance, Collection, Counterparty, Entry, FlowEvent, FlowMode, FlowSettings,
    Hold, LineAccount, NewEntry, NewLine, SaleAmounts, Settled, Settlement,
};
use crate::entity::{book, entry, flow_setting, hold};
use crate::events;
use crate::permissions::{HOLDS_RELEASE, READ, SETTINGS_MANAGE};
use crate::posting::{draft_for, insufficient_balance, write_in};
use crate::service::{LedgerService, describe_entry, money, now, valid_id, valid_name};

pub(crate) const RELEASE_HOLDS: &str = "ledger.release_holds";

/// Số ngày tạm giữ khi chưa đặt (hạn đổi trả thường gặp).
const DEFAULT_HOLD_DAYS: i32 = 7;

// ---- Lỗi ----------------------------------------------------------------------------------------

pub(crate) fn flow_mode_unsupported(mode: FlowMode, provider: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "ledger.flow_mode_unsupported",
        "The money-flow mode `{mode}` does not support this, or the partner `{provider}` cannot do it.",
    )
    .with_param("mode", mode.as_str())
    .with_param("provider", provider)
}

pub(crate) fn hold_not_found() -> Error {
    Error::not_found("ledger.hold_not_found", "There is no such held amount.")
}

fn hold_state(status: &str) -> Error {
    Error::conflict("ledger.hold_state", "The held amount is `{status}`; this cannot be done now.")
        .with_param("status", status)
}

// ---- Mô tả --------------------------------------------------------------------------------------

pub(crate) fn describe_hold(row: &hold::Model) -> Result<Hold, Error> {
    Ok(Hold {
        id: row.id,
        book_id: row.book_id,
        org_id: row.org_id,
        seller_org_id: row.seller_org_id,
        amount: money(row.amount_minor, &row.currency)?,
        original: money(row.original_minor, &row.currency)?,
        release_at: row.release_at,
        status: row.status.clone(),
        source_module: row.source_module.clone(),
        source_id: row.source_id.clone(),
        release_entry_id: row.release_entry_id,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn describe_settings(org: OrgId, found: Option<flow_setting::Model>) -> FlowSettings {
    match found {
        Some(row) => FlowSettings {
            org_id: org.as_uuid(),
            source_org_id: Some(row.org_id),
            inherited: row.org_id != org.as_uuid(),
            mode: FlowMode::parse(&row.mode).unwrap_or(FlowMode::SelfCollect),
            payment_provider: row.payment_provider,
            payout_provider: row.payout_provider,
            hold_days: row.hold_days,
            cod_remittance: CodRemittance::parse(&row.cod_remittance)
                .unwrap_or(CodRemittance::PartnerCollection),
            updated_at: Some(row.updated_at),
        },
        None => FlowSettings {
            org_id: org.as_uuid(),
            source_org_id: None,
            inherited: false,
            mode: FlowMode::SelfCollect,
            payment_provider: None,
            payout_provider: None,
            hold_days: DEFAULT_HOLD_DAYS,
            cod_remittance: CodRemittance::PartnerCollection,
            updated_at: None,
        },
    }
}

/// Cài đặt dòng tiền gửi lên (REST / MCP).
#[derive(Debug, Clone)]
pub(crate) struct FlowSettingsInput {
    pub mode: String,
    pub payment_provider: Option<String>,
    pub payout_provider: Option<String>,
    pub hold_days: Option<i32>,
    pub cod_remittance: Option<String>,
}

/// Bộ lọc danh sách khoản tạm giữ.
#[derive(Debug, Clone, Default)]
pub(crate) struct HoldFilter {
    pub status: Option<String>,
    pub seller_org_id: Option<Uuid>,
    pub source_module: Option<String>,
    pub source_id: Option<String>,
}

fn seller_party(seller: OrgId) -> Counterparty {
    Counterparty::new("org", seller.as_uuid().to_string())
}

/// Tài khoản nhận tiền của đường thu `via` trong sổ `book`.
fn cash_account(book: OrgId, via: &Collection, amount: Money) -> AccountKey {
    match via {
        Collection::Gateway { provider } => {
            AccountKey::new(book, "clearing.gateway", amount.currency())
                .with_counterparty(Counterparty::new("partner", provider.clone()))
        }
        Collection::Cod { carrier } => {
            AccountKey::new(book, "clearing.carrier_cod", amount.currency())
                .with_counterparty(Counterparty::new("carrier", carrier.clone()))
        }
        Collection::BankTransfer { bank_account } => {
            AccountKey::new(book, "cash.bank", amount.currency())
                .with_counterparty(Counterparty::new("bank_account", bank_account.to_string()))
        }
    }
}

fn via_name(via: &Collection) -> &str {
    match via {
        Collection::Gateway { provider } => provider,
        Collection::Cod { .. } => "cod",
        Collection::BankTransfer { .. } => "bank_transfer",
    }
}

fn key(book: OrgId, purpose: &str, amount: Money) -> LineAccount {
    LineAccount::Key(AccountKey::new(book, purpose, amount.currency()))
}

fn seller_key(book: OrgId, purpose: &str, seller: OrgId, amount: Money) -> LineAccount {
    LineAccount::Key(
        AccountKey::new(book, purpose, amount.currency()).with_counterparty(seller_party(seller)),
    )
}

/// Thêm dòng có số tiền dương (dòng 0 bị bỏ: bút toán chỉ nhận số > 0).
fn push(lines: &mut Vec<NewLine>, line: NewLine) {
    if line.amount.is_positive() {
        lines.push(line);
    }
}

/// Kiểm `amounts` (dương, cùng tiền, gross = net + vat), thêm lỗi vào `problems`.
fn check_amounts(amounts: &SaleAmounts, field: &str, problems: &mut Vec<FieldError>) {
    let currency = amounts.gross.currency();
    if !amounts.gross.is_positive() || amounts.net.is_negative() || amounts.vat.is_negative() {
        problems.push(FieldError::new(
            format!("{field}.gross"),
            "range",
            "must be greater than zero, with net and VAT not negative",
        ));
    } else if amounts.net.currency() != currency
        || amounts.vat.currency() != currency
        || amounts.net.checked_add(amounts.vat).ok() != Some(amounts.gross)
    {
        problems.push(FieldError::new(
            format!("{field}.gross"),
            "sum",
            "must equal net plus VAT, in one currency",
        ));
    }
}

/// Việc đã kiểm, kèm số liệu đã tính ngoài transaction (khấu trừ thay).
struct Plan {
    mode: FlowMode,
    book: book::Model,
    book_org: OrgId,
    seller: OrgId,
    day: Date,
    hold_days: i32,
    /// Khấu trừ thay đã tính: (GTGT, TNCN).
    withheld: Option<(Money, Money)>,
    /// Thu COD thẳng về người bán: sàn không ghi gì.
    skip: bool,
}

/// Nguồn bị trừ khi tính phí / khấu trừ / hoàn ở chế độ thu hộ.
enum Debtor {
    /// Còn tạm giữ: trừ `payable.seller_held` và khoản tạm giữ.
    Held(Uuid),
    /// Đã hết hạn giữ: trừ `payable.seller_available`.
    Available,
    /// Tiền không qua sàn (COD về thẳng người bán): người bán nợ sàn.
    Receivable,
    /// Chia tại cổng: phần cổng chia cho sàn nằm ở `payable.seller_held`, không có khoản tạm giữ.
    Split,
}

impl LedgerService {
    // ---- Cài đặt dòng tiền ------------------------------------------------------------------

    /// Cài đặt hiệu lực của nút `org`: của nút gần nhất trên chuỗi tổ tiên (gồm chính nó) có đặt.
    pub(crate) async fn flow_settings(
        &self,
        tenant: &TenantContext,
        org: OrgId,
    ) -> Result<FlowSettings, Error> {
        let read = tenant.database().read();
        let mut chain = OrgTree::ancestors(read, tenant.id(), org).await?;
        if chain.is_empty() {
            chain.push(org);
        }
        let rows = flow_setting::Entity::find()
            .scoped(tenant)
            .filter(flow_setting::Column::OrgId.is_in(chain.iter().map(OrgId::as_uuid)))
            .all(read)
            .await?;
        let found =
            nearest(&chain, rows.into_iter().map(|row| (OrgId::from_uuid(row.org_id), row)));
        Ok(describe_settings(org, found.map(|(_, row)| row)))
    }

    /// Cài đặt của nút `org` cho người có `ledger.read` ở nút đó.
    pub(crate) async fn flow_settings_for(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: OrgId,
    ) -> Result<FlowSettings, Error> {
        principal.require(&READ)?;
        principal.require_at(tenant.database().read(), &READ, OrgTarget::new(org, None)).await?;
        self.flow_settings(tenant, org).await
    }

    /// Đặt cài đặt dòng tiền ở nút `org` (nút con kế thừa).
    pub(crate) async fn set_flow_settings(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: OrgId,
        input: FlowSettingsInput,
    ) -> Result<FlowSettings, Error> {
        principal.require(&SETTINGS_MANAGE)?;
        principal
            .require_at(tenant.database().read(), &SETTINGS_MANAGE, OrgTarget::new(org, None))
            .await?;
        let mut problems = Vec::new();
        let mode = FlowMode::parse(&input.mode);
        if mode.is_none() {
            problems.push(FieldError::new(
                "mode",
                "invalid",
                "must be `self_collect`, `partner_collect` or `split_at_gateway`",
            ));
        }
        let hold_days = input.hold_days.unwrap_or(DEFAULT_HOLD_DAYS);
        if !(0..=365).contains(&hold_days) {
            problems.push(FieldError::new("hold_days", "range", "must be 0 to 365"));
        }
        let cod = match input.cod_remittance.as_deref() {
            None => Some(CodRemittance::PartnerCollection),
            Some(raw) => CodRemittance::parse(raw),
        };
        if cod.is_none() {
            problems.push(FieldError::new(
                "cod_remittance",
                "invalid",
                "must be `partner_collection` or `seller_direct`",
            ));
        }
        let gateway = match input.payment_provider.as_deref() {
            Some(name) => match self.gateway(Some(name)) {
                Ok(found) => Some(found),
                Err(_) => {
                    problems.push(FieldError::new(
                        "payment_provider",
                        "unknown",
                        "is not a payment partner set up on this server",
                    ));
                    None
                }
            },
            None => None,
        };
        if let Some(name) = input.payout_provider.as_deref()
            && self.payout_gateway(name).is_err()
        {
            problems.push(FieldError::new(
                "payout_provider",
                "unknown",
                "is not a payout partner set up on this server",
            ));
        }
        match mode {
            Some(FlowMode::PartnerCollect) if input.payment_provider.is_none() => {
                problems.push(FieldError::new(
                    "payment_provider",
                    "required",
                    "is required in this mode",
                ));
            }
            Some(FlowMode::PartnerCollect) if input.payout_provider.is_none() => {
                problems.push(FieldError::new(
                    "payout_provider",
                    "required",
                    "is required in this mode",
                ));
            }
            Some(FlowMode::SplitAtGateway) if input.payment_provider.is_none() => {
                problems.push(FieldError::new(
                    "payment_provider",
                    "required",
                    "is required in this mode",
                ));
            }
            _ => {}
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let (Some(mode), Some(cod)) = (mode, cod) else {
            return Err(Error::internal("checked flow settings went missing"));
        };
        if let Some(gateway) = &gateway {
            let capabilities = gateway.capabilities();
            let fits = match mode {
                FlowMode::SelfCollect => true,
                FlowMode::PartnerCollect => capabilities.collection_on_behalf,
                FlowMode::SplitAtGateway => capabilities.split,
            };
            if !fits {
                return Err(flow_mode_unsupported(mode, gateway.provider()));
            }
        }
        let database = tenant.database();
        let input = &input;
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                principal
                    .require_write_at(&transaction, &SETTINGS_MANAGE, OrgTarget::new(org, None))
                    .await?;
                let at = now();
                flow_setting::Entity::insert(flow_setting::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    org_id: Set(org.as_uuid()),
                    mode: Set(mode.as_str().to_owned()),
                    payment_provider: Set(input.payment_provider.clone()),
                    payout_provider: Set(input.payout_provider.clone()),
                    hold_days: Set(hold_days),
                    cod_remittance: Set(cod.as_str().to_owned()),
                    updated_by: Set(Some(principal.user_id())),
                    created_at: Set(at),
                    updated_at: Set(at),
                })
                .on_conflict(
                    OnConflict::columns([
                        flow_setting::Column::TenantId,
                        flow_setting::Column::OrgId,
                    ])
                    .update_columns([
                        flow_setting::Column::Mode,
                        flow_setting::Column::PaymentProvider,
                        flow_setting::Column::PayoutProvider,
                        flow_setting::Column::HoldDays,
                        flow_setting::Column::CodRemittance,
                        flow_setting::Column::UpdatedBy,
                        flow_setting::Column::UpdatedAt,
                    ])
                    .to_owned(),
                )
                .exec_without_returning(&transaction)
                .await?;
                transaction.commit().await?;
                Ok(())
            })
            .await?;
        self.flow_settings(tenant, org).await
    }

    // ---- settle -----------------------------------------------------------------------------

    /// Một việc của dòng tiền (`dyn Ledger`).
    pub(crate) async fn settle(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        input: Settlement,
    ) -> Result<Submitted<Settled>, Error> {
        let mut problems = Vec::new();
        let reference = Reference::checked(&input.reference, "reference", &mut problems);
        if !valid_name(&input.source_module) {
            problems.push(FieldError::new("source_module", "format", "must be a module name"));
        }
        if !valid_id(&input.source_id) {
            problems.push(FieldError::new(
                "source_id",
                "format",
                "must be 1 to 100 visible ASCII characters",
            ));
        }
        match &input.event {
            FlowEvent::Collected { amounts, operator_share, .. } => {
                check_amounts(amounts, "amounts", &mut problems);
                if operator_share.is_some_and(|share| {
                    !share.is_positive()
                        || share.currency() != amounts.gross.currency()
                        || share.minor() > amounts.gross.minor()
                }) {
                    problems.push(FieldError::new(
                        "operator_share",
                        "range",
                        "must be greater than zero and at most the gross amount",
                    ));
                }
            }
            FlowEvent::Refunded { amounts, .. } => check_amounts(amounts, "amounts", &mut problems),
            FlowEvent::FeeCharged { fee, vat } => {
                if !fee.is_positive()
                    || vat.is_negative()
                    || vat.currency() != fee.currency()
                    || vat.minor() > fee.minor()
                {
                    problems.push(FieldError::new(
                        "fee",
                        "range",
                        "must be greater than zero and include its VAT",
                    ));
                }
            }
            FlowEvent::TaxWithheld { revenue, business_line } => {
                if !revenue.is_positive() {
                    problems.push(FieldError::new("revenue", "range", "must be greater than zero"));
                }
                if !["goods", "services", "production_transport", "other"]
                    .contains(&business_line.as_str())
                {
                    problems.push(FieldError::new(
                        "business_line",
                        "invalid",
                        "must be `goods`, `services`, `production_transport` or `other`",
                    ));
                }
            }
            FlowEvent::GatewayFeeCharged { provider, fee } => {
                if !fee.is_positive() {
                    problems.push(FieldError::new("fee", "range", "must be greater than zero"));
                }
                if !valid_name(provider) {
                    problems.push(FieldError::new("provider", "format", "must be a partner name"));
                }
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let Some(reference) = reference else {
            return Err(Error::internal("a checked reference went missing"));
        };
        let plan = self.plan(tenant, &input).await?;
        let (database, outbox, input, plan, reference) =
            (tenant.database(), &self.outbox, &input, &plan, &reference);
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let outcome =
                    settle_in(&transaction, tenant, actor, input, plan, reference, outbox).await?;
                transaction.commit().await?;
                Ok(outcome)
            })
            .await
    }

    /// Đọc chế độ, chọn sổ, kiểm việc có hợp với chế độ / cổng không, tính khấu trừ thay — mọi thứ
    /// cần I/O, ngoài transaction ghi.
    async fn plan(&self, tenant: &TenantContext, input: &Settlement) -> Result<Plan, Error> {
        let settings = self.flow_settings(tenant, input.seller).await?;
        let mode = settings.mode;
        let book_org = match (mode, settings.source_org_id) {
            (FlowMode::SelfCollect, _) | (_, None) => input.seller,
            (_, Some(operator)) => OrgId::from_uuid(operator),
        };
        let mut skip = false;
        let mut withheld = None;
        match &input.event {
            FlowEvent::Collected { via, operator_share, .. } => {
                self.check_collection(mode, via, *operator_share)?;
                skip = mode == FlowMode::PartnerCollect
                    && matches!(via, Collection::Cod { .. })
                    && settings.cod_remittance == CodRemittance::SellerDirect;
            }
            FlowEvent::Refunded { via, .. } => {
                self.check_collection(mode, via, None)?;
            }
            FlowEvent::FeeCharged { .. } if mode == FlowMode::SelfCollect => {
                // Tự thu: sàn thu phí bằng hoá đơn / thuê bao, không trừ trên sổ của người bán.
                return Err(flow_mode_unsupported(mode, "-"));
            }
            FlowEvent::TaxWithheld { revenue, business_line } => {
                if mode == FlowMode::SelfCollect {
                    // Nền tảng không có chức năng thanh toán thì không khấu trừ thay (D-187).
                    return Err(flow_mode_unsupported(mode, "-"));
                }
                withheld = self.withholding(tenant, input, *revenue, business_line).await?;
                skip = withheld.is_none();
            }
            FlowEvent::FeeCharged { .. } | FlowEvent::GatewayFeeCharged { .. } => {}
        }
        let book = self.ensure_book(tenant, book_org).await?;
        let day = match input.effective_date {
            Some(day) => day,
            None => self.today(tenant, book_org).await,
        };
        Ok(Plan {
            mode,
            book,
            book_org,
            seller: input.seller,
            day,
            hold_days: settings.hold_days,
            withheld,
            skip,
        })
    }

    fn check_collection(
        &self,
        mode: FlowMode,
        via: &Collection,
        operator_share: Option<Money>,
    ) -> Result<(), Error> {
        match (mode, via) {
            (FlowMode::SelfCollect, _) => Ok(()),
            (_, Collection::BankTransfer { .. }) => Err(flow_mode_unsupported(mode, via_name(via))),
            (FlowMode::SplitAtGateway, Collection::Cod { .. }) => {
                Err(flow_mode_unsupported(mode, via_name(via)))
            }
            // COD ở chế độ thu hộ: về tài khoản thu hộ của đối tác hay thẳng người bán (`cod`), không
            // bao giờ về tài khoản của sàn.
            (FlowMode::PartnerCollect, Collection::Cod { .. }) => Ok(()),
            (_, Collection::Gateway { provider }) => {
                let gateway = self.gateway(Some(provider))?;
                let capabilities = gateway.capabilities();
                let fits = if mode == FlowMode::PartnerCollect {
                    capabilities.collection_on_behalf
                } else {
                    capabilities.split
                };
                if !fits {
                    return Err(flow_mode_unsupported(mode, gateway.provider()));
                }
                if mode == FlowMode::SplitAtGateway && operator_share.is_none() {
                    return Err(Error::validation(vec![FieldError::new(
                        "operator_share",
                        "required",
                        "is required when the partner splits the payment",
                    )]));
                }
                Ok(())
            }
        }
    }

    /// Khấu trừ thay theo hồ sơ thuế của người bán (NĐ 117/2025/NĐ-CP — cần đối chiếu): chỉ hộ / cá
    /// nhân kinh doanh (`household`); doanh nghiệp tự kê khai → `None`.
    async fn withholding(
        &self,
        tenant: &TenantContext,
        input: &Settlement,
        revenue: Money,
        business_line: &str,
    ) -> Result<Option<(Money, Money)>, Error> {
        use autoxide_tax::api::WithholdingRequest;
        let Some(engine) = &self.tax else {
            return Err(Error::internal("withholding needs the `tax` module"));
        };
        let Some(profile) = engine.profile(tenant, input.seller).await? else {
            return Ok(None);
        };
        if profile.regime != "household" {
            return Ok(None);
        }
        let on = match input.effective_date {
            Some(day) => day,
            None => self.today(tenant, input.seller).await,
        };
        let request = WithholdingRequest::new(
            profile.jurisdiction,
            on,
            "marketplace",
            profile.regime,
            business_line,
            revenue,
        );
        let quote = engine.withholding(tenant, request).await?;
        let sum = |kind: &str| {
            Money::sum(
                revenue.currency(),
                quote
                    .components
                    .iter()
                    .filter(|part| part.tax_kind == kind)
                    .map(|part| part.amount),
            )
        };
        let (vat, pit) = (sum("vat")?, sum("pit")?);
        Ok((vat.is_positive() || pit.is_positive()).then_some((vat, pit)))
    }

    // ---- Tạm giữ ----------------------------------------------------------------------------

    pub(crate) async fn hold_status(
        &self,
        tenant: &TenantContext,
        source_module: &str,
        source_id: &str,
    ) -> Result<Option<Hold>, Error> {
        let found = hold::Entity::find()
            .scoped(tenant)
            .filter(hold::Column::SourceModule.eq(source_module))
            .filter(hold::Column::SourceId.eq(source_id))
            .one(tenant.database().read())
            .await?;
        found.as_ref().map(describe_hold).transpose()
    }

    pub(crate) async fn list_holds(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: HoldFilter,
        page: &PageRequest,
    ) -> Result<Page<hold::Model>, Error> {
        let condition = principal.scope_condition(&READ, hold::Column::OrgId, None)?;
        let mut select = hold::Entity::find().scoped(tenant).filter(condition);
        if let Some(status) = filter.status {
            select = select.filter(hold::Column::Status.eq(status));
        }
        if let Some(seller) = filter.seller_org_id {
            select = select.filter(hold::Column::SellerOrgId.eq(seller));
        }
        if let Some(module) = filter.source_module {
            select = select.filter(hold::Column::SourceModule.eq(module));
        }
        if let Some(id) = filter.source_id {
            select = select.filter(hold::Column::SourceId.eq(id));
        }
        fetch_page(select, hold::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Nhả sớm khoản tạm giữ `id` (REST / MCP): cần `ledger.holds_release` ở nút của sổ.
    pub(crate) async fn release_hold_manual(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Hold, Error> {
        principal.require(&HOLDS_RELEASE)?;
        let read = tenant.database().read();
        let found = hold::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(hold_not_found)?;
        principal
            .require_at(read, &HOLDS_RELEASE, OrgTarget::new(OrgId::from_uuid(found.org_id), None))
            .await
            .map_err(
                |error| if error.code() == "org_out_of_scope" { hold_not_found() } else { error },
            )?;
        self.release_hold(tenant, Actor::from(principal), id).await
    }

    /// Chuyển khoản tạm giữ `id` sang số dư khả dụng của người bán. Đã nhả → trả như cũ (job và
    /// người cùng nhả: một bút toán).
    pub(crate) async fn release_hold(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
    ) -> Result<Hold, Error> {
        let database = tenant.database();
        let current = hold::Entity::find_by_id(id)
            .scoped(tenant)
            .one(database.read())
            .await?
            .ok_or_else(hold_not_found)?;
        let book_org = OrgId::from_uuid(current.org_id);
        let day = self.today(tenant, book_org).await;
        let book = self.ensure_book(tenant, book_org).await?;
        let (outbox, book) = (&self.outbox, &book);
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let row = rows::lock_one(&transaction, hold::Entity::find_by_id(id).scoped(tenant))
                    .await?
                    .ok_or_else(hold_not_found)?;
                if row.status != "held" {
                    return describe_hold(&row);
                }
                let mut entry_id = None;
                if row.amount_minor > 0 {
                    let amount = money(row.amount_minor, &row.currency)?;
                    let seller = OrgId::from_uuid(row.seller_org_id);
                    let entry = NewEntry::new(
                        book_org,
                        "ledger",
                        "hold_release",
                        format!("hold:{id}:released"),
                        "Return window ended: seller money becomes available",
                        vec![
                            NewLine::debit(seller_key(book_org, "payable.seller_held", seller, amount), amount),
                            NewLine::credit(
                                seller_key(book_org, "payable.seller_available", seller, amount),
                                amount,
                            ),
                        ],
                    )
                    .source_id(row.source_id.clone());
                    let draft = draft_for(entry, book.clone(), day, None, None, None)?;
                    let written = write_in(&transaction, tenant, actor, &draft, outbox).await?;
                    entry_id = Some(written.into_inner().id);
                }
                let mut active: hold::ActiveModel = row.into();
                active.status = Set("released".to_owned());
                active.release_entry_id = Set(entry_id);
                active.updated_at = Set(now());
                let saved = hold::Entity::update(active).exec(&transaction).await?;
                let event = events::HoldReleased(json!({
                    "hold_id": saved.id, "org_id": saved.org_id, "seller_org_id": saved.seller_org_id,
                    "amount": money(saved.amount_minor, &saved.currency)?,
                    "source_module": saved.source_module, "source_id": saved.source_id,
                }));
                outbox
                    .record(&transaction, tenant, actor, Some(format!("ledger.hold:{id}")), &event)
                    .await?;
                transaction.commit().await?;
                describe_hold(&saved)
            })
            .await
    }

    /// Khoản tạm giữ hết hạn trên `database` → nhả (lịch 5 phút). Trả số đã nhả. Lỗi của một khoản
    /// không chặn khoản khác.
    pub(crate) async fn release_due_holds(
        &self,
        database: &autoxide_kernel::database::Database,
    ) -> Result<u64, Error> {
        let due: Vec<(Uuid, Uuid)> = hold::Entity::find()
            .select_only()
            .column(hold::Column::TenantId)
            .column(hold::Column::Id)
            .filter(hold::Column::Status.eq("held"))
            .filter(hold::Column::ReleaseAt.lte(Timestamp::now()))
            .limit(500)
            .into_tuple()
            .all(database.read())
            .await?;
        let mut released = 0;
        for (tenant, id) in due {
            let tenant = self.databases.tenant(TenantId::from_uuid(tenant)).await?;
            match self.release_hold(&tenant, Actor::system(), id).await {
                Ok(_) => released += 1,
                Err(error) => {
                    tracing::warn!(tenant_id = %tenant.id(), hold_id = %id, error = %error, "cannot release a hold");
                }
            }
        }
        Ok(released)
    }
}

/// Ghi một việc của dòng tiền trong `transaction` (chạy lại được: mọi thứ đọc lại).
async fn settle_in(
    transaction: &DatabaseTransaction,
    tenant: &TenantContext,
    actor: Actor,
    input: &Settlement,
    plan: &Plan,
    reference: &Reference,
    outbox: &autoxide_kernel::event::Outbox,
) -> Result<Submitted<Settled>, Error> {
    let current_hold = || async {
        hold::Entity::find()
            .scoped(tenant)
            .filter(hold::Column::SourceModule.eq(input.source_module.as_str()))
            .filter(hold::Column::SourceId.eq(input.source_id.as_str()))
            .one(transaction)
            .await
    };
    let settled = |entry: Option<Entry>, hold: Option<hold::Model>| -> Result<Settled, Error> {
        Ok(Settled {
            mode: plan.mode,
            book_org: plan.book_org,
            entry,
            hold: hold.as_ref().map(describe_hold).transpose()?,
        })
    };
    // Gửi lại: bút toán đã có cùng `reference` → trả lại, không tính lại (trạng thái tạm giữ có thể
    // đã đổi từ lần đầu).
    let stored = entry::Entity::find()
        .scoped(tenant)
        .filter(entry::Column::SourceModule.eq(input.source_module.as_str()))
        .filter(entry::Column::Reference.eq(reference.as_str()))
        .one(transaction)
        .await?;
    if let Some(stored) = stored {
        let described = describe_entry(transaction, tenant, stored).await?;
        return Ok(Submitted::Replayed(settled(Some(described), current_hold().await?)?));
    }
    if plan.skip {
        return Ok(Submitted::Created(settled(None, current_hold().await?)?));
    }
    let (book, seller) = (plan.book_org, plan.seller);
    let mut lines = Vec::new();
    // Thay đổi khoản tạm giữ sau khi ghi bút toán: (dòng đã khoá, số giảm) hay tạo mới.
    let mut reduce: Option<(Uuid, i64)> = None;
    let mut create_hold: Option<Money> = None;
    let (kind, description) = match &input.event {
        FlowEvent::Collected { amounts, via, operator_share } => {
            let cash = |amount: Money| {
                NewLine::debit(LineAccount::Key(cash_account(book, via, amount)), amount)
            };
            match plan.mode {
                FlowMode::SelfCollect => {
                    push(&mut lines, cash(amounts.gross));
                    push(
                        &mut lines,
                        NewLine::credit(key(book, "revenue.sales", amounts.net), amounts.net),
                    );
                    push(
                        &mut lines,
                        NewLine::credit(key(book, "tax.vat_output", amounts.vat), amounts.vat),
                    );
                }
                FlowMode::PartnerCollect => {
                    if let Some(found) = current_hold().await? {
                        return Err(hold_state(&found.status));
                    }
                    push(&mut lines, cash(amounts.gross));
                    push(
                        &mut lines,
                        NewLine::credit(
                            seller_key(book, "payable.seller_held", seller, amounts.gross),
                            amounts.gross,
                        ),
                    );
                    create_hold = Some(amounts.gross);
                }
                FlowMode::SplitAtGateway => {
                    let share = operator_share.unwrap_or(amounts.gross);
                    push(&mut lines, cash(share));
                    push(
                        &mut lines,
                        NewLine::credit(
                            seller_key(book, "payable.seller_held", seller, share),
                            share,
                        ),
                    );
                }
            }
            ("sale_collected", "Sale money collected")
        }
        FlowEvent::FeeCharged { fee, vat } => {
            let net = fee.checked_sub(*vat)?;
            let debtor = debtor(transaction, plan, current_hold().await?, *fee).await?;
            push(&mut lines, NewLine::debit(debtor_account(&debtor, book, seller, *fee), *fee));
            push(&mut lines, NewLine::credit(key(book, "revenue.platform_fee", net), net));
            push(&mut lines, NewLine::credit(key(book, "tax.vat_output", *vat), *vat));
            if let Debtor::Held(row) = debtor {
                reduce = Some((row, fee.minor()));
            }
            ("platform_fee", "Marketplace fee charged to the seller")
        }
        FlowEvent::TaxWithheld { .. } => {
            let Some((vat, pit)) = plan.withheld else {
                return Ok(Submitted::Created(settled(None, current_hold().await?)?));
            };
            let total = vat.checked_add(pit)?;
            let debtor = debtor(transaction, plan, current_hold().await?, total).await?;
            push(&mut lines, NewLine::debit(debtor_account(&debtor, book, seller, total), total));
            push(&mut lines, NewLine::credit(key(book, "tax.withheld_vat", vat), vat));
            push(&mut lines, NewLine::credit(key(book, "tax.withheld_pit", pit), pit));
            if let Debtor::Held(row) = debtor {
                reduce = Some((row, total.minor()));
            }
            ("tax_withheld", "Seller tax withheld on behalf of the tax authority")
        }
        FlowEvent::Refunded { amounts, via } => {
            let gross = amounts.gross;
            let paid_back =
                NewLine::credit(LineAccount::Key(cash_account(book, via, gross)), gross);
            match plan.mode {
                FlowMode::SelfCollect => {
                    push(
                        &mut lines,
                        NewLine::debit(key(book, "revenue.sales", amounts.net), amounts.net),
                    );
                    push(
                        &mut lines,
                        NewLine::debit(key(book, "tax.vat_output", amounts.vat), amounts.vat),
                    );
                }
                FlowMode::PartnerCollect => {
                    let debtor = debtor(transaction, plan, current_hold().await?, gross).await?;
                    if matches!(debtor, Debtor::Receivable) {
                        return Err(flow_mode_unsupported(plan.mode, via_name(via)));
                    }
                    push(
                        &mut lines,
                        NewLine::debit(debtor_account(&debtor, book, seller, gross), gross),
                    );
                    if let Debtor::Held(row) = debtor {
                        reduce = Some((row, gross.minor()));
                    }
                }
                FlowMode::SplitAtGateway => {
                    push(
                        &mut lines,
                        NewLine::debit(
                            seller_key(book, "payable.seller_held", seller, gross),
                            gross,
                        ),
                    );
                }
            }
            push(&mut lines, paid_back);
            ("refund", "Sale refunded to the buyer")
        }
        FlowEvent::GatewayFeeCharged { provider, fee } => {
            let clearing = AccountKey::new(book, "clearing.gateway", fee.currency())
                .with_counterparty(Counterparty::new("partner", provider.clone()));
            push(&mut lines, NewLine::debit(key(book, "expense.gateway_fee", *fee), *fee));
            push(&mut lines, NewLine::credit(LineAccount::Key(clearing), *fee));
            ("gateway_fee", "Payment partner fee")
        }
    };
    let mut entry = NewEntry::new(
        book,
        input.source_module.clone(),
        kind,
        reference.as_str(),
        description,
        lines,
    )
    .source_id(input.source_id.clone());
    entry.effective_date = Some(plan.day);
    let draft = draft_for(entry, plan.book.clone(), plan.day, None, None, None)?;
    let written = write_in(transaction, tenant, actor, &draft, outbox).await?.into_inner();
    if let Some((hold_id, minor)) = reduce {
        let changed = rows::adjust::<hold::Entity, _>(
            transaction,
            hold::Column::AmountMinor,
            hold::Column::Id.eq(hold_id),
            -minor,
            Bounds::NON_NEGATIVE,
        )
        .await?;
        if changed == 0 {
            return Err(insufficient_balance());
        }
        hold::Entity::update_many()
            .scoped(tenant)
            .col_expr(hold::Column::UpdatedAt, now().into())
            .filter(hold::Column::Id.eq(hold_id))
            .exec(transaction)
            .await?;
    }
    if let Some(amount) = create_hold {
        let at = now();
        let release_at = at + SignedDuration::from_hours(24 * (i64::from(plan.hold_days)));
        hold::Entity::insert(hold::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            book_id: Set(plan.book.id),
            org_id: Set(plan.book.org_id),
            seller_org_id: Set(seller.as_uuid()),
            amount_minor: Set(amount.minor()),
            original_minor: Set(amount.minor()),
            currency: Set(amount.currency().code().to_owned()),
            release_at: Set(release_at),
            status: Set("held".to_owned()),
            source_module: Set(input.source_module.clone()),
            source_id: Set(input.source_id.clone()),
            release_entry_id: Set(None),
            created_at: Set(at),
            updated_at: Set(at),
        })
        .exec_without_returning(transaction)
        .await?;
    }
    Ok(Submitted::Created(settled(Some(written), current_hold().await?)?))
}

/// Bên bị trừ phí / khấu trừ / hoàn: khoản tạm giữ (đã khoá dòng) nếu còn giữ và đủ, số dư khả dụng
/// nếu đã nhả, người bán nợ sàn nếu tiền không qua sàn.
async fn debtor(
    transaction: &DatabaseTransaction,
    plan: &Plan,
    found: Option<hold::Model>,
    amount: Money,
) -> Result<Debtor, Error> {
    if plan.mode == FlowMode::SplitAtGateway {
        // Phần cổng chia cho sàn nằm ở `payable.seller_held` (không có khoản tạm giữ).
        return Ok(Debtor::Split);
    }
    let Some(found) = found else { return Ok(Debtor::Receivable) };
    // Lọc theo khách của dòng vừa đọc (hàm không nhận `TenantContext`).
    let locked = rows::lock_one(
        transaction,
        hold::Entity::find_by_id(found.id).filter(hold::Column::TenantId.eq(found.tenant_id)),
    )
    .await?
    .ok_or_else(hold_not_found)?;
    if locked.status != "held" {
        return Ok(Debtor::Available);
    }
    if locked.amount_minor < amount.minor() {
        return Err(insufficient_balance());
    }
    Ok(Debtor::Held(locked.id))
}

fn debtor_account(debtor: &Debtor, book: OrgId, seller: OrgId, amount: Money) -> LineAccount {
    match debtor {
        Debtor::Held(_) | Debtor::Split => seller_key(book, "payable.seller_held", seller, amount),
        Debtor::Available => seller_key(book, "payable.seller_available", seller, amount),
        Debtor::Receivable => seller_key(book, "receivable.seller", seller, amount),
    }
}

/// Lịch `ledger.release_holds` (5 phút, mọi database).
pub(crate) struct ReleaseHoldsJob(pub Arc<LedgerService>);

#[async_trait::async_trait]
impl JobHandler for ReleaseHoldsJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.release_due_holds(&job.database).await.map(|_| ())
    }
}
