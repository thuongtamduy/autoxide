//! Thu hộ khi giao (COD, bước 44 mục 5). Hãng thu tiền và chuyển trả theo hợp đồng với hãng; core
//! chỉ ghi nghĩa vụ và đối soát, không nhận hay giữ tiền (D-187, D-216).
//!
//! - Giao xong có COD → job `shipping.post_cod`: Nợ `clearing.carrier_cod` (đối tượng: tài khoản
//!   hãng) / Có `receivable.customer` (đối tượng: đơn), mã `shipping.cod:<vận đơn>`.
//! - Nhập bảng kê của hãng (CSV chung `tracking_number,cod_amount,fee` hay API của hãng), khớp theo
//!   mã vận đơn. Dòng khớp → job ghi Nợ `shipping.settlement_in_transit` (số thực trả) + Nợ
//!   `expense.shipping` (phí hãng trừ) / Có `clearing.carrier_cod`, mã `shipping.cod_settle:<dòng>`;
//!   vận đơn `settled`, event `shipping.cod_settled` mang `seller_org_id` và nơi nhận tiền.
//! - Dòng lệch (tiền khác, mã lạ, chưa giao) → chênh lệch `open` + `shipping.cod_mismatch`. Giao xong
//!   quá `COD_SETTLEMENT_DAYS` mà chưa vào bảng kê → chênh lệch `overdue` (lịch hằng ngày).
//! - Giải quyết chênh lệch cần `resolution_code` của danh sách lý do `shipping.cod_resolution` và
//!   ghi chú; `written_off` ghi bút toán xoá khoản phải thu (`expense.cod_write_off`).
//!
//! Số tiền COD lấy từ yêu cầu giao (của `orders`), không từ webhook hay bảng kê của hãng.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::EnqueueOptions;
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::secret::sha256;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use autoxide_ledger::api::Side;
use chrono::{Duration, NaiveDate};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, Set,
};
use uuid::Uuid;

use crate::api::{CodStatement, CodStatementLine, ShipmentStatus, events as names};
use crate::carriers::within;
use crate::entity::{cod_discrepancy, cod_statement, cod_statement_line, shipment};
use crate::errors;
use crate::events::RecordNamed;
use crate::jobs::SettleCod;
use crate::ledger::Account;
use crate::service::{ShippingService, now};
use crate::shipments::status_of;

/// Danh sách lý do giải quyết chênh lệch (D-223).
pub(crate) const RESOLUTION_LIST: &str = "shipping.cod_resolution";
/// Số dòng tối đa của một bảng kê.
const MAX_LINES: usize = 20_000;

/// Đọc bảng kê CSV chung: cột `tracking_number`, `cod_amount`, `fee` (đơn vị nhỏ nhất).
pub(crate) fn parse_csv(
    text: &str,
    currency: autoxide_kernel::money::Currency,
) -> Result<Vec<CodStatementLine>, Error> {
    let rows = autoxide_kernel::csv::read(text, MAX_LINES + 1).map_err(|error| {
        Error::validation(vec![
            FieldError::new("csv", "csv", "is not valid CSV: {problem}")
                .with_param("problem", error),
        ])
    })?;
    let mut rows = rows.into_iter();
    let header: Vec<String> = rows
        .next()
        .unwrap_or_default()
        .into_iter()
        .map(|cell| cell.trim().to_ascii_lowercase())
        .collect();
    let column = |name: &str| header.iter().position(|cell| cell == name);
    let (Some(tracking), Some(cod), Some(fee)) =
        (column("tracking_number"), column("cod_amount"), column("fee"))
    else {
        return Err(Error::validation(vec![FieldError::new(
            "csv",
            "columns",
            "needs the columns `tracking_number`, `cod_amount` and `fee`",
        )]));
    };
    let mut lines = Vec::new();
    let mut problems = Vec::new();
    for (index, row) in rows.enumerate() {
        let cell = |at: usize| row.get(at).map(|value| value.trim().to_owned()).unwrap_or_default();
        let number = |at: usize| cell(at).replace([',', '.', ' '], "").parse::<i64>();
        match (cell(tracking), number(cod), number(fee)) {
            (code, Ok(cod), Ok(fee)) if !code.is_empty() && cod >= 0 && fee >= 0 => {
                lines.push(CodStatementLine::new(
                    code,
                    Money::new(cod, currency),
                    Money::new(fee, currency),
                ));
            }
            _ => problems.push(FieldError::new(
                format!("csv[{}]", index + 2),
                "row",
                "needs a tracking number and two amounts of 0 or more",
            )),
        }
    }
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    Ok(lines)
}

