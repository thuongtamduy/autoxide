//! SDK di động (bước 69 P1, D-20261119-step69-sdk): Swift, Kotlin, Dart sinh từ `docs/openapi.json`
//! bằng công cụ chuẩn, ghim phiên bản. Gate không cần Swift hay JDK: phần này chỉ giữ cấu hình sinh
//! khớp với hợp đồng API; CI có job riêng sinh và biên dịch từng SDK.
//!
//! - Swift (`sdk/swift`): swift-openapi-generator của Apple chạy lúc build (plugin của SwiftPM) trên
//!   bản chép `Sources/Autoxide/openapi.json` — `cargo xtask sdk` chép, `--check` so.
//! - Kotlin, Dart: openapi-generator (generator `kotlin`, `dart-dio`) theo `sdk/openapitools.json`;
//!   mã sinh ra không commit. `--check` kiểm phiên bản gói trong đó bằng phiên bản workspace.

use std::path::Path;

use serde_json::Value;

/// Bản chép hợp đồng API cho plugin của SwiftPM.
pub const SWIFT_SPEC: &str = "sdk/swift/Sources/Autoxide/openapi.json";
/// Cấu hình openapi-generator cho Kotlin và Dart.
pub const GENERATORS: &str = "sdk/openapitools.json";

/// Sinh (chép bản Swift) hay kiểm (`check`). `Ok(false)`: lệch, đã báo lên stderr.
pub fn run(root: &Path, check: bool, workspace_version: &str) -> Result<bool, String> {
    let spec = std::fs::read_to_string(root.join("docs/openapi.json"))
        .map_err(|error| format!("cannot read docs/openapi.json: {error}"))?;
    let mut ok = true;
    if check {
        let copy = std::fs::read_to_string(root.join(SWIFT_SPEC)).unwrap_or_default();
        if copy != spec {
            eprintln!(
                "sdk: {SWIFT_SPEC} is out of date with docs/openapi.json; regenerate it with `cargo xtask sdk`"
            );
            ok = false;
        }
    } else {
        std::fs::write(root.join(SWIFT_SPEC), &spec)
            .map_err(|error| format!("cannot write {SWIFT_SPEC}: {error}"))?;
        println!("sdk: wrote {SWIFT_SPEC}");
    }
    let text = std::fs::read_to_string(root.join(GENERATORS))
        .map_err(|error| format!("cannot read {GENERATORS}: {error}"))?;
    for problem in generator_problems(&text, workspace_version) {
        eprintln!("sdk: {GENERATORS}: {problem}");
        ok = false;
    }
    if ok && check {
        println!("sdk: Swift, Kotlin and Dart SDK settings match docs/openapi.json");
    }
    Ok(ok)
}

/// Vấn đề của `sdk/openapitools.json`: ghim phiên bản công cụ, đủ hai generator, phiên bản gói bằng
/// phiên bản workspace.
fn generator_problems(text: &str, workspace_version: &str) -> Vec<String> {
    let Ok(config) = serde_json::from_str::<Value>(text) else {
        return vec!["is not JSON".to_owned()];
    };
    let cli = &config["generator-cli"];
    let mut problems = Vec::new();
    let pinned = cli["version"].as_str().unwrap_or_default();
    if pinned.split('.').count() != 3 || !pinned.split('.').all(|part| part.parse::<u32>().is_ok())
    {
        problems
            .push("`generator-cli.version` must pin an exact version, e.g. `7.27.0`".to_owned());
    }
    for (key, generator, field) in
        [("kotlin", "kotlin", "artifactVersion"), ("dart", "dart-dio", "pubVersion")]
    {
        let entry = &cli["generators"][key];
        if entry["generatorName"] != generator {
            problems.push(format!("`generators.{key}.generatorName` must be `{generator}`"));
        }
        if entry["inputSpec"].as_str().is_none_or(|spec| !spec.ends_with("docs/openapi.json")) {
            problems.push(format!("`generators.{key}.inputSpec` must be docs/openapi.json"));
        }
        let version = entry["additionalProperties"][field].as_str().unwrap_or("none");
        if version != workspace_version {
            problems.push(format!(
                "`generators.{key}.additionalProperties.{field}` is {version} but the workspace is \
                 {workspace_version}; set them equal (docs/RELEASING.md, step 2)"
            ));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r##"{"generator-cli": {"version": "7.27.0", "generators": {
        "kotlin": {"generatorName": "kotlin", "inputSpec": "#{cwd}/../docs/openapi.json",
                   "additionalProperties": {"artifactVersion": "1.2.3"}},
        "dart": {"generatorName": "dart-dio", "inputSpec": "#{cwd}/../docs/openapi.json",
                 "additionalProperties": {"pubVersion": "1.2.3"}}}}}"##;

    #[test]
    fn cau_hinh_generator_dung_va_sai() {
        assert!(generator_problems(GOOD, "1.2.3").is_empty());
        assert_eq!(generator_problems(GOOD, "1.2.4").len(), 2);
        let loose = GOOD.replace("7.27.0", "latest");
        assert_eq!(generator_problems(&loose, "1.2.3").len(), 1);
        assert_eq!(generator_problems("nope", "1.2.3"), ["is not JSON"]);
    }

    #[test]
    fn ban_chep_swift_phai_khop() {
        let root = std::env::temp_dir().join(format!("autoxide-sdk-mobile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::create_dir_all(root.join("sdk/swift/Sources/Autoxide")).unwrap();
        std::fs::write(root.join("docs/openapi.json"), "{}").unwrap();
        std::fs::write(root.join(GENERATORS), GOOD).unwrap();
        assert!(!run(&root, true, "1.2.3").unwrap(), "missing copy");
        assert!(run(&root, false, "1.2.3").unwrap());
        assert!(run(&root, true, "1.2.3").unwrap());
        std::fs::write(root.join("docs/openapi.json"), "{\"x\": 1}").unwrap();
        assert!(!run(&root, true, "1.2.3").unwrap(), "contract changed");
        let _ = std::fs::remove_dir_all(&root);
    }
}
