//! Lệnh `autoxide openapi`: in tài liệu OpenAPI của cả chương trình.

use std::process::ExitCode;

use autoxide_kernel::http;

use super::{report_failure, report_invalid_configuration, run_with_observability};
use crate::runtime;
use crate::settings::{self, Settings};

pub fn run() -> ExitCode {
    // Hợp đồng đầy đủ: mọi module, kể cả module bản cài này đang tắt (bước 70 P1).
    let settings = match settings::load_everything() {
        Ok(settings) => settings,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    let config = settings.config.clone();
    match run_with_observability(&config, render(settings)) {
        Ok(document) => {
            print!("{document}");
            ExitCode::SUCCESS
        }
        Err(error) => report_failure(&error),
    }
}

/// Tài liệu OpenAPI dạng JSON dễ đọc, kết thúc bằng một dòng trống. Tài liệu chưa
/// đủ để gọi API mà không đọc mã nguồn (xem `http::lint`) thì là lỗi.
pub async fn render(settings: Settings) -> Result<String, String> {
    let runtime = runtime::start(settings, runtime::Migrations::Ignore).await?;
    let mut problems = http::lint(&runtime.api.openapi);
    problems.extend(runtime.api.schema_conflicts.iter().cloned());
    // Qua `Value` để mọi khoá được sắp theo tên: extension của operation (`x-permissions`,
    // `x-sunset`) nằm trong một `HashMap` của utoipa, thứ tự đổi giữa các lần sinh.
    let document = serde_json::to_value(&runtime.api.openapi)
        .and_then(|document| serde_json::to_string_pretty(&document));
    runtime.database.close().await.map_err(|error| format!("database: {error}"))?;

    if !problems.is_empty() {
        return Err(format!(
            "the OpenAPI document is incomplete:\n  - {}",
            problems.join("\n  - ")
        ));
    }
    let mut document =
        document.map_err(|error| format!("cannot serialize the OpenAPI document: {error}"))?;
    document.push('\n');
    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::config::ConfigSource;
    use autoxide_testkit::TestDatabase;

    /// `docs/openapi.json` là hợp đồng của API, nằm trong repo để mọi thay đổi API
    /// hiện ra khi xem lại thay đổi. Nó phải luôn khớp code; lệch thì sinh lại:
    ///   cargo xtask regen
    #[tokio::test]
    async fn file_openapi_trong_repo_khop_voi_code() {
        TestDatabase::run(async |database| {
            let source = ConfigSource::from_pairs([("DATABASE_URL", database.url())]);
            let settings = settings::read(&source).unwrap();

            let generated = render(settings).await.unwrap();

            let committed = include_str!("../../../../docs/openapi.json");
            assert_eq!(
                committed, generated,
                "`docs/openapi.json` is out of date; regenerate it with \
                 `cargo xtask regen`"
            );
        })
        .await
        .unwrap();
    }
}
