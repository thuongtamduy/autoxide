//! The foundation of Autoxide: what every module builds on (configuration, database, error type, HTTP,
//! module contract). The kernel knows no module - dependencies go in one direction, from module to kernel
//! (`cargo xtask check` verifies this).
//!
//! Layout (D-247): files are arranged in private groups - `contracts/` (traits modules provide to each
//! other), `data/` (database), `security/`, `ops/` (operations), `values/` (value types). On top are the
//! parts every module touches: `error`, `response`, `i18n`, `module`, `config`, `http`, `event`, `job`,
//! `mcp`. Public paths do NOT carry the group name: the `pub use` below keeps
//! `autoxide_kernel::ai::TextModel` for modules in separate repos (D-167), and `crate::ai::…` inside the
//! kernel - the group is private so there is no second path (`autoxide_kernel::contracts::ai`). A new
//! module goes into the group that fits it (each group file records which step 33+ modules belong there)
//! and is then added to `pub use`; the test `tests/public_paths.rs` guards every public path.

// `#[derive(Record)]` sinh mã gọi `::autoxide_kernel::…`; dòng này cho chính kernel dùng được derive đó.
extern crate self as autoxide_kernel;

mod contracts;
mod data;
mod ops;
mod security;
mod values;

pub use contracts::{
    accounts, ai, analytics, blocks, einvoice, identity, mail, moderation, payment, privacy,
    provider, push, report, resource, sms, storage, store, voice, zns,
};
pub use data::{
    backup, database, db, migration, numbering, pagination, reference, rows, search, sync, tenancy,
    transfer, upload, version,
};
pub use ops::{
    automation, bench_seed, bulk, connections, custom_fields, doctor, find, list_exports, logging,
    metrics, reasons, resolve, setup, suggest, summary, tags, task, telemetry, texts, usages, work,
};
pub(crate) use ops::{builtin, module_catalog, user_flags};
pub use security::{
    access, antispam, entitlement, idempotency, ip_hash, net, secret, throttle, tls, tokens,
};
pub use values::{csv, decimal, email, money, public_url, slug, template, time};

pub mod config;
pub mod error;
pub mod event;
pub mod http;
pub mod i18n;
pub mod job;
pub mod mcp;
pub mod module;
pub mod response;

/// Lets a trait have `async fn` and still be used as `dyn Trait` (`#[async_trait]` on the trait and on each impl).
/// Modules write `autoxide_kernel::async_trait` instead of depending on the `async-trait` crate.
pub use async_trait::async_trait;

/// Paths used by the code the kernel's macros generate. Not part of the contract: never write them by hand
/// (`cargo xtask check` refuses `__private` in module code).
#[doc(hidden)]
pub mod __private {
    pub use axum;
    pub use utoipa;
    pub use uuid::Uuid;
    /// The checks `#[derive(Validate)]` generates calls to.
    pub mod check {
        pub use crate::http::check::*;
    }

    /// Written only by `#[derive(Record)]`: a hand-written `impl Record` cannot exist.
    pub trait RecordSeal {}
}
