//! `cargo xtask new-module <module> <entity>`: sinh một module mới từ khuôn `modules/notes`
//! (D-165, mượn generator của Loco / Rails — D-164).
//!
//! Vì sao chép `notes` rồi đổi tên thay vì giữ một bộ template riêng: `notes` đã dùng đủ mọi
//! phần của hợp đồng module và đi qua gate mỗi lần chạy, nên khuôn không bao giờ cũ hay hỏng
//! âm thầm. Đổi tên là thay đúng sáu dạng chữ (`notes`, `note`, `Notes`, `Note`, `NOTES`,
//! `NOTE`) trong một lượt quét — tên mới chứa `note` cũng không bị thay lần hai.

use std::fs;
use std::path::{Path, PathBuf};

/// Khuôn nằm ở đây, tương đối với gốc workspace.
const TEMPLATE: &str = "modules/notes";
/// Tên số nhiều và số ít của khuôn.
const TEMPLATE_MODULE: &str = "notes";
const TEMPLATE_ENTITY: &str = "note";

/// Thư mục của khuôn không chép sang (bộ đo hiệu năng của core chỉ có ở `notes`, D-071).
const SKIPPED_DIRS: [&str; 1] = ["benches"];

/// Tên không dùng được: tiền tố bảng hệ thống của database (D-038), crate sẵn có, từ khoá Rust.
const RESERVED: [&str; 12] = [
    "sqlite", "pg", "kernel", "testkit", "app", "xtask", "api", "crate", "self", "super", "mod",
    "type",
];

/// Một dạng chữ của khuôn và chữ thay nó.
struct Rename {
    from: String,
    to: String,
}

/// Kiểm tên rồi sinh module vào `<root>/modules/<module>`. Trả về các file đã ghi.
pub fn new_module(root: &Path, module: &str, entity: &str) -> Result<Vec<PathBuf>, String> {
    let existing = existing_modules(&root.join("modules"))?;
    validate(module, entity, &existing)?;
    let target = root.join("modules").join(module);
    generate(&root.join(TEMPLATE), &target, module, entity)
}

/// Tên module hiện có trong `modules/`.
fn existing_modules(dir: &Path) -> Result<Vec<String>, String> {
    let entries =
        fs::read_dir(dir).map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
        if entry.path().is_dir() {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    Ok(names)
}

/// Luật đặt tên của D-038, cộng: số ít khác số nhiều, không đụng module sẵn có.
pub fn validate(module: &str, entity: &str, existing: &[String]) -> Result<(), String> {
    for (what, name) in [("module", module), ("entity", entity)] {
        let mut chars = name.chars();
        let starts_with_letter = chars.next().is_some_and(|c| c.is_ascii_lowercase());
        if !starts_with_letter
            || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            || name.ends_with('_')
            || name.contains("__")
        {
            return Err(format!(
                "{what} name `{name}` must start with a lowercase letter and use only lowercase \
                 letters, digits and single `_` (e.g. `bookmarks`)"
            ));
        }
        if RESERVED.contains(&name) {
            return Err(format!("{what} name `{name}` is reserved; choose another"));
        }
    }
    if module == entity {
        return Err(format!(
            "the entity name must differ from the module name: the module is the collection \
             (`bookmarks`), the entity one record (`bookmark`); got `{module}` for both"
        ));
    }
    for other in existing {
        if other == module {
            return Err(format!("module `{module}` already exists in modules/{module}"));
        }
        // Tên module là tiền tố bảng: `a` và `a_b` sẽ giẫm bảng của nhau (D-038).
        if other.starts_with(&format!("{module}_")) || module.starts_with(&format!("{other}_")) {
            return Err(format!(
                "module `{module}` and existing module `{other}` would share a table prefix; \
                 choose a name that is not `{other}_…` or a prefix of it"
            ));
        }
    }
    Ok(())
}

/// Chép khuôn `template` sang `target`, đổi tên trong nội dung và tên file.
pub fn generate(
    template: &Path,
    target: &Path,
    module: &str,
    entity: &str,
) -> Result<Vec<PathBuf>, String> {
    if target.exists() {
        return Err(format!("{} already exists", target.display()));
    }
    let renames = renames(module, entity);
    let mut written = Vec::new();
    for relative in files(template, Path::new(""))? {
        let source = template.join(&relative);
        let text = fs::read_to_string(&source)
            .map_err(|error| format!("cannot read {}: {error}", source.display()))?;
        let text = rename(&text, &renames);
        let text = match relative.to_str() {
            Some("Cargo.toml") => manifest(&text, module)?,
            Some("src/lib.rs") => crate_doc(&text, module)?,
            _ => text,
        };
        let destination = target.join(rename(&relative.to_string_lossy(), &renames));
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        }
        fs::write(&destination, text)
            .map_err(|error| format!("cannot write {}: {error}", destination.display()))?;
        written.push(destination);
    }
    Ok(written)
}

