//! Contracts between modules: traits that one module provides through `StartContext::provide` and other
//! modules obtain through `require` (AI, mail sending, file storage, personal data, the object registry).
//!
//! Step 33+ adds more here: `payment`, `einvoice`, `identity`, `sms`, `entitlement`.

pub mod ai;
pub mod analytics;
pub mod blocks;
pub mod einvoice;
pub mod identity;
pub mod mail;
pub mod moderation;
pub mod payment;
pub mod privacy;
pub mod provider;
pub mod push;
pub mod report;
pub mod resource;
pub mod sms;
pub mod storage;
pub mod store;
pub mod voice;
pub mod zns;
