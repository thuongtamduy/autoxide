//! Luật `entity-tenant` (bước 33, D-253): entity SeaORM trong `src/` của kernel và của module là
//! bảng cấp khách hàng — có trường `tenant_id` — trừ bảng mặt điều khiển (module khai trong
//! `fn control_plane_tables`).
//!
//! Đọc mã nguồn theo dòng sau `cargo fmt` (như các luật trong `boundaries.rs`): `table_name = "…"`
//! rồi khối `pub struct Model { … }` ngay sau nó.

use std::fs;

use crate::boundaries::{Violation, rust_files};
use crate::workspace::{Role, Workspace};

pub fn check_entities(workspace: &Workspace) -> Result<Vec<Violation>, String> {
    let mut violations = Vec::new();
    for package in &workspace.packages {
        if !matches!(package.role, Role::Module | Role::Kernel) {
            continue;
        }
        let dir = workspace.root.join(&package.dir);
        let mut sources = Vec::new();
        for file in rust_files(&dir.join("src"))? {
            let text = read(&dir.join(&file))?;
            sources.push((format!("{}/src/{file}", package.dir), text));
        }
        let control: Vec<String> =
            sources.iter().flat_map(|(_, text)| control_plane_tables(text)).collect();
        for (file, text) in &sources {
            violations.extend(check_source(file, text, &control));
        }
    }
    Ok(violations)
}

/// Vi phạm của một file: entity không có `tenant_id` mà không được miễn.
pub fn check_source(file: &str, text: &str, control: &[String]) -> Vec<Violation> {
    let lines: Vec<&str> = text.lines().collect();
    let mut violations = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let Some(table) = table_name(line) else { continue };
        if control.iter().any(|name| name == table) {
            continue;
        }
        let model = lines[index..]
            .iter()
            .skip_while(|line| !line.contains("pub struct Model"))
            .take_while(|line| line.trim() != "}");
        if !model.into_iter().any(|line| line.trim_start().starts_with("pub tenant_id:")) {
            violations.push(Violation {
                rule: "entity-tenant",
                location: format!("{file}:{}", index + 1),
                message: format!(
                    "entity of table `{table}` has no `tenant_id`: tenant-level tables carry it \
                     (and `impl TenantScoped`); declare the table in `Module::control_plane_tables` \
                     if it is shared by the whole platform"
                ),
            });
        }
    }
    violations
}

/// `#[sea_orm(table_name = "x")]` → `x`.
fn table_name(line: &str) -> Option<&str> {
    let rest = line.split("table_name = \"").nth(1)?;
    rest.split('"').next()
}

/// Chuỗi trong thân `fn control_plane_tables`.
fn control_plane_tables(text: &str) -> Vec<String> {
    let Some(start) = text.find("fn control_plane_tables") else { return Vec::new() };
    let body = &text[start..];
    let body = &body[..body.find("\n    }").unwrap_or(body.len())];
    quoted(body)
}

fn quoted(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .flat_map(|line| line.split('"').skip(1).step_by(2).map(str::to_owned).collect::<Vec<_>>())
        .collect()
}

fn read(path: &std::path::Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("cannot read {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTITY: &str = r#"
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "shop_item")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub title: String,
}
"#;

    #[test]
    fn entity_thieu_tenant_id_bi_chan_tru_khi_duoc_mien() {
        let found = check_source("shop/src/entity.rs", ENTITY, &[]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].location, "shop/src/entity.rs:3");
        let with =
            ENTITY.replace("pub title: String,", "pub tenant_id: Uuid,\n    pub title: String,");
        assert!(check_source("f", &with, &[]).is_empty());
        assert!(check_source("f", ENTITY, &["shop_item".to_owned()]).is_empty());
    }

    #[test]
    fn doc_danh_sach_mien() {
        let module = "    fn control_plane_tables(&self) -> Vec<&'static str> {\n        // x\n        vec![\"a_b\", \"a_c\"]\n    }\n";
        assert_eq!(control_plane_tables(module), ["a_b", "a_c"]);
    }
}