/// Mọi file của khuôn (đường dẫn tương đối, đã sắp), trừ thư mục bỏ qua.
fn files(root: &Path, relative: &Path) -> Result<Vec<PathBuf>, String> {
    let dir = root.join(relative);
    let entries =
        fs::read_dir(&dir).map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = relative.join(&name);
        if entry.path().is_dir() {
            if relative.as_os_str().is_empty() && SKIPPED_DIRS.contains(&name.as_str()) {
                continue;
            }
            found.extend(files(root, &path)?);
        } else {
            found.push(path);
        }
    }
    found.sort();
    Ok(found)
}

/// Sáu dạng chữ, dạng dài trước dạng ngắn (`Notes` trước `Note`).
fn renames(module: &str, entity: &str) -> Vec<Rename> {
    let forms = |name: &str| [pascal(name), name.to_ascii_uppercase(), name.to_owned()];
    let [module_pascal, module_upper, module_lower] = forms(module);
    let [entity_pascal, entity_upper, entity_lower] = forms(entity);
    let [template_pascal, template_upper, template_lower] = forms(TEMPLATE_MODULE);
    let [one_pascal, one_upper, one_lower] = forms(TEMPLATE_ENTITY);
    vec![
        Rename { from: template_pascal, to: module_pascal },
        Rename { from: one_pascal, to: entity_pascal },
        Rename { from: template_upper, to: module_upper },
        Rename { from: one_upper, to: entity_upper },
        Rename { from: template_lower, to: module_lower },
        Rename { from: one_lower, to: entity_lower },
    ]
}

/// `reading_lists` → `ReadingLists`.
fn pascal(name: &str) -> String {
    name.split('_')
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_ascii_uppercase().to_string() + chars.as_str()
            })
        })
        .collect()
}

/// Thay trong một lượt quét: chỗ đã thay không bị xét lại.
fn rename(text: &str, renames: &[Rename]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    'scan: while !rest.is_empty() {
        for Rename { from, to } in renames {
            if let Some(after) = rest.strip_prefix(from.as_str()) {
                out.push_str(to);
                rest = after;
                continue 'scan;
            }
        }
        // Chép nguyên một ký tự (không cắt giữa ký tự UTF-8 nhiều byte).
        let mut chars = rest.chars();
        if let Some(c) = chars.next() {
            out.push(c);
        }
        rest = chars.as_str();
    }
    out
}

