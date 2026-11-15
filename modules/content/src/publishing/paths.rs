//! Đường dẫn công khai và cây trang (D-101, D-102).
//!
//! - Đường dẫn do máy chủ tính: trang = đường dẫn công khai của trang cha + `/` + slug (trang
//!   gốc: `/` + slug); bài = `/<tiền tố>/` + slug. Không có tên miền hay tiền tố ngôn ngữ.
//! - Mọi thay đổi cây / đường dẫn của một ngôn ngữ (đăng, gỡ, chuyển, xoá) bắt đầu
//!   transaction bằng việc khoá dòng của ngôn ngữ đó trong `content_tree_locks`
//!   (`rows::lock_one`, D-181; cột `version` không còn dùng): các thay đổi chạy lần lượt —
//!   hai lệnh chuyển chéo nhau không cùng qua được bước kiểm vòng lặp; SQLite vốn chỉ có một
//!   người ghi. Mỗi khách một bộ dòng khoá (bước 33): khách này không chờ khách kia.

use std::collections::HashMap;

use autoxide_kernel::error::Error;
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use chrono::{DateTime, Utc};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

use crate::entity::{published, redirect, tree_lock};
use crate::entries::{Entry, Kind};

/// Khoá cây của ngôn ngữ `locale` tới hết `transaction`.
pub(crate) async fn lock_tree<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    locale: &str,
) -> Result<(), Error> {
    // Dòng khoá của (khách, ngôn ngữ) tạo khi cần lần đầu (khách mới không phải gieo trước).
    tree_lock::Entity::insert(tree_lock::ActiveModel {
        tenant_id: Set(tenant.id().as_uuid()),
        locale: Set(locale.to_owned()),
        version: Set(0),
    })
    .on_conflict(
        OnConflict::columns([tree_lock::Column::TenantId, tree_lock::Column::Locale])
            .do_nothing()
            .to_owned(),
    )
    .exec_without_returning(transaction)
    .await?;
    rows::lock_one(
        transaction,
        tree_lock::Entity::find().scoped(tenant).filter(tree_lock::Column::Locale.eq(locale)),
    )
    .await?;
    Ok(())
}

/// Đường dẫn công khai của `entry` (bản nháp hiện tại) — trang cha phải đang công khai.
/// `Err(None)` khi trang cha chưa công khai.
pub(crate) async fn path_of<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    entry: &Entry,
    post_prefix: &str,
) -> Result<Result<String, ()>, Error> {
    if entry.kind == Kind::Post.as_str() {
        return Ok(Ok(format!("/{post_prefix}/{}", entry.slug)));
    }
    let Some(parent) = entry.parent_id else {
        return Ok(Ok(format!("/{}", entry.slug)));
    };
    Ok(
        match published::Entity::find()
            .scoped(tenant)
            .filter(published::Column::EntryId.eq(parent))
            .one(connection)
            .await?
        {
            Some(parent) => Ok(format!("{}/{}", parent.path, entry.slug)),
            None => Err(()),
        },
    )
}

/// Bản công khai của mọi trang con, cháu… (đang công khai) của trang `id`.
pub(crate) async fn published_descendants<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<Vec<published::Model>, Error> {
    let mut found = Vec::new();
    let mut current = vec![id];
    for _ in 0..=crate::entries::MAX_TREE_DEPTH {
        if current.is_empty() {
            break;
        }
        let children = published::Entity::find()
            .scoped(tenant)
            .filter(published::Column::ParentId.is_in(current.clone()))
            .all(connection)
            .await?;
        current = children.iter().map(|child| child.entry_id).collect();
        found.extend(children);
    }
    Ok(found)
}

/// Một đường dẫn công khai đổi: (mục, đường dẫn cũ, đường dẫn mới).
pub(crate) type Moved = (Uuid, String, String);

/// Đổi đường dẫn công khai của các trang con khi đường dẫn của cha đổi từ `old` sang `new`;
/// trả danh sách thay đổi (để ghi chuyển hướng và event).
pub(crate) async fn repath_descendants<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    id: Uuid,
    old: &str,
    new: &str,
    at: DateTime<Utc>,
) -> Result<Vec<Moved>, Error> {
    let mut moved = Vec::new();
    for child in published_descendants(connection, tenant, id).await? {
        let Some(rest) = child.path.strip_prefix(old) else { continue };
        let path = format!("{new}{rest}");
        // Đường dẫn mới của trang con có thể trùng một bản công khai khác (vd bài viết ở
        // `/blog/x`): báo `content.path_taken` như với chính trang được đổi, không để câu
        // `UPDATE` vấp unique rồi thành lỗi 500.
        if path_used(connection, tenant, &child.locale, &path, child.entry_id).await? {
            return Err(crate::publishing::path_taken(&path));
        }
        published::Entity::update_many()
            .scoped(tenant)
            .col_expr(published::Column::Path, Expr::value(path.clone()))
            .col_expr(published::Column::UpdatedAt, Expr::value(at))
            .filter(published::Column::EntryId.eq(child.entry_id))
            .exec(connection)
            .await?;
        moved.push((child.entry_id, child.path, path));
    }
    Ok(moved)
}

/// Đường dẫn `path` (ngôn ngữ `locale`) đã có bản công khai nào khác `except` dùng chưa.
pub(crate) async fn path_used<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    locale: &str,
    path: &str,
    except: Uuid,
) -> Result<bool, Error> {
    Ok(published::Entity::find()
        .scoped(tenant)
        .filter(published::Column::Locale.eq(locale))
        .filter(published::Column::Path.eq(path))
        .filter(published::Column::EntryId.ne(except))
        .one(connection)
        .await?
        .is_some())
}

