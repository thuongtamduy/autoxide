//! Chép, xoá, so file lưu trữ của khách giữa hai nơi lưu.

use super::*;

/// Tiền tố khoá file của khách `tenant` (không phải khách mặc định — khách đó không dời được).
pub(super) fn tenant_prefix(tenant: Uuid) -> String {
    format!("tenants/{tenant}/")
}

/// `a` và `b` thật ra là cùng một nơi lưu: ghi một file đánh dấu (dưới `prefix`) vào `b` rồi tìm nó
/// ở `a`. Cấu hình khác nhau vẫn có thể trỏ cùng một chỗ (vd thư mục của nơi lưu chung) — khi đó
/// chép hay xoá "nguồn" là xoá file đang dùng.
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

/// Chép mọi file dưới `prefix` từ `source` sang `target` (đè file cùng khoá, xoá file thừa dưới
/// `prefix` ở `target`), rồi so số file. Trả số file đã chép; `0` khi hai bên là cùng một nơi lưu.
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

/// Xoá mọi file dưới `prefix` ở `doomed` — trừ khi `doomed` thật ra là cùng nơi lưu với `kept`.
/// Trả số file đã xoá.
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

/// So số dòng của khách ở hai database, từng bảng; đích có thêm đúng một event
/// `kernel.tenant_moved`.
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
