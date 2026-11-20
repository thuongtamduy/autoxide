//! Bút toán nghĩa vụ điểm (bước 47 mục 3): ghi qua job `customers.post_ledger` đưa vào hàng đợi CÙNG
//! transaction với thay đổi điểm (không gọi `ledger` trong transaction ghi; transaction rollback thì
//! không có bút toán). `reference` = `customers:points:<id bút toán điểm>`; job chạy lại thì `ledger`
//! trả `Replayed`, không ghi hai lần. Mọi bút toán điểm ghi ở sổ của nút gốc của khách.
//!
//! | Việc | Nợ | Có |
//! |---|---|---|
//! | Tích điểm | `loyalty.expense` | `loyalty.liability` (điểm × giá trị lúc tích) |
//! | Đổi điểm trả đơn | `loyalty.liability` (giá trị lúc tích) | `buyer.prepayments` (tiền được trừ), chênh lệch vào `loyalty.expense` |
//! | Hết hạn | `loyalty.liability` | `loyalty.breakage` |
//! | Trả lại điểm giữ | — (chưa ghi gì khi giữ) | |
//! | Điều chỉnh, đổi lấy voucher, thu hồi khi hoàn | `loyalty.liability` ↔ `loyalty.expense` | |

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::{TenantContext, root_of};
use autoxide_ledger::api::{AccountKey, LineAccount, NewEntry, NewLine, OpenAccount, Side};
use sea_orm::ConnectionTrait;
use serde::{Deserialize, Serialize};

use crate::service::CustomersService;

pub(crate) const POST_LEDGER: &str = "customers.post_ledger";

pub(crate) const LIABILITY: &str = "loyalty.liability";
pub(crate) const EXPENSE: &str = "loyalty.expense";
pub(crate) const BREAKAGE: &str = "loyalty.breakage";
pub(crate) const PREPAYMENTS: &str = "buyer.prepayments";

/// Bên thường của các tài khoản riêng của điểm.
fn normal_side(purpose: &str) -> Side {
    if purpose == EXPENSE || purpose.ends_with(".expense") { Side::Debit } else { Side::Credit }
}

/// Một dòng bút toán: mục đích tài khoản, Nợ (`debit`) hay Có, số tiền nhỏ nhất.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Line {
    pub purpose: String,
    pub debit: bool,
    pub minor: i64,
}

impl Line {
    pub(crate) fn debit(purpose: &str, minor: i64) -> Self {
        Line { purpose: purpose.to_owned(), debit: true, minor }
    }
    pub(crate) fn credit(purpose: &str, minor: i64) -> Self {
        Line { purpose: purpose.to_owned(), debit: false, minor }
    }
}

/// Job ghi một bút toán.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct PostLedger {
    pub reference: String,
    pub description: String,
    pub currency: String,
    pub lines: Vec<Line>,
}

impl Job for PostLedger {
    const KIND: &'static str = POST_LEDGER;
}

impl CustomersService {
    /// Đưa bút toán vào hàng đợi cùng transaction `connection`. Không có `ledger` (chưa cài): bỏ
    /// qua. Dòng 0 đồng bị bỏ; còn ít hơn hai dòng thì không ghi gì.
    pub(crate) async fn queue_posting<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        reference: String,
        description: &str,
        currency: &str,
        lines: Vec<Line>,
    ) -> Result<(), Error> {
        if self.ledger.is_none() {
            return Ok(());
        }
        let lines: Vec<Line> = lines.into_iter().filter(|line| line.minor != 0).collect();
        if lines.len() < 2 {
            return Ok(());
        }
        let job = PostLedger {
            reference: reference.clone(),
            description: description.to_owned(),
            currency: currency.to_owned(),
            lines,
        };
        self.jobs
            .enqueue(connection, tenant, &job, EnqueueOptions::default().unique_key(reference))
            .await?;
        Ok(())
    }
}

pub(crate) struct PostLedgerJob(pub Arc<CustomersService>);

#[async_trait::async_trait]
impl JobHandler for PostLedgerJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let Some(ledger) = &self.0.ledger else { return Ok(()) };
        let tenant =
            job.tenant.as_ref().ok_or_else(|| Error::internal("ledger job without tenant"))?;
        let input: PostLedger =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        let currency = Currency::parse(&input.currency)?;
        let book = root_of(tenant.id());
        let mut lines = Vec::with_capacity(input.lines.len());
        for line in &input.lines {
            let key = AccountKey::new(book, line.purpose.clone(), currency);
            // Tài khoản riêng của điểm: mở với bên thường đúng (gọi lại trả tài khoản có sẵn).
            let account = ledger
                .open_account(tenant, OpenAccount::new(key).normal_side(normal_side(&line.purpose)))
                .await?;
            let amount = Money::new(line.minor, currency);
            lines.push(if line.debit {
                NewLine::debit(LineAccount::Id(account), amount)
            } else {
                NewLine::credit(LineAccount::Id(account), amount)
            });
        }
        let entry =
            NewEntry::new(book, "customers", "points", input.reference, input.description, lines);
        ledger.post(tenant, Actor::system(), entry).await?;
        Ok(())
    }
}
