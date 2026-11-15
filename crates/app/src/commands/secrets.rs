//! Lệnh `autoxide secrets reseal` (bước 33, D-257): mã hoá lại mọi secret đã lưu bằng khoá hiện tại
//! — sau khi xoay `AUTOXIDE_SECRET_KEY` (khoá cũ ở `AUTOXIDE_SECRET_KEY_PREVIOUS`), hay để mã hoá
//! secret chữ thường còn sót của bản cũ. Chạy mọi task `<module>.reseal_secrets` module khai
//! (`Module::tasks`), theo thứ tự đăng ký; chạy lại được.

use std::process::ExitCode;

use autoxide_kernel::module::Modules;

use super::{EXIT_FAILURE, report_failure, report_invalid_configuration, run_with_observability};
use crate::runtime::{self, Migrations};
use crate::settings;

/// Tên task một module khai để mã hoá lại secret của nó.
const RESEAL_TASK: &str = "reseal_secrets";

#[derive(Debug, clap::Args)]
pub struct Arguments {
    #[command(subcommand)]
    action: Action,
}

#[derive(Debug, clap::Subcommand)]
enum Action {
    /// Encrypt every stored secret again with the current AUTOXIDE_SECRET_KEY.
    ///
    /// Run it after setting a new AUTOXIDE_SECRET_KEY (with the old one in
    /// AUTOXIDE_SECRET_KEY_PREVIOUS); then remove the old key. Safe to run again.
    Reseal,
}

pub fn run(arguments: &Arguments) -> ExitCode {
    let Action::Reseal = arguments.action;
    let settings = match settings::load() {
        Ok(settings) => settings,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    let config = settings.config.clone();
    let outcome = run_with_observability(&config, async move {
        let runtime = runtime::start(settings, Migrations::RequireApplied).await?;
        let outcome = reseal(&runtime.modules).await;
        runtime.database.close().await.map_err(|error| format!("database: {error}"))?;
        Ok(outcome)
    });
    match outcome {
        Ok(Ok(report)) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Ok(Err(message)) => {
            eprintln!("autoxide secrets reseal: {message}");
            ExitCode::from(EXIT_FAILURE)
        }
        Err(error) => report_failure(&error),
    }
}

/// Chạy mọi task `<module>.reseal_secrets`; trả câu báo (mỗi module một dòng).
async fn reseal(modules: &Modules) -> Result<String, String> {
    let tasks = modules.tasks().map_err(|error| error.to_string())?;
    let mut lines = Vec::new();
    for (name, task) in &tasks {
        if name.rsplit_once('.').map(|(_, task)| task) != Some(RESEAL_TASK) {
            continue;
        }
        let args = task.parse(&[]).map_err(|error| error.to_string())?;
        tracing::info!(task = name.as_str(), "re-encrypting stored secrets");
        let done = task.run(&args).await.map_err(|error| format!("{name}: {error}"))?;
        lines.push(format!("{name}: {done}"));
    }
    if lines.is_empty() {
        return Ok("No module keeps encrypted secrets yet; nothing to do.".to_owned());
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use autoxide_kernel::error::Error;
    use autoxide_kernel::module::Module;
    use autoxide_kernel::task::{Task, TaskArgs, TaskRun};

    use super::*;

    struct Reseal;

    #[async_trait::async_trait]
    impl TaskRun for Reseal {
        async fn run(&self, _: &TaskArgs) -> Result<String, Error> {
            Ok("2 secret(s) encrypted again".to_owned())
        }
    }

    struct Vault;

    impl Module for Vault {
        fn name(&self) -> &'static str {
            "vault"
        }

        fn tasks(&self) -> Vec<Task> {
            vec![
                Task::new(RESEAL_TASK, "Encrypt the secrets again.", Arc::new(Reseal)),
                Task::new("other", "Something else.", Arc::new(Reseal)),
            ]
        }
    }

    #[tokio::test]
    async fn chay_moi_task_reseal_secrets() {
        let modules = Modules::new(vec![Box::new(Vault)]).unwrap();
        assert_eq!(
            reseal(&modules).await.unwrap(),
            "vault.reseal_secrets: 2 secret(s) encrypted again"
        );
        let none = Modules::new(Vec::new()).unwrap();
        assert!(reseal(&none).await.unwrap().contains("nothing to do"));
    }
}
