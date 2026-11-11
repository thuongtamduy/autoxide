//! Lệnh `autoxide openapi`: in tài liệu OpenAPI của cả chương trình.

use std::process::ExitCode;

use autoxide_kernel::http;

use super::{report_failure, report_invalid_configuration, run_with_observability};
use crate::runtime;
use crate::settings::{self, Settings};

pub fn run() -> ExitCode {
    let settings = match settings::load() {
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
    let problems = http::lint(&runtime.api.openapi);
    let document = runtime.api.openapi.to_pretty_json();
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
    ///   cargo run -p autoxide -- openapi > docs/openapi.json
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
                 `cargo run -p autoxide -- openapi > docs/openapi.json`"
            );
        })
        .await
        .unwrap();
    }
}
