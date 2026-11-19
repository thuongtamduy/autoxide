// TẠM (PR #199): chỉ để đo CI khi mọi crate bị ảnh hưởng; xoá trước khi merge.
//! Nền của Autoxide: thứ mà mọi module dựa vào (cấu hình, database, kiểu lỗi,
//! HTTP, hợp đồng module). Kernel không biết module nào — phụ thuộc chỉ đi một
//! chiều từ module vào kernel (`cargo xtask check` kiểm điều này).
//!
//! Bố cục (D-247): file xếp theo nhóm riêng tư — `contracts/` (trait module cung cấp cho nhau),
//! `data/` (database), `security/`, `ops/` (vận hành), `values/` (kiểu giá trị). Trên cùng là
//! phần mọi module chạm tới: `error`, `response`, `i18n`, `module`, `config`, `http`, `event`,
//! `job`, `mcp`. Đường dẫn công khai KHÔNG có tên nhóm: `pub use` dưới đây giữ
//! `autoxide_kernel::ai::TextModel` cho module ở repo riêng (D-167), và `crate::ai::…` bên trong
//! kernel — nhóm là riêng tư nên không có đường thứ hai (`autoxide_kernel::contracts::ai`). Module
//! mới vào nhóm hợp với nó (mỗi file nhóm ghi module nào của bước 33+ thuộc về đó) rồi thêm vào
//! `pub use`; test `tests/public_paths.rs` giữ mọi đường dẫn công khai.

mod contracts;
mod data;
mod ops;
mod security;
mod values;

pub use contracts::{
    ai, analytics, einvoice, identity, mail, moderation, payment, privacy, push, report, resource,
    sms, storage, store, zns,
};
pub use data::{
    backup, database, migration, numbering, pagination, reference, rows, search, sync, tenancy,
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
pub use values::{csv, decimal, email, money, public_url, slug, template};

pub mod config;
pub mod error;
pub mod event;
pub mod http;
pub mod i18n;
pub mod job;
pub mod mcp;
pub mod module;
pub mod response;
