//! Kiểm "chỉ cấp được quyền mình có" khi gán role (membership, lời mời, SCIM, OIDC): so cả tên
//! permission LẪN phạm vi mà role sẽ phủ ở nút được gán (D-20261121-grant-scope-check).

use autoxide_kernel::access::Grant;

use super::*;

/// Vì sao `actor` không gán được các role `roles` ở nút `org_id` (rỗng: gán được): nút phải có
/// trong khách và trong phạm vi của người gán, role phải có, `admin` chỉ ở gốc, và người gán phải
/// có mọi permission của role với phạm vi bao được phạm vi role sẽ phủ khi gán ở `org_id` (không
/// tự nâng quyền — cả về tên lẫn về độ rộng). `field`: tiền tố tên ô (`memberships[0].`).
/// Dùng cho membership, lời mời, luật nhóm SCIM và role mặc định của nhà cung cấp OIDC.
pub(crate) async fn grant_problems<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    actor: &Principal,
    field: &str,
    org_id: Uuid,
    roles: &BTreeSet<Uuid>,
) -> Result<Vec<FieldError>, Error> {
    let mut problems = Vec::new();
    let root = root_of(tenant.id());
    let org = OrgId::from_uuid(org_id);
    let ancestors = OrgTree::ancestors(connection, tenant.id(), org).await?;
    if org != root && ancestors.is_empty() {
        problems.push(FieldError::new(
            format!("{field}org_id"),
            "unknown",
            "is not an organization unit of this tenant",
        ));
        return Ok(problems);
    }
    if !actor.reaches(connection, org).await? {
        problems.push(FieldError::new(
            format!("{field}org_id"),
            "out_of_scope",
            "must be a unit you work in (one of your roles covers it)",
        ));
    }
    // Chuỗi tổ tiên của nút (kể cả chính nó và gốc — gốc có thể chưa có dòng trong cây).
    let mut chain = ancestors;
    chain.extend([org, root]);
    let found = role::Entity::find()
        .filter(role::Column::Id.is_in(roles.iter().copied()))
        .scoped(tenant)
        .all(connection)
        .await?;
    let mut grants = permissions_of(connection, tenant, found.iter().map(|row| row.id)).await?;
    for role_id in roles {
        let Some(row) = found.iter().find(|row| row.id == *role_id) else {
            problems.push(FieldError::new(
                format!("{field}role_ids"),
                "unknown",
                "every entry must be the id of an existing role",
            ));
            continue;
        };
        if row.all_permissions && org != root {
            problems.push(FieldError::new(
                format!("{field}role_ids"),
                "root_only",
                "the `admin` role can only be given at the root unit",
            ));
        }
        let covers = if row.all_permissions {
            // Role `admin` phủ mọi thứ cho cả khách: chỉ người toàn quyền, không bị rào, cấp được.
            actor.has_all_permissions() && actor.fence().is_none()
        } else {
            grants
                .remove(&row.id)
                .unwrap_or_default()
                .grants
                .iter()
                .all(|(permission, scope)| actor_covers(actor, org, &chain, permission, *scope))
        };
        if !covers {
            problems.push(
                FieldError::new(
                    format!("{field}role_ids"),
                    "more_than_you_have",
                    "role `{role}` has permissions you do not have, or has them more widely \
                     than you do; you cannot give it",
                )
                .with_param("role", &row.name),
            );
        }
    }
    Ok(problems)
}

/// Người gán `actor` có `permission` với phạm vi bao được grant độ sâu `needed` khi gán ở nút
/// `target` (`chain`: các nút tổ tiên của `target`, gồm chính nó và gốc).
fn actor_covers(
    actor: &Principal,
    target: OrgId,
    chain: &[OrgId],
    permission: &str,
    needed: Scope,
) -> bool {
    if !actor.allows(permission) {
        return false;
    }
    // API key gắn nút: mọi quyền chỉ có hiệu lực trong nhánh rào — không cấp được thứ phủ cả khách
    // hay nút ngoài rào.
    if let Some(fence) = actor.fence()
        && (needed == Scope::Tenant || !chain.contains(&fence))
    {
        return false;
    }
    let Some(map) = actor.grant_map() else {
        // Toàn quyền: mọi permission ở độ sâu `tenant`.
        return true;
    };
    let Some(held) = map.get(permission) else { return false };
    held.iter().any(|grant| grant_covers(*grant, target, chain, needed))
}

/// Một grant của người gán (`held`) có bao được grant độ sâu `needed` ở nút `target` không.
/// Thứ tự rộng dần `own < unit < subtree < tenant`:
/// - `tenant` của người gán bao mọi thứ; cần `tenant` thì chỉ `tenant` bao được.
/// - `subtree` ở một tổ tiên (hay chính nút) bao `subtree` / `unit` / `own` ở nút đó.
/// - `unit` ở đúng nút bao `unit` / `own`; `own` ở đúng nút chỉ bao `own`.
fn grant_covers(held: Grant, target: OrgId, chain: &[OrgId], needed: Scope) -> bool {
    match (held.scope, needed) {
        (Scope::Tenant, _) => true,
        (_, Scope::Tenant) => false,
        (Scope::Subtree, _) => chain.contains(&held.org),
        (Scope::Unit, Scope::Unit | Scope::Own) | (Scope::Own, Scope::Own) => target == held.org,
        (Scope::Unit | Scope::Own, _) => false,
    }
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::access::{Grant, Scope};
    use autoxide_kernel::tenancy::OrgId;
    use uuid::Uuid;

    use super::grant_covers;

    #[test]
    fn grant_hep_khong_bao_duoc_grant_rong() {
        let (root, a, a1, b) = (org(), org(), org(), org());
        // Gán ở A1: chuỗi tổ tiên A1 → A → gốc.
        let chain = [a1, a, root];
        let at = a1;
        let subtree_a = Grant::new(a, Scope::Subtree);
        assert!(grant_covers(subtree_a, at, &chain, Scope::Subtree));
        assert!(grant_covers(subtree_a, at, &chain, Scope::Unit));
        assert!(!grant_covers(subtree_a, at, &chain, Scope::Tenant), "nhánh không cấp cả khách");
        let subtree_b = Grant::new(b, Scope::Subtree);
        assert!(!grant_covers(subtree_b, at, &chain, Scope::Unit), "nhánh khác");
        let unit_a1 = Grant::new(a1, Scope::Unit);
        assert!(grant_covers(unit_a1, at, &chain, Scope::Unit));
        assert!(grant_covers(unit_a1, at, &chain, Scope::Own));
        assert!(!grant_covers(unit_a1, at, &chain, Scope::Subtree));
        let unit_a = Grant::new(a, Scope::Unit);
        assert!(!grant_covers(unit_a, at, &chain, Scope::Unit), "unit chỉ ở đúng nút");
        let own_a1 = Grant::new(a1, Scope::Own);
        assert!(grant_covers(own_a1, at, &chain, Scope::Own));
        assert!(!grant_covers(own_a1, at, &chain, Scope::Unit));
        let tenant = Grant::new(b, Scope::Tenant);
        assert!(grant_covers(tenant, at, &chain, Scope::Tenant));
    }

    fn org() -> OrgId {
        OrgId::from_uuid(Uuid::now_v7())
    }
}