/// `Cargo.toml` của module mới: mô tả mới, bỏ phần bộ đo hiệu năng chỉ `notes` có.
fn manifest(text: &str, module: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut description = false;
    let mut in_bench = false;
    for line in text.lines() {
        if line.starts_with("description = ") {
            out.push_str(&format!(
                "description = \"Autoxide module `{module}` (generated by `cargo xtask new-module`).\"\n"
            ));
            description = true;
            continue;
        }
        // Phần bench: dòng chú thích ngay trước `[[bench]]` và bảng đó, tới bảng kế tiếp.
        if line.starts_with("# ") && line.contains("cargo bench") {
            in_bench = true;
            continue;
        }
        if in_bench {
            if line.starts_with('[') && line != "[[bench]]" {
                in_bench = false;
            } else {
                continue;
            }
        }
        // `tower` chỉ bộ đo hiệu năng dùng.
        if line == "tower.workspace = true" {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !description || out.contains("[[bench]]") {
        return Err("the template's Cargo.toml changed shape; update xtask/src/scaffold.rs".into());
    }
    Ok(out)
}

/// Chú thích đầu `lib.rs`: thay phần giới thiệu khuôn bằng lời cho module mới.
fn crate_doc(text: &str, module: &str) -> Result<String, String> {
    let body = text.trim_start_matches(|_| false);
    let start = body.lines().take_while(|line| line.starts_with("//!")).count();
    if start == 0 {
        return Err(
            "the template's lib.rs has no `//!` header; update xtask/src/scaffold.rs".into()
        );
    }
    let rest: Vec<&str> = text.lines().skip(start).collect();
    Ok(format!(
        "//! Module `{module}`.\n//!\n//! Sinh bằng `cargo xtask new-module` (D-165). Việc còn lại: viết lại\n//! bảng, service, mô tả, câu thông báo và bản dịch `locales/vi.json` cho đúng nghiệp vụ —\n//! xem `docs/MODULE_GUIDE.md`.\n{}\n",
        rest.join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn existing() -> Vec<String> {
        ["notes", "auth", "media"].iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn ten_hop_le_va_khong_hop_le() {
        assert!(validate("bookmarks", "bookmark", &existing()).is_ok());
        assert!(validate("reading_lists", "reading_list", &existing()).is_ok());
        for (module, entity) in [
            ("Bookmarks", "bookmark"),
            ("1x", "x"),
            ("a-b", "ab"),
            ("a__b", "ab"),
            ("pg", "row"),
            ("media", "item"),
            ("auth_extra", "thing"),
            ("bookmarks", "bookmarks"),
            ("bookmarks", "type"),
        ] {
            assert!(validate(module, entity, &existing()).is_err(), "{module}/{entity}");
        }
    }

    #[test]
    fn doi_ten_mot_luot_khong_thay_lai() {
        let renames = renames("notebooks", "notebook");
        assert_eq!(
            rename("NotesModule notes_note NOTE_CREATED NoteService", &renames),
            "NotebooksModule notebooks_notebook NOTEBOOK_CREATED NotebookService"
        );
        assert_eq!(pascal("reading_lists"), "ReadingLists");
    }

    /// Sinh thật từ `modules/notes` vào thư mục tạm: không còn chữ `note` nào, không có bench,
    /// đủ file nguồn — khuôn đổi hình dạng thì test này đỏ trước khi người dùng gặp lỗi.
    #[test]
    fn sinh_tu_khuon_notes_that() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let target = std::env::temp_dir()
            .join(format!("autoxide-scaffold-{}", std::process::id()))
            .join("bookmarks");
        let _ = fs::remove_dir_all(&target);
        let written = generate(&root.join(TEMPLATE), &target, "bookmarks", "bookmark").unwrap();

        let relative: Vec<String> = written
            .iter()
            .map(|path| path.strip_prefix(&target).unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(relative.contains(&"Cargo.toml".to_owned()));
        assert!(relative.contains(&"src/lib.rs".to_owned()));
        assert!(relative.contains(&"locales/vi.json".to_owned()));
        assert!(relative.iter().any(|path| path.ends_with("create_bookmark.rs")), "{relative:?}");
        assert!(!relative.iter().any(|path| path.starts_with("benches")));
        for path in &written {
            let text = fs::read_to_string(path).unwrap();
            assert!(!text.to_lowercase().contains("note"), "`note` left in {}", path.display());
        }
        let manifest = fs::read_to_string(target.join("Cargo.toml")).unwrap();
        assert!(manifest.contains("name = \"autoxide-bookmarks\""));
        assert!(!manifest.contains("bench") && !manifest.contains("tower"), "{manifest}");
        let lib = fs::read_to_string(target.join("src/lib.rs")).unwrap();
        assert!(lib.starts_with("//! Module `bookmarks`."), "{lib}");
        assert!(lib.contains("pub fn module() -> impl Module"));
        fs::remove_dir_all(target.parent().unwrap()).unwrap();
    }
}
