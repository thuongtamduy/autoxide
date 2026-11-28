//! Shared errors of the multi-tenancy part (stable codes, translations in `crates/kernel/locales/vi.json`).
//! Moved out of `tenancy.rs` unchanged (D-20261118-file-length).

use crate::error::Error;

/// The request's tenant cannot be determined.
pub fn tenant_required() -> Error {
    Error::bad_request(
        "tenant_required",
        "No customer (tenant) could be told for this request. Send it to the tenant's domain, \
         add the header `Autoxide-Tenant: <slug>`, or sign in.",
    )
}

pub fn tenant_not_found(tenant: &str) -> Error {
    Error::not_found("tenant_not_found", "There is no tenant `{tenant}`.")
        .with_param("tenant", tenant)
}

pub fn tenant_suspended() -> Error {
    Error::forbidden("tenant_suspended", "This tenant is suspended. Contact the service provider.")
}

pub fn tenant_read_only() -> Error {
    Error::forbidden(
        "tenant_read_only",
        "This tenant is read-only for now: its data can be read and exported, but not \
         changed. Contact the service provider.",
    )
}

pub fn tenant_maintenance(seconds: u64) -> Error {
    Error::unavailable(
        "tenant_maintenance",
        "The data of this tenant is under maintenance. Retry in {seconds} seconds.",
    )
    .with_param("seconds", seconds)
    .with_retry_after(seconds)
}

pub fn tenant_mismatch() -> Error {
    Error::forbidden(
        "tenant_mismatch",
        "The credentials belong to another tenant than the one this request is for.",
    )
}

/// The request came from the web origin (`Origin`) of one tenant's verified domain but acts for another
/// tenant: a page on a tenant's domain may only call the API as that tenant.
pub fn tenant_origin_mismatch() -> Error {
    Error::forbidden(
        "tenant_origin_mismatch",
        "This request comes from a web page on another tenant's domain. A page on a tenant's \
         domain may only call the API for that tenant: call from that tenant's own page, or add \
         the page's origin to AUTOXIDE_HTTP_CORS_ORIGINS.",
    )
}

pub fn tenant_platform_only() -> Error {
    Error::forbidden("tenant_platform_only", "Only administrators of the platform can do this.")
}

pub fn tenant_single_mode() -> Error {
    Error::conflict(
        "tenant_single_mode",
        "This installation runs in single-tenant mode; set AUTOXIDE_TENANCY_MODE=multi to add \
         tenants.",
    )
}

pub fn tenant_placement_taken(placement: &str) -> Error {
    Error::conflict("tenant_placement_taken", "There is already a placement named `{placement}`.")
        .with_param("placement", placement)
}

pub fn tenant_placement_behind(placement: &str) -> Error {
    Error::conflict(
        "tenant_placement_behind",
        "The placement `{placement}` has migrations to apply: run `autoxide migrate up --all` \
         first.",
    )
    .with_param("placement", placement)
}

pub fn tenant_residency_violation(country: &str) -> Error {
    Error::conflict(
        "tenant_residency_violation",
        "The tenant must keep its data in {country}; this placement does not meet that (its \
         country is unknown or different). Set the placement's country with `autoxide \
         placement country`, or choose a placement in {country}.",
    )
    .with_param("country", country)
}

pub fn tenant_slug_taken(slug: &str) -> Error {
    Error::conflict("tenant_slug_taken", "Another tenant already uses the slug `{slug}`.")
        .with_param("slug", slug)
}

pub fn tenant_setting_unknown(name: &str) -> Error {
    Error::new(
        crate::error::ErrorKind::Validation,
        "tenant_setting_unknown",
        "No module takes the setting `{name}`. List them with `GET /api/v1/kernel/settings`.",
    )
    .with_param("name", name)
}

pub fn tenant_placement_not_found(placement: &str) -> Error {
    Error::not_found("tenant_placement_not_found", "There is no placement `{placement}`.")
        .with_param("placement", placement)
}

pub fn tenant_placement_storage_in_use(placement: &str, tenants: u64) -> Error {
    Error::conflict(
        "tenant_placement_storage_in_use",
        "{tenants} tenant(s) keep their data on the placement `{placement}`; changing its file \
         storage would hide their files. Move them to another placement first, or pass \
         `--force` when the new storage already holds the same files (e.g. a new access key).",
    )
    .with_param("placement", placement)
    .with_param("tenants", tenants)
}

pub fn tenant_domain_invalid(domain: &str) -> Error {
    // Giá trị do client gửi: chỉ nhắc lại phần đầu.
    let shown: String = domain.chars().take(100).collect();
    Error::new(
        crate::error::ErrorKind::Validation,
        "tenant_domain_invalid",
        "`{domain}` is not a domain that can be used: send a name such as `shop.example` \
         (punycode for international names), not an IP address, `localhost` or the \
         platform's own domain.",
    )
    .with_param("domain", shown)
}