fn content_hash(statement: &CodStatement) -> String {
    let mut text =
        format!("{}|{}|{}", statement.reference, statement.period_from, statement.period_to);
    for line in &statement.lines {
        text.push_str(&format!(
            "|{}:{}:{}",
            line.tracking_number,
            line.cod.minor(),
            line.fee.minor()
        ));
    }
    sha256(&text)
}

impl ShippingService {
    /// Ghi sổ cái khi giao xong có COD (job). Chạy lại không ghi hai lần (mã tham chiếu).
    pub(crate) async fn post_cod(
        &self,
        tenant: &TenantContext,
        shipment_id: Uuid,
    ) -> Result<(), Error> {
        let Some(ledger) = &self.ledger else {
            tracing::warn!(shipment_id = %shipment_id, "cash on delivery collected but the ledger module is not installed");
            return Ok(());
        };
        let row =
            crate::shipments::find_shipment(tenant.database().read(), tenant, shipment_id).await?;
        let (Some(minor), Some(account)) = (row.cod_minor, row.carrier_account_id) else {
            return Ok(());
        };
        let amount = Money::from_columns(minor, &row.currency)?;
        crate::ledger::post(
            ledger.as_ref(),
            tenant,
            OrgId::from_uuid(row.org_id),
            "cod_collected",
            format!("shipping.cod:{shipment_id}"),
            "Cash on delivery collected by the carrier",
            shipment_id,
            vec![
                (Side::Debit, Account::CarrierCod(account), amount),
                (Side::Credit, Account::BuyerReceivable(row.order_id), amount),
            ],
        )
        .await?;
        Ok(())
    }

