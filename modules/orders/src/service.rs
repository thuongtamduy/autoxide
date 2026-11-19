//! Service layer của `orders`: mọi logic ở đây (và ở các file tính năng `carts.rs`, …); handler
//! REST, tool MCP, job và event đều gọi vào đây.

use std::sync::Arc;

use autoxide_address::api::AdministrativeUnits;
use autoxide_kernel::event::Outbox;
use autoxide_kernel::tenancy::DatabaseRegistry;
use autoxide_shipping::api::AddressBook;

use crate::config::OrdersConfig;
use crate::extensions::Extensions;
use crate::pricing::Sources;

/// Các phần service cần, lấy lúc `start`.
pub(crate) struct Parts {
    pub outbox: Outbox,
    pub sources: Sources,
    pub address_book: Option<Arc<dyn AddressBook>>,
    pub units: Option<Arc<dyn AdministrativeUnits>>,
    pub extensions: Arc<Extensions>,
    pub databases: DatabaseRegistry,
}

pub(crate) struct OrdersService {
    pub outbox: Outbox,
    pub sources: Sources,
    pub address_book: Option<Arc<dyn AddressBook>>,
    pub units: Option<Arc<dyn AdministrativeUnits>>,
    pub extensions: Arc<Extensions>,
    pub databases: DatabaseRegistry,
    pub config: OrdersConfig,
}

impl OrdersService {
    pub(crate) fn new(parts: Parts, config: OrdersConfig) -> Self {
        OrdersService {
            outbox: parts.outbox,
            sources: parts.sources,
            address_book: parts.address_book,
            units: parts.units,
            extensions: parts.extensions,
            databases: parts.databases,
            config,
        }
    }
}