pub fn tenant_domain_taken(domain: &str) -> Error {
    Error::conflict("tenant_domain_taken", "The domain `{domain}` belongs to another tenant.")
        .with_param("domain", domain)
}

pub fn tenant_domain_not_found() -> Error {
    Error::not_found(
        "tenant_domain_not_found",
        "This domain is not one of the current tenant's domains.",
    )
}

pub fn tenant_domain_revoked() -> Error {
    Error::conflict(
        "tenant_domain_revoked",
        "This domain was removed from the tenant and cannot be changed. Add it again with \
         `POST /api/v1/kernel/domains`.",
    )
}

pub fn tenant_domain_dns_failed(error: &str) -> Error {
    Error::unavailable("tenant_domain_dns_failed", "The DNS lookup failed: {error}")
        .with_param("error", error)
}

pub fn tenant_moving() -> Error {
    Error::conflict(
        "tenant_moving",
        "The tenant is being moved to another placement. Wait until the move is done, or \
         abort it with `autoxide tenant move <tenant> --abort`.",
    )
}

pub fn tenant_move_platform() -> Error {
    Error::conflict(
        "tenant_move_platform",
        "The platform tenant always keeps its data on the placement `main`; it cannot be moved.",
    )
}

pub fn tenant_move_same_placement(placement: &str) -> Error {
    Error::conflict(
        "tenant_move_same_placement",
        "The tenant already keeps its data on the placement `{placement}`.",
    )
    .with_param("placement", placement)
}

pub fn tenant_move_not_active(status: &str) -> Error {
    Error::conflict(
        "tenant_move_not_active",
        "Only an active tenant can be moved; this one is `{status}`.",
    )
    .with_param("status", status)
}

pub fn tenant_move_running(placement: &str) -> Error {
    Error::conflict(
        "tenant_move_running",
        "A move of this tenant to the placement `{placement}` is running now. Follow it with \
         the move status (`GET /api/v1/kernel/tenants/<tenant>/moves/<move>`) and try again when it is done.",
    )
    .with_param("placement", placement)
}

pub fn tenant_move_unfinished(placement: &str) -> Error {
    Error::conflict(
        "tenant_move_unfinished",
        "A move of this tenant to the placement `{placement}` stopped before the end. Resume \
         it by moving the tenant to `{placement}` again, or abort it with \
         `autoxide tenant move <tenant> --abort`.",
    )
    .with_param("placement", placement)
}

pub fn tenant_move_not_abortable() -> Error {
    Error::conflict(
        "tenant_move_not_abortable",
        "The tenant already uses its new placement; the move only has to clean the old one. \
         Resume it instead of aborting: move the tenant to the same placement again.",
    )
}

pub fn tenant_move_schema_differs(placement: &str, pending: usize) -> Error {
    Error::conflict(
        "tenant_move_schema_differs",
        "The placement `{placement}` has {pending} migration(s) not applied: run \
         `autoxide migrate up --all` so both databases have the same schema.",
    )
    .with_param("placement", placement)
    .with_param("pending", pending)
}

pub fn tenant_move_same_database(placement: &str) -> Error {
    Error::conflict(
        "tenant_move_same_database",
        "The placement `{placement}` is the same database as the tenant's current one; moving \
         would delete the tenant's data. Give each placement its own database.",
    )
    .with_param("placement", placement)
}

pub fn tenant_move_unreachable(placement: &str, error: &str) -> Error {
    Error::unavailable(
        "tenant_move_unreachable",
        "Cannot reach the database of the placement `{placement}`: {error}",
    )
    .with_param("placement", placement)
    .with_param("error", error)
}

pub fn tenant_move_storage_unavailable(placement: &str, error: &str) -> Error {
    Error::unavailable(
        "tenant_move_storage_unavailable",
        "Cannot reach the file storage of the placement `{placement}`: {error}",
    )
    .with_param("placement", placement)
    .with_param("error", error)
}

pub fn tenant_move_busy(jobs: u64) -> Error {
    Error::unavailable(
        "tenant_move_busy",
        "{jobs} job(s) or event(s) of the tenant are still being run at the old placement. \
         The move is paused; run it again in a few minutes.",
    )
    .with_param("jobs", jobs)
}

pub fn tenant_move_count_mismatch(table: &str, source: u64, target: u64) -> Error {
    Error::conflict(
        "tenant_move_count_mismatch",
        "After copying, `{table}` has {target} row(s) of the tenant at the new placement and \
         {source} at the old one; the tenant keeps its old placement. Run the move again to \
         copy once more.",
    )
    .with_param("table", table)
    .with_param("source", source)
    .with_param("target", target)
}

pub fn tenant_move_not_found(id: &str) -> Error {
    Error::not_found("tenant_move_not_found", "The tenant has no move `{id}`.").with_param("id", id)
}

pub fn tenant_purge_current(placement: &str) -> Error {
    Error::conflict(
        "tenant_purge_current",
        "The placement `{placement}` keeps the tenant's live data; only a copy left on \
         another placement (by `--keep-source`) can be deleted.",
    )
    .with_param("placement", placement)
}
