//! Lệnh `autoxide mcp-manifest`: in hợp đồng tool MCP của cả chương trình (D-233, đặc tả 31 A6).
//!
//! `docs/mcp-tools.json` (sinh bằng lệnh này) với tool như `docs/openapi.json` với endpoint: agent
//! viết cho bản cũ gọi tool theo tên và tham số đã biết, nên `cargo xtask api-diff` so file này với
//! bản ở tag phát hành trước và chặn thay đổi phá vỡ.

use std::process::ExitCode;

use autoxide_kernel::mcp::Tool;
use serde_json::{Value, json};

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

/// Manifest dạng JSON dễ đọc, kết thúc bằng một dòng trống. Tất định: tool sắp theo tên, khoá của
/// object tự sắp (`serde_json` của workspace không bật `preserve_order`).
pub async fn render(settings: Settings) -> Result<String, String> {
    let runtime = runtime::start(settings, runtime::Migrations::Ignore).await?;
    let declared = runtime.modules.tools().and_then(|tools| {
        Ok((tools, runtime.modules.mcp_resources()?, runtime.modules.prompts()?))
    });
    runtime.database.close().await.map_err(|error| format!("database: {error}"))?;
    let (mut tools, mut resources, mut prompts) = declared.map_err(|error| error.to_string())?;
    tools.sort_by(|a, b| a.name().cmp(b.name()));
    resources.sort_by(|a, b| a.uri().cmp(b.uri()));
    prompts.sort_by(|a, b| a.name().cmp(b.name()));
    // Resource và prompt (bước 40 P2): hai resource của kernel (`llms.txt`, `openapi.json`) do lớp
    // HTTP thêm, không nằm ở đây.
    let manifest = json!({
        "manifest_version": 1,
        "tools": tools.iter().map(describe).collect::<Vec<Value>>(),
        "resources": resources.iter().map(|resource| json!({
            "uri": resource.uri(),
            "name": resource.name(),
            "description": resource.description(),
            "mime_type": resource.mime_type(),
            "permissions": resource.permission().map(|permission| permission.name()).into_iter().collect::<Vec<_>>(),
        })).collect::<Vec<Value>>(),
        "prompts": prompts.iter().map(|prompt| json!({
            "name": prompt.name(),
            "description": prompt.description(),
            "arguments": prompt.arguments().iter().map(|argument| json!({
                "name": argument.name,
                "description": argument.description,
                "required": argument.required,
            })).collect::<Vec<Value>>(),
            "tools": prompt.tools(),
        })).collect::<Vec<Value>>(),
    });
    let mut document = serde_json::to_string_pretty(&manifest)
        .map_err(|error| format!("cannot serialize the MCP tool manifest: {error}"))?;
    document.push('\n');
    Ok(document)
}

fn describe(tool: &Tool) -> Value {
    let permissions: Vec<&str> = tool
        .permission()
        .into_iter()
        .chain(tool.alternative())
        .map(|permission| permission.name())
        .collect();
    json!({
        "name": tool.name(),
        "description": tool.description(),
        "permissions": permissions,
        "annotations": {
            "read_only": tool.is_read_only(),
            "destructive": tool.is_destructive(),
            "returns_secret": tool.is_secret(),
            // Có thật từ bước 51 (P12); tới đó luôn `false`.
            "requires_approval_for_agent": false,
        },
        "input_schema": tool.input_schema(),
        // `Tool` chưa khai schema kết quả (đặc tả 31 A6).
        "output_schema": Value::Null,
        // `experimental`: tool của module thử nghiệm, có thể đổi giữa các bản (bước 70 P3).
        "stability": if tool.is_experimental() { "experimental" } else { "official" },
        "deprecation": tool.deprecation().map(|deprecation| json!({
            "since": deprecation.since,
            "sunset": deprecation.sunset,
            "successor": deprecation.successor,
        })),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::config::ConfigSource;
    use autoxide_testkit::TestDatabase;

    /// `docs/mcp-tools.json` là hợp đồng tool MCP; phải luôn khớp code. Lệch thì sinh lại:
    ///   cargo run -p autoxide -- mcp-manifest > docs/mcp-tools.json
    #[tokio::test]
    async fn file_mcp_tools_trong_repo_khop_voi_code() {
        TestDatabase::run(async |database| {
            let source = ConfigSource::from_pairs([("DATABASE_URL", database.url())]);
            let settings = settings::read(&source).unwrap();

            let generated = render(settings).await.unwrap();

            let committed = include_str!("../../../../docs/mcp-tools.json");
            assert_eq!(
                committed, generated,
                "`docs/mcp-tools.json` is out of date; regenerate it with \
                 `cargo run -p autoxide -- mcp-manifest > docs/mcp-tools.json`"
            );
        })
        .await
        .unwrap();
    }
}
