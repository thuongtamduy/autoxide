//! Tài khoản sổ cái `shipping` dùng khi ghi tiền thu hộ (COD, D-216). Core không giữ tiền: các bút
//! toán chỉ ghi nghĩa vụ giữa người bán, hãng vận chuyển và người mua. Sổ là nút bán của vận đơn.
//!
//! | Tài khoản | Mục đích | Đối tượng |
//! |---|---|---|
//! | Hãng còn nợ tiền thu hộ | `clearing.carrier_cod` (cơ bản) | `shipping.carrier_account:<id>` |
//! | Người mua còn nợ tiền đơn | `receivable.customer` (cơ bản) | `orders.order:<id>` |
//! | Tiền hãng đang chuyển về | `shipping.settlement_in_transit` (riêng, bên Nợ) | tài khoản hãng |
//! | Phí hãng trừ vào tiền thu hộ | `shipping.carrier_fee` (riêng, bên Nợ) | — |
//! | Khoản thu hộ không thu được | `shipping.cod_write_off` (riêng, bên Nợ) | — |

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use autoxide_ledger::api::{
    AccountKey, Counterparty, Ledger, LineAccount, NewEntry, NewLine, OpenAccount, Side,
};
use uuid::Uuid;

/// Một tài khoản cần dùng.
pub(crate) enum Account {
    CarrierCod(Uuid),
    BuyerReceivable(Uuid),
    InTransit(Uuid),
    CarrierFee,
    WriteOff,
}

async fn resolve(
    ledger: &dyn Ledger,
    tenant: &TenantContext,
    book: OrgId,
    currency: Currency,
    account: &Account,
) -> Result<Uuid, Error> {
    let carrier = |id: &Uuid| Counterparty::new("shipping.carrier_account", id.to_string());
    match account {
        Account::CarrierCod(id) => {
            let key = AccountKey::new(book, "clearing.carrier_cod", currency)
                .with_counterparty(carrier(id));
            ledger.ensure_account(tenant, key).await
        }
        Account::BuyerReceivable(order) => {
            let key = AccountKey::new(book, "receivable.customer", currency)
                .with_counterparty(Counterparty::new("orders.order", order.to_string()));
            ledger.ensure_account(tenant, key).await
        }
        Account::InTransit(id) => {
            let key = AccountKey::new(book, "shipping.settlement_in_transit", currency)
                .with_counterparty(carrier(id));
            ledger.open_account(tenant, OpenAccount::new(key).normal_side(Side::Debit)).await
        }
        Account::CarrierFee => {
            let key = AccountKey::new(book, "shipping.carrier_fee", currency);
            ledger.open_account(tenant, OpenAccount::new(key).normal_side(Side::Debit)).await
        }
        Account::WriteOff => {
            let key = AccountKey::new(book, "shipping.cod_write_off", currency);
            ledger.open_account(tenant, OpenAccount::new(key).normal_side(Side::Debit)).await
        }
    }
}

/// Ghi một bút toán của `shipping` (mã tham chiếu chống ghi hai lần khi job chạy lại).
#[expect(clippy::too_many_arguments, reason = "các phần của một bút toán, chỉ gọi trong module")]
pub(crate) async fn post(
    ledger: &dyn Ledger,
    tenant: &TenantContext,
    book: OrgId,
    kind: &str,
    reference: String,
    description: &str,
    shipment_id: Uuid,
    lines: Vec<(Side, Account, Money)>,
) -> Result<(), Error> {
    let mut entry_lines = Vec::with_capacity(lines.len());
    for (side, account, amount) in lines {
        let id = resolve(ledger, tenant, book, amount.currency(), &account).await?;
        entry_lines.push(match side {
            Side::Debit => NewLine::debit(LineAccount::Id(id), amount),
            Side::Credit => NewLine::credit(LineAccount::Id(id), amount),
        });
    }
    let entry = NewEntry::new(book, "shipping", kind, reference, description, entry_lines)
        .source_id(shipment_id.to_string());
    ledger.post(tenant, Actor::system(), entry).await?;
    Ok(())
}
