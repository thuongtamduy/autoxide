//! Copies, deletes and compares a tenant's stored files between two stores.

use super::*;

/// Key prefix of the files of tenant `tenant` (not the default tenant, which cannot be moved).
pub(super) fn tenant_prefix(tenant: Uuid) -> String {
    format!("tenants/{tenant}/")
}

/// Whether `a` and `b` are really the same store: writes a marker file (under `prefix`) to `b` then looks
/// for it in `a`. Different configurations can still point at the same place (e.g. the shared store's
/// directory); then copying or deleting the "source" would delete files in use.
pub(super) async fn same_store(
    a: &dyn Storage,
    b: &dyn Storage,
    prefix: &str,
) -> Result<bool, Error> {
    let marker = format!("{prefix}move-check-{}", Uuid::now_v7());
    let content = futures_util::stream::iter([Ok(bytes::Bytes::from_static(b"autoxide"))]);
    b.put(&marker, Box::pin(content)).await?;
    let found = a.get(&marker).await;
    b.delete(&marker).await?;
    Ok(found?.is_some())
}

/// Copies every file under `prefix` from `source` to `target` (overwriting files with the same key,
/// deleting extra files under `prefix` at `target`), then compares file counts. Returns the number of files
/// copied; `0` when both sides are the same store.
pub(super) async fn copy_files(
    source: &dyn Storage,
    target: &dyn Storage,
    prefix: &str,
) -> Result<u64, Error> {
    if same_store(source, target, prefix).await? {
        return Ok(0);
    }
    let keys = source.keys_with_prefix(prefix).await?;
    let wanted: std::collections::HashSet<&str> = keys.iter().map(String::as_str).collect();
    for stale in target.keys_with_prefix(prefix).await? {
        if !wanted.contains(stale.as_str()) {
            target.delete(&stale).await?;
        }
    }
    for key in &keys {
        // File biến mất giữa lúc liệt kê và lúc đọc (khách `moving` không ghi gì; chỉ job dọn có
        // thể xoá): bỏ qua, phép so bên dưới bắt lệch.
        let Some(file) = source.get(key).await? else { continue };
        target.put(key, file.content).await?;
    }
    let copied = target.keys_with_prefix(prefix).await?.len();
    if copied != keys.len() {
        return Err(errors::tenant_move_count_mismatch("files", keys.len() as u64, copied as u64));
    }
    Ok(keys.len() as u64)
}

/// Deletes every file under `prefix` at `doomed`, unless `doomed` is really the same store as `kept`.
/// Returns the number of files deleted.
pub(super) async fn delete_files(
    doomed: &dyn Storage,
    kept: &dyn Storage,
    prefix: &str,
) -> Result<u64, Error> {
    if same_store(doomed, kept, prefix).await? {
        return Ok(0);
    }
    let keys = doomed.keys_with_prefix(prefix).await?;
    for key in &keys {
        doomed.delete(key).await?;
    }
    Ok(keys.len() as u64)
}

/// Compares the tenant's row counts in the two databases, table by table; the target has exactly one extra
/// event `kernel.tenant_moved`.
pub(super) async fn verify(
    tables: &TenantTables,
    source: &Database,
    target: &Database,
    tenant: TenantId,
) -> Result<(), Error> {
    let before = tables.count(source, tenant).await.map_err(transfer)?;
    let after = tables.count(target, tenant).await.map_err(transfer)?;
    for (source, target) in before.iter().zip(&after) {
        let expected = source.rows + u64::from(source.table == KERNEL_EVENTS);
        if target.rows != expected {
            return Err(errors::tenant_move_count_mismatch(
                &source.table,
                source.rows,
                target.rows,
            ));
        }
    }
    Ok(())
}