    /// Nhập một bảng kê (CSV hay API của hãng). Nhập lại cùng nội dung → bảng kê cũ; cùng mã khác
    /// nội dung → `shipping.statement_duplicate`.
    pub(crate) async fn import_statement(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        account_id: Uuid,
        statement: CodStatement,
    ) -> Result<(cod_statement::Model, bool), Error> {
        let account =
            crate::carriers::find_account(tenant.database().read(), tenant, account_id).await?;
        let hash = content_hash(&statement);
        if let Some(existing) = cod_statement::Entity::find()
            .filter(cod_statement::Column::CarrierAccountId.eq(account_id))
            .filter(cod_statement::Column::StatementReference.eq(&statement.reference))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
        {
            return if existing.content_hash == hash {
                Ok((existing, false))
            } else {
                Err(errors::statement_duplicate(&statement.reference))
            };
        }
        let currency = statement
            .lines
            .first()
            .map_or(autoxide_kernel::money::Currency::VND, |line| line.cod.currency());
        let total_cod: i64 = statement.lines.iter().map(|line| line.cod.minor()).sum();
        let total_fee: i64 = statement.lines.iter().map(|line| line.fee.minor()).sum();
        let at = now();
        let id = Uuid::now_v7();
        let transaction = tenant.database().begin_write().await?;
        let mut mismatches = 0usize;
        let mut settled = Vec::new();
        for (position, line) in statement.lines.iter().enumerate() {
            let found = shipment::Entity::find()
                .filter(shipment::Column::CarrierCode.eq(&account.carrier_code))
                .filter(shipment::Column::TrackingNumber.eq(&line.tracking_number))
                .scoped(tenant)
                .one(&transaction)
                .await?;
            let kind = match &found {
                None => "unknown_tracking",
                Some(row) if !matches!(status_of(row), ShipmentStatus::Delivered) => {
                    "not_delivered"
                }
                Some(row) if row.cod_minor.unwrap_or(0) != line.cod.minor() => "amount_mismatch",
                Some(_) => "matched",
            };
            let line_id = Uuid::now_v7();
            cod_statement_line::ActiveModel {
                id: Set(line_id),
                tenant_id: Set(tenant.id().as_uuid()),
                statement_id: Set(id),
                position: Set(i32::try_from(position + 1).unwrap_or(i32::MAX)),
                tracking_number: Set(line.tracking_number.chars().take(100).collect()),
                cod_minor: Set(line.cod.minor()),
                fee_minor: Set(line.fee.minor()),
                shipment_id: Set(found.as_ref().map(|row| row.id)),
                match_kind: Set(kind.to_owned()),
            }
            .insert(&transaction)
            .await?;
            if kind == "matched" {
                if let Some(row) = &found {
                    settled.push((line_id, row.clone()));
                }
            } else {
                mismatches += 1;
                let expected = found.as_ref().and_then(|row| row.cod_minor).unwrap_or(0);
                self.open_discrepancy(
                    &transaction,
                    tenant,
                    format!("line:{line_id}"),
                    Some(line_id),
                    found.as_ref(),
                    kind,
                    expected,
                    line.cod.minor(),
                    currency.code(),
                )
                .await?;
            }
        }
        let row = cod_statement::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            carrier_account_id: Set(account_id),
            statement_reference: Set(statement.reference.chars().take(100).collect()),
            content_hash: Set(hash),
            period_from: Set(statement.period_from),
            period_to: Set(statement.period_to),
            total_cod_minor: Set(total_cod),
            total_fee_minor: Set(total_fee),
            net_paid_minor: Set(total_cod - total_fee),
            currency: Set(currency.code().to_owned()),
            status: Set(if mismatches > 0 { "has_mismatch" } else { "reconciled" }.to_owned()),
            imported_by: Set(Some(principal.user_id())),
            imported_at: Set(at),
        }
        .insert(&transaction)
        .await?;
        // Chốt theo ID vận đơn tăng dần: `settle_shipment` khoá từng vận đơn, hai bảng kê chồng
        // nhau khoá cùng thứ tự thì không deadlock.
        settled.sort_by_key(|(_, shipment)| shipment.id);
        for (line_id, shipment) in settled {
            self.settle_shipment(&transaction, tenant, shipment.id, line_id).await?;
        }
        transaction.commit().await?;
        Ok((row, true))
    }

    /// Lấy bảng kê qua API của hãng rồi nhập.
    pub(crate) async fn fetch_statement(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        account_id: Uuid,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<(cod_statement::Model, bool), Error> {
        let (_, account, carrier) = self.account(tenant, account_id).await?;
        let statement =
            within(self.config.carrier_timeout_seconds, carrier.cod_statement(&account, from, to))
                .await?
                .ok_or_else(errors::statement_not_available)?;
        self.import_statement(tenant, principal, account_id, statement).await
    }

    /// Dòng khớp: vận đơn `settled`, event `shipping.cod_settled`, job ghi sổ.
    async fn settle_shipment(
        &self,
        transaction: &sea_orm::DatabaseTransaction,
        tenant: &TenantContext,
        shipment_id: Uuid,
        line_id: Uuid,
    ) -> Result<(), Error> {
        // Khoá vận đơn rồi mới xét `settled` (D-181): hai bảng kê cùng có tracking này (khác mã, hay
        // một bảng kê lặp dòng) nối đuôi nhau — bên sau thấy vận đơn đã chốt và không ghi sổ lần
        // hai. Bản đọc lúc đối chiếu dòng không khoá nên có thể đã cũ; ghi từ bản vừa khoá để không
        // đè cột webhook vừa đổi.
        let row = crate::shipments::lock_shipment(transaction, tenant, shipment_id).await?;
        if row.cod_status == "settled" {
            return Ok(());
        }
        let mut after = row;
        after.cod_status = "settled".to_owned();
        after.updated_at = now();
        let after = shipment::Entity::update(
            sea_orm::IntoActiveModel::into_active_model(after).reset_all(),
        )
        .validate()?
        .scoped(tenant)
        .exec(transaction)
        .await?;
        let mut payload =
            serde_json::to_value(crate::shipments::event_of(&after)).map_err(Error::internal)?;
        // D-216: tiền COD về người bán (hay đối tác thu hộ của người bán), không về nhà vận hành sàn.
        payload["seller_org_id"] = serde_json::json!(after.org_id);
        payload["paid_to"] = serde_json::json!({"kind": "carrier_account_payee", "carrier_account_id": after.carrier_account_id});
        payload["statement_line_id"] = serde_json::json!(line_id);
        let change = crate::events::Change { before: None, after: Some(payload) };
        self.outbox
            .record_named(
                transaction,
                tenant,
                Actor::system(),
                names::COD_SETTLED,
                crate::events::shipment_subject(after.id),
                &change,
            )
            .await?;
        self.jobs
            .enqueue(
                transaction,
                tenant,
                &SettleCod { line_id },
                EnqueueOptions::default().unique_key(format!("settle_cod:{line_id}")),
            )
            .await?;
        Ok(())
    }

    /// Ghi sổ cái cho một dòng bảng kê khớp (job).
    pub(crate) async fn post_settlement(
        &self,
        tenant: &TenantContext,
        line_id: Uuid,
    ) -> Result<(), Error> {
        let Some(ledger) = &self.ledger else { return Ok(()) };
        let read = tenant.database().read();
        let Some(line) =
            cod_statement_line::Entity::find_by_id(line_id).scoped(tenant).one(read).await?
        else {
            return Ok(());
        };
        let Some(shipment_id) = line.shipment_id else { return Ok(()) };
        let row = crate::shipments::find_shipment(read, tenant, shipment_id).await?;
        let Some(account) = row.carrier_account_id else { return Ok(()) };
        let currency = autoxide_kernel::money::Currency::parse(&row.currency)?;
        let cod = Money::new(line.cod_minor, currency);
        let fee = Money::new(line.fee_minor, currency);
        let net = cod.checked_sub(fee)?;
        let mut lines = Vec::new();
        if net.is_positive() {
            lines.push((Side::Debit, Account::InTransit(account), net));
        }
        if fee.is_positive() {
            lines.push((Side::Debit, Account::CarrierFee, fee));
        }
        lines.push((Side::Credit, Account::CarrierCod(account), cod));
        crate::ledger::post(
            ledger.as_ref(),
            tenant,
            OrgId::from_uuid(row.org_id),
            "cod_settled",
            format!("shipping.cod_settle:{line_id}"),
            "Cash on delivery settled by the carrier statement",
            shipment_id,
            lines,
        )
        .await?;
        Ok(())
    }

    #[expect(clippy::too_many_arguments, reason = "các trường của một dòng chênh lệch")]
    async fn open_discrepancy(
        &self,
        transaction: &sea_orm::DatabaseTransaction,
        tenant: &TenantContext,
        key: String,
        line_id: Option<Uuid>,
        shipment: Option<&shipment::Model>,
        kind: &str,
        expected: i64,
        reported: i64,
        currency: &str,
    ) -> Result<bool, Error> {
        let exists = cod_discrepancy::Entity::find()
            .filter(cod_discrepancy::Column::DiscrepancyKey.eq(&key))
            .scoped(tenant)
            .one(transaction)
            .await?;
        if exists.is_some() {
            return Ok(false);
        }
        let id = Uuid::now_v7();
        let row = cod_discrepancy::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            discrepancy_key: Set(key),
            statement_line_id: Set(line_id),
            shipment_id: Set(shipment.map(|row| row.id)),
            kind: Set(kind.to_owned()),
            expected_minor: Set(expected),
            reported_minor: Set(reported),
            currency: Set(currency.to_owned()),
            status: Set("open".to_owned()),
            resolution_code: Set(None),
            resolution_note: Set(String::new()),
            resolved_by: Set(None),
            resolved_at: Set(None),
            created_at: Set(now()),
        }
        .insert(transaction)
        .await?;
        if let Some(shipment) = shipment
            && shipment.cod_status != "mismatch"
            && shipment.cod_status != "settled"
        {
            shipment::Entity::update_many()
                .col_expr(shipment::Column::CodStatus, Expr::value("mismatch"))
                .filter(shipment::Column::Id.eq(shipment.id))
                .scoped(tenant)
                .exec(transaction)
                .await?;
        }
        let change = crate::events::Change {
            before: None,
            after: Some(serde_json::json!({
                "discrepancy_id": row.id,
                "kind": row.kind,
                "shipment_id": row.shipment_id,
                "order_id": shipment.map(|row| row.order_id),
                "expected": Money::from_columns(expected, currency).ok(),
                "reported": Money::from_columns(reported, currency).ok(),
            })),
        };
        self.outbox
            .record_named(
                transaction,
                tenant,
                Actor::system(),
                names::COD_MISMATCH,
                crate::events::discrepancy_subject(id),
                &change,
            )
            .await?;
        Ok(true)
    }

    /// Lịch hằng ngày: vận đơn giao xong có COD chưa đối soát quá `COD_SETTLEMENT_DAYS`.
    pub(crate) async fn cod_aging(&self, tenant: &TenantContext) -> Result<u64, Error> {
        let limit = now() - Duration::days(self.config.cod_settlement_days);
        let rows = shipment::Entity::find()
            .filter(shipment::Column::CodStatus.eq("collected"))
            .filter(shipment::Column::DeliveredAt.lte(limit))
            .scoped(tenant)
            .order_by_asc(shipment::Column::Id)
            .all(tenant.database().read())
            .await?;
        let mut opened = 0;
        for row in rows {
            let transaction = tenant.database().begin_write().await?;
            let created = self
                .open_discrepancy(
                    &transaction,
                    tenant,
                    format!("overdue:{}", row.id),
                    None,
                    Some(&row),
                    "overdue",
                    row.cod_minor.unwrap_or(0),
                    0,
                    &row.currency,
                )
                .await?;
            transaction.commit().await?;
            opened += u64::from(created);
        }
        Ok(opened)
    }

    pub(crate) async fn list_discrepancies(
        &self,
        tenant: &TenantContext,
        status: Option<String>,
        page: &PageRequest,
    ) -> Result<Page<cod_discrepancy::Model>, Error> {
        let mut condition = Condition::all();
        if let Some(status) = status {
            condition = condition.add(cod_discrepancy::Column::Status.eq(status));
        }
        let select = cod_discrepancy::Entity::find().filter(condition).scoped(tenant);
        fetch_page(
            select,
            cod_discrepancy::Column::Id,
            |row| row.id,
            page,
            tenant.database().read(),
        )
        .await
    }

    pub(crate) async fn list_statements(
        &self,
        tenant: &TenantContext,
        page: &PageRequest,
    ) -> Result<Page<cod_statement::Model>, Error> {
        let select = cod_statement::Entity::find().scoped(tenant);
        fetch_page(select, cod_statement::Column::Id, |row| row.id, page, tenant.database().read())
            .await
    }

    pub(crate) async fn get_statement(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<(cod_statement::Model, Vec<cod_statement_line::Model>), Error> {
        let read = tenant.database().read();
        let row = cod_statement::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| errors::statement_not_found(id))?;
        let lines = cod_statement_line::Entity::find()
            .filter(cod_statement_line::Column::StatementId.eq(id))
            .scoped(tenant)
            .order_by_asc(cod_statement_line::Column::Position)
            .all(read)
            .await?;
        Ok((row, lines))
    }

    /// Giải quyết chênh lệch: mã lý do của danh sách `shipping.cod_resolution` + ghi chú bắt buộc.
    /// `written_off` → bút toán xoá khoản phải thu của hãng (job ghi sổ, mã tham chiếu riêng).
    pub(crate) async fn resolve_discrepancy(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        code: String,
        note: String,
    ) -> Result<cod_discrepancy::Model, Error> {
        let mut problems = Vec::new();
        if let Err(error) = self.reasons.ensure(tenant, RESOLUTION_LIST, &code).await {
            problems.push(
                FieldError::new("resolution_code", "unknown", "must be an active code of `{list}`")
                    .with_param("list", RESOLUTION_LIST),
            );
            tracing::debug!(code = %error.code(), "resolution code refused");
        }
        crate::service::check_length(&mut problems, "note", note.trim(), 1, 1000);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let transaction = tenant.database().begin_write().await?;
        // Khoá chênh lệch rồi mới xét `resolved` (D-181): hai người xử lý cùng lúc thì bên sau thấy
        // đã xử lý — không ghi đè cách xử lý của bên trước trong khi job xoá nợ của bên đó đã xếp.
        let row = autoxide_kernel::rows::lock_one(
            &transaction,
            cod_discrepancy::Entity::find_by_id(id).scoped(tenant),
        )
        .await?
        .ok_or_else(|| errors::discrepancy_not_found(id))?;
        if row.status == "resolved" {
            return Err(errors::discrepancy_resolved());
        }
        let mut active: cod_discrepancy::ActiveModel = row.into();
        active.status = Set("resolved".to_owned());
        active.resolution_code = Set(Some(code.clone()));
        active.resolution_note = Set(note.trim().to_owned());
        active.resolved_by = Set(Some(principal.user_id()));
        active.resolved_at = Set(Some(now()));
        let row = active.update(&transaction).await?;
        let change = crate::events::Change {
            before: None,
            after: Some(
                serde_json::json!({"discrepancy_id": id, "resolution_code": code, "shipment_id": row.shipment_id}),
            ),
        };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::user(principal.user_id()),
                crate::events::discrepancy_subject(id),
                &crate::events::CodDiscrepancyResolved(change),
            )
            .await?;
        if code == "written_off" {
            self.jobs
                .enqueue(
                    &transaction,
                    tenant,
                    &crate::jobs::WriteOffCod { discrepancy_id: id },
                    EnqueueOptions::default().unique_key(format!("write_off:{id}")),
                )
                .await?;
        }
        transaction.commit().await?;
        Ok(row)
    }

    /// Bút toán xoá khoản COD không thu được: Nợ `expense.cod_write_off` / Có `clearing.carrier_cod`.
    pub(crate) async fn post_write_off(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<(), Error> {
        let Some(ledger) = &self.ledger else { return Ok(()) };
        let read = tenant.database().read();
        let Some(row) = cod_discrepancy::Entity::find_by_id(id).scoped(tenant).one(read).await?
        else {
            return Ok(());
        };
        let Some(shipment_id) = row.shipment_id else { return Ok(()) };
        let shipment = crate::shipments::find_shipment(read, tenant, shipment_id).await?;
        let Some(account) = shipment.carrier_account_id else { return Ok(()) };
        let gap = (row.expected_minor - row.reported_minor).max(0);
        if gap == 0 {
            return Ok(());
        }
        let currency = autoxide_kernel::money::Currency::parse(&row.currency)?;
        let amount = Money::new(gap, currency);
        crate::ledger::post(
            ledger.as_ref(),
            tenant,
            OrgId::from_uuid(shipment.org_id),
            "cod_written_off",
            format!("shipping.cod_write_off:{id}"),
            "Cash on delivery written off after reconciliation",
            shipment_id,
            vec![
                (Side::Debit, Account::WriteOff, amount),
                (Side::Credit, Account::CarrierCod(account), amount),
            ],
        )
        .await?;
        Ok(())
    }

    /// Hỏi lại hành trình của vận đơn có chênh lệch (gọi hãng ngoài transaction ghi).
    pub(crate) async fn refresh_discrepancy(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<cod_discrepancy::Model, Error> {
        let read = tenant.database().read();
        let row = cod_discrepancy::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| errors::discrepancy_not_found(id))?;
        if let Some(shipment_id) = row.shipment_id {
            self.refresh_tracking(tenant, shipment_id).await?;
        }
        Ok(row)
    }
}
