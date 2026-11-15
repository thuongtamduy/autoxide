//! Lệnh `autoxide ai check`: thử cấu hình AI với nhà cung cấp thật (D-109) — một lời gọi sinh
//! chữ rất nhỏ có schema, và một lời gọi embedding nếu bật. Người cài chạy lệnh này sau khi đặt
//! `AUTOXIDE_AI_*`; gate không chạy (cần key thật, tốn vài token).

use std::process::ExitCode;
use std::sync::Arc;

use autoxide_kernel::ai::{Effort, EmbedKind, Embedder, Part, Prompt, TextModel};
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use serde_json::json;

use super::{report_failure, report_invalid_configuration, run_with_observability};
use crate::runtime::{self, Migrations};
use crate::settings::{self, Settings};

#[derive(Debug, clap::Args)]
pub struct Arguments {
    #[command(subcommand)]
    action: Action,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Action {
    /// Send one tiny request to the configured AI provider (and embeddings service) and say
    /// whether it works.
    ///
    /// Uses a few tokens, counted in the daily limit. Run `autoxide migrate up` first.
    Check,
}

pub fn run(arguments: &Arguments) -> ExitCode {
    let Action::Check = arguments.action;
    let settings = match settings::load() {
        Ok(settings) => settings,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    let config = settings.config.clone();
    match run_with_observability(&config, check(settings)) {
        Ok((report, true)) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Ok((report, false)) => {
            print!("{report}");
            ExitCode::from(super::EXIT_FAILURE)
        }
        Err(error) => report_failure(&error),
    }
}

/// Báo cáo bằng lời thường, và `true` khi mọi thứ đã bật đều chạy.
async fn check(settings: Settings) -> Result<(String, bool), String> {
    let runtime = runtime::start(settings, Migrations::RequireApplied).await?;
    let text = runtime.modules.api::<dyn TextModel>();
    let embedder = runtime.modules.api::<dyn Embedder>();
    let outcome = match (text, embedder) {
        (Some(text), Some(embedder)) => {
            Ok(report(&TenantContext::default_on(runtime.database.clone()), text, embedder).await)
        }
        _ => Err("the `ai` module is not registered".to_owned()),
    };
    let closed = runtime.database.close().await.map_err(|error| format!("database: {error}"));
    let outcome = outcome?;
    closed?;
    Ok(outcome)
}

async fn report(
    tenant: &TenantContext,
    text: Arc<dyn TextModel>,
    embedder: Arc<dyn Embedder>,
) -> (String, bool) {
    let mut lines = String::new();
    let mut healthy = true;
    let capabilities = text.capabilities();
    if capabilities.enabled {
        let schema = json!({
            "type": "object",
            "properties": {"answer": {"type": "string"}},
            "required": ["answer"],
            "additionalProperties": false
        });
        let instructions = "Reply with the word `ok` in the field `answer`.".to_owned();
        let prompt = Prompt::new("ai.check", instructions, schema)
            .input(vec![Part::Text("Health check.".to_owned())])
            .effort(Effort::Low)
            .max_output_tokens(1024);
        match text.generate(tenant, prompt).await {
            Ok(done) => lines.push_str(&format!(
                "Text generation: works ({} {}, answered by {}, {} tokens).\n",
                capabilities.provider,
                capabilities.model,
                done.model,
                done.usage.input_tokens + done.usage.output_tokens
            )),
            Err(error) => {
                healthy = false;
                lines.push_str(&format!("Text generation: FAILED — {}\n", describe(&error)));
            }
        }
    } else {
        lines.push_str("Text generation: off (set AUTOXIDE_AI_PROVIDER to turn it on).\n");
    }
    if embedder.enabled() {
        let texts = vec!["Health check.".to_owned()];
        match embedder.embed(tenant, "ai.check", &texts, EmbedKind::Query).await {
            Ok(_) => lines.push_str(&format!(
                "Embeddings: work ({}, {} dimensions).\n",
                embedder.model_id(),
                embedder.dimensions()
            )),
            Err(error) => {
                healthy = false;
                lines.push_str(&format!("Embeddings: FAILED — {}\n", describe(&error)));
            }
        }
    } else {
        lines.push_str("Embeddings: off (set AUTOXIDE_EMBEDDING_PROVIDER to turn them on).\n");
    }
    (lines, healthy)
}

fn describe(error: &Error) -> String {
    format!("{} ({}). The server log has details.", error.message(), error.code())
}
