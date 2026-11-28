//! The catalog of declared settings, the checks on module declarations, and copying settings to a sandbox.
//! Moved out of `settings.rs` unchanged (D-20261118-file-length).

use super::*;

/// Registry of the declared settings of every module, filled once when `Modules` is built. Cheap to clone.
#[derive(Clone, Default)]
pub(crate) struct Catalog(Arc<OnceLock<Vec<TenantSetting>>>);

impl Catalog {
    pub(crate) fn fill(&self, settings: Vec<TenantSetting>) {
        let _ = self.0.set(settings);
    }

    pub(super) fn all(&self) -> &[TenantSetting] {
        self.0.get().map_or(&[], Vec::as_slice)
    }

    /// The setting named `name`, current or former ([`TenantSetting::formerly`]).
    pub(super) fn find(&self, name: &str) -> Result<TenantSetting, Error> {
        self.all()
            .iter()
            .find(|setting| setting.answers_to(name))
            .copied()
            .ok_or_else(|| super::super::errors::tenant_setting_unknown(name))
    }
}

/// `name` has the form `<module>.<key>`; the key is one or more segments (lowercase, digits, `_`) joined by `.`.
fn well_named(module: &str, name: &str) -> bool {
    name.len() <= 100
        && name
            .strip_prefix(module)
            .and_then(|rest| rest.strip_prefix('.'))
            .is_some_and(|key| key.split('.').all(crate::module::is_identifier))
}

/// Checks the declarations of module `module`: name `<module>.<key>`, no duplicates (former names included);
/// settings of the `connection` group, and only they, have the name `<module>.connection.<kind>.<field>`.
pub(crate) fn declaration_problems(
    module: &str,
    settings: &[TenantSetting],
    seen: &mut Vec<&'static str>,
) -> Vec<String> {
    let mut problems = Vec::new();
    for setting in settings {
        if !well_named(module, setting.name) {
            problems.push(format!(
                "tenant setting `{}` of module `{module}` must be named `{module}.<key>` (lower \
                 case letters, digits and `_`; several parts are joined with `.`; at most 100 \
                 characters)",
                setting.name
            ));
        }
        if setting.description.trim().is_empty() {
            problems.push(format!("tenant setting `{}` has no description", setting.name));
        }
        if let Some(group) = setting.group
            && !crate::module::is_identifier(group)
        {
            problems.push(format!(
                "tenant setting `{}` is in group `{group}`: a group is lower case letters, digits \
                 and `_`",
                setting.name
            ));
        }
        // `<module>.connection.<loại>.<trường>`: ít nhất hai đoạn sau `connection`.
        let mut parts = setting.name.split('.').skip(1);
        let connection_name = parts.next() == Some(CONNECTION_GROUP);
        let in_group = setting.group == Some(CONNECTION_GROUP);
        if in_group && !(connection_name && parts.count() >= 2) {
            problems.push(format!(
                "tenant setting `{}` is in group `connection`, so it must be named \
                 `{module}.connection.<kind>.<field>`",
                setting.name
            ));
        } else if connection_name && !in_group {
            problems.push(format!(
                "tenant setting `{}` is named like a partner connection: declare it with \
                 `.group(CONNECTION_GROUP)`",
                setting.name
            ));
        }
        if let Some(former) = setting.former_name
            && !well_named(module, former)
        {
            problems.push(format!(
                "tenant setting `{}` has the former name `{former}`, which is not a name of \
                 module `{module}`",
                setting.name
            ));
        }
        for name in std::iter::once(setting.name).chain(setting.former_name) {
            if seen.contains(&name) {
                problems.push(format!("tenant setting `{name}` is declared twice"));
            }
            seen.push(name);
        }
    }
    problems
}

/// Copies every NON-secret setting from `source` to `target` (sandbox, D-232): a secret (partner connection
/// information) never crosses; the sandbox uses fakes. A setting `target` already has is kept. Returns the
/// number of new rows.
pub(crate) async fn copy_plain(
    source: &TenantContext,
    target: &TenantContext,
) -> Result<u64, Error> {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};

    let rows = entity::Entity::find()
        .filter(entity::Column::TenantId.eq(source.id().as_uuid()))
        .filter(entity::Column::Secret.eq(false))
        .all(source.database().read())
        .await?;
    let mut copied = 0;
    for row in rows {
        let insert = entity::Entity::insert(entity::ActiveModel {
            tenant_id: Set(target.id().as_uuid()),
            name: Set(row.name),
            value: Set(row.value),
            secret: Set(false),
            updated_by: Set(None),
            updated_at: Set(row.updated_at),
        })
        .on_conflict(
            sea_orm::sea_query::OnConflict::columns([
                entity::Column::TenantId,
                entity::Column::Name,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(target.database().write())
        .await?;
        copied += insert;
    }
    Ok(copied)
}
