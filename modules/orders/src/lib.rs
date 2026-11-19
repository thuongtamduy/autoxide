//! Module `orders` (bước 45): giỏ hàng cho người mua và vãng lai, đặt hàng là chuỗi giữ chỗ có hạn
//! (tồn kho, lượt khuyến mại) rồi tạo đơn có số tuần tự, thanh toán qua đối tác có giấy phép theo
//! chế độ dòng tiền của khách, máy trạng thái của đơn, đổi trả, hoàn tiền, hoá đơn điện tử, bán tại
//! quầy, đánh giá sau mua. Gọi `catalog`, `inventory`, `promotions`, `shipping`, `tax`, `ledger` qua
//! `api` của chúng; core không bao giờ tự giữ tiền của người mua (D-187).
//! Đặc tả: `docs/steps/45-orders.md`.

pub mod api;
mod events;
mod extensions;
mod migration;
mod permissions;
mod reasons;
mod relocation;

use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::error::Error;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};

use crate::extensions::Extensions;

/// Module `orders`, để binary đăng ký trong `crates/app/src/modules.rs`.
pub fn module() -> impl Module {
    OrdersModule::default()
}

#[derive(Default)]
struct OrdersModule {
    /// Điểm cắm cho module đăng ký sau (bước 46–49); có từ lúc tạo để `start` trao ra ngoài.
    extensions: Arc<Extensions>,
}

impl Module for OrdersModule {
    fn name(&self) -> &'static str {
        "orders"
    }

    fn description(&self) -> &'static str {
        "Orders: carts for buyers and guests, checkout with time-limited holds of stock and \
         promotions, orders with sequential numbers and a lifecycle, payments through licensed \
         partners, returns and refunds, electronic invoices, point of sale, product reviews."
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn permissions(&self) -> Vec<Permission> {
        permissions::all()
    }

    fn events(&self) -> Vec<autoxide_kernel::event::EventInfo> {
        events::all()
    }

    fn reason_lists(&self) -> Vec<autoxide_kernel::reasons::ReasonList> {
        reasons::lists()
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        // `Arc<Extensions>` tự đổi thành `Arc<dyn OrderExtensions>`: module khác chỉ thấy trait.
        context.provide::<dyn api::OrderExtensions>(self.extensions.clone())?;
        Ok(())
    }

    fn relocation(&self) -> autoxide_kernel::tenancy::Relocation {
        relocation::relocation()
    }
}
