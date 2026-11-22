//! The `acme` server: every official Autoxide module, plus `bookmarks`.
//!
//! This is the whole program. `Autoxide::run` gives it the standard command line (`acme serve`,
//! `acme migrate up`, `acme doctor`, `acme openapi`, `acme config`…), and the module added here gets
//! everything a built-in module gets.

use std::process::ExitCode;

fn main() -> ExitCode {
    autoxide::Autoxide::new()
        .name("acme")
        .version(env!("CARGO_PKG_VERSION"))
        // One line per module of your own, in the order they start: a module that uses another
        // module's trait comes after it. They all start after the official modules.
        .module(acme_bookmarks::module)
        .run()
}
