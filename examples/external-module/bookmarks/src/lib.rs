//! Module `bookmarks`: links a user keeps. An example of a module that lives outside the
//! Autoxide repository and still uses every part of the module contract: settings, migrations,
//! permissions, events, an event subscription, a scheduled job, REST routes in the OpenAPI
//! document, MCP tools and translations.
//!
//! A module crate shows two things to the outside, nothing else: `pub mod api` (what other
//! modules may use) and `pub fn module()` (for the binary). Everything else stays private, so
//! the compiler keeps other crates out of the tables and the service.

pub mod api;
mod config;
mod entity;
mod events;
mod http;
mod migration;
mod permissions;
mod retention;
mod service;
mod tools;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{EventInfo, Subscription};
use autoxide_kernel::job::{JobRegistration, Schedule};
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use utoipa_axum::router::OpenApiRouter;

use crate::config::BookmarksConfig;
use crate::service::BookmarkService;

/// The `bookmarks` module, for a binary to register: `Autoxide::new().module(acme_bookmarks::module)`.
pub fn module() -> impl Module {
    BookmarksModule::default()
}

#[derive(Default)]
struct BookmarksModule {
    /// Set by `configure`.
    config: Option<BookmarksConfig>,
    /// Set by `start`.
    service: Option<Arc<BookmarkService>>,
}

impl Module for BookmarksModule {
    // The name is the prefix of everything the module owns: tables (`bookmarks_…`), routes
    // (`/api/v1/bookmarks`), permissions, events, jobs, error codes and MCP tools (`bookmarks.…`).
    fn name(&self) -> &'static str {
        "bookmarks"
    }

    fn description(&self) -> &'static str {
        "Bookmarks: links each user keeps, with a title."
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(BookmarksConfig::read(reader)?);
        Ok(())
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn permissions(&self) -> Vec<Permission> {
        permissions::all()
    }

    fn events(&self) -> Vec<EventInfo> {
        events::all()
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        // `include_str!` puts the file into the binary at compile time: nothing to ship next to it.
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self.config.clone().unwrap_or_default();
        let service = Arc::new(BookmarkService::new(context.outbox()?, config));
        self.service = Some(service.clone());
        // Other modules see only the trait: `Arc<BookmarkService>` becomes `Arc<dyn Bookmarks>`.
        context.provide::<dyn api::Bookmarks>(service)
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        // An account was deleted (an event of the official `auth` module): delete its bookmarks.
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![Subscription::new(autoxide_kernel::privacy::USER_DELETED, service)]
        })
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![JobRegistration::new(
                retention::CLEANUP,
                Arc::new(retention::Cleanup::new(service)),
            )]
        })
    }

    fn schedules(&self) -> Vec<Schedule> {
        vec![retention::schedule()]
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    fn tools(&self) -> Vec<Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }
}