/// Ở `path` (ngôn ngữ `locale`) có chuyển hướng làm tay (`manual`) không.
pub(crate) async fn manual_redirect_at<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    locale: &str,
    path: &str,
) -> Result<bool, Error> {
    Ok(redirect::Entity::find()
        .scoped(tenant)
        .filter(redirect::Column::Locale.eq(locale))
        .filter(redirect::Column::FromPath.eq(path))
        .filter(redirect::Column::Source.eq("manual"))
        .one(connection)
        .await?
        .is_some())
}

/// Ghi chuyển hướng 301 tự động từ đường dẫn cũ tới mục (theo ID) cho mỗi thay đổi trong
/// `moved`; bỏ chuyển hướng tự động / "đã gỡ" đang chiếm đường dẫn mới.
pub(crate) async fn redirect_moved<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    locale: &str,
    moved: &[Moved],
    by: Option<Uuid>,
    at: DateTime<Utc>,
) -> Result<(), Error> {
    for (entry_id, old, new) in moved {
        clear_automatic(connection, tenant, locale, new).await?;
        let row = Generated {
            locale,
            from: old,
            to_entry: Some(*entry_id),
            status: 301,
            source: "auto",
            by,
            at,
        };
        put_redirect(connection, tenant, row).await?;
    }
    Ok(())
}

/// Bỏ chuyển hướng tự động hay "đã gỡ" ở `path` (chuyển hướng làm tay thì giữ).
pub(crate) async fn clear_automatic<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    locale: &str,
    path: &str,
) -> Result<(), Error> {
    redirect::Entity::delete_many()
        .scoped(tenant)
        .filter(redirect::Column::Locale.eq(locale))
        .filter(redirect::Column::FromPath.eq(path))
        .filter(redirect::Column::Source.is_in(["auto", "gone"]))
        .exec(connection)
        .await?;
    Ok(())
}

/// Một chuyển hướng máy chủ tự ghi (`auto`: 301 tới mục; `gone`: 410).
pub(crate) struct Generated<'a> {
    pub locale: &'a str,
    pub from: &'a str,
    pub to_entry: Option<Uuid>,
    pub status: i32,
    pub source: &'a str,
    pub by: Option<Uuid>,
    pub at: DateTime<Utc>,
}

/// Ghi (thay) chuyển hướng tự động ở `row.from` — chuyển hướng làm tay đang có thì giữ nguyên.
pub(crate) async fn put_redirect<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    row: Generated<'_>,
) -> Result<(), Error> {
    clear_automatic(connection, tenant, row.locale, row.from).await?;
    redirect::Entity::insert(redirect::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant.id().as_uuid()),
        locale: Set(row.locale.to_owned()),
        from_path: Set(row.from.to_owned()),
        to_entry_id: Set(row.to_entry),
        to_url: Set(None),
        status_code: Set(row.status),
        source: Set(row.source.to_owned()),
        created_by: Set(row.by),
        created_at: Set(row.at),
        updated_at: Set(row.at),
    })
    .on_conflict(
        OnConflict::columns([
            redirect::Column::TenantId,
            redirect::Column::Locale,
            redirect::Column::FromPath,
        ])
        .do_nothing()
        .to_owned(),
    )
    .exec_without_returning(connection)
    .await?;
    Ok(())
}

/// Chuẩn hoá đường dẫn khách gửi: bắt đầu bằng `/`, không `/` ở cuối (trừ `/`), không phần
/// rỗng, `.` hay `..`. `None` nếu sai dạng.
pub(crate) fn normalize(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if !raw.starts_with('/')
        || raw.len() > 2048
        // `\` thành `/` trong trình duyệt: `/\evil.com` là địa chỉ ngoài site.
        || raw.chars().any(|c| c.is_control() || c == '?' || c == '#' || c == '\\')
    {
        return None;
    }
    let parts: Vec<&str> = raw.split('/').filter(|part| !part.is_empty()).collect();
    if parts.iter().any(|part| *part == "." || *part == "..") {
        return None;
    }
    Some(format!("/{}", parts.join("/")))
}

/// `raw` là một đường dẫn trong site dùng được làm đích link (chuyển hướng, menu): hợp lệ theo
/// [`normalize`] và không bắt đầu bằng `//` — trình duyệt hiểu `//evil.com` là địa chỉ của site
/// khác (URL không có scheme), dù `normalize` gộp nó thành `/evil.com`. Cùng luật với link trong
/// thân bài (`body::safe_link`). (`/\` đã bị `normalize` từ chối.)
pub(crate) fn is_site_link(raw: &str) -> bool {
    !raw.trim().starts_with("//") && normalize(raw).is_some()
}

/// Đường dẫn công khai của các mục trong `ids` (mục không công khai thì không có mặt).
pub(crate) async fn paths<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, String>, Error> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(published::Entity::find()
        .scoped(tenant)
        .filter(published::Column::EntryId.is_in(ids.to_vec()))
        .all(connection)
        .await?
        .into_iter()
        .map(|row| (row.entry_id, row.path))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chuan_hoa_duong_dan() {
        assert_eq!(normalize("/a/b/").as_deref(), Some("/a/b"));
        assert_eq!(normalize("/").as_deref(), Some("/"));
        assert_eq!(normalize("//a//b").as_deref(), Some("/a/b"));
        for bad in ["a/b", "/a/../b", "/a?x=1", "", "/\\evil.com", "/a\\b"] {
            assert_eq!(normalize(bad), None, "{bad}");
        }
    }

    #[test]
    fn link_trong_site_khong_nhan_dia_chi_khong_scheme() {
        assert!(is_site_link("/about/team"));
        assert!(is_site_link("/"));
        for bad in ["//evil.com", " //evil.com/x", "/\\evil.com", "\\\\evil.com", "evil.com"] {
            assert!(!is_site_link(bad), "{bad}");
        }
    }
}
