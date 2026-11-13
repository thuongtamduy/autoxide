//! Lệnh `autoxide task`: liệt kê và chạy lệnh quản trị do module khai (`Module::tasks`, D-169).

use std::process::ExitCode;

use autoxide_kernel::module::Modules;
use autoxide_kernel::task::Task;

use super::{EXIT_FAILURE, report_failure, report_invalid_configuration, run_with_observability};
use crate::runtime::{self, Migrations};
use crate::settings;

/// Mã thoát khi gọi sai (tên lệnh, tham số): như `EX_USAGE` của BSD.
const EXIT_USAGE: u8 = 64;

#[derive(Debug, clap::Args)]
pub struct Arguments {
    /// Full name of the task, `<module>.<task>`. Without it, every task is listed.
    name: Option<String>,
    /// Arguments of the task, each written `name=value`.
    arguments: Vec<String>,
}

pub fn run(arguments: &Arguments) -> ExitCode {
    let settings = match settings::load() {
        Ok(settings) => settings,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    let config = settings.config.clone();
    let name = arguments.name.clone();
    let raw = arguments.arguments.clone();
    let outcome = run_with_observability(&config, async move {
        let runtime = runtime::start(settings, Migrations::RequireApplied).await?;
        let outcome = execute(&runtime.modules, name.as_deref(), &raw).await;
        runtime.database.close().await.map_err(|error| format!("database: {error}"))?;
        Ok(outcome)
    });
    match outcome {
        Ok(Ok(report)) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Ok(Err(Failure::Usage(message))) => {
            eprintln!("autoxide task: {message}");
            ExitCode::from(EXIT_USAGE)
        }
        Ok(Err(Failure::Run(message))) => {
            eprintln!("autoxide task: {message}");
            ExitCode::from(EXIT_FAILURE)
        }
        Err(error) => report_failure(&error),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Failure {
    /// Gọi sai: không có lệnh đó, tham số sai.
    Usage(String),
    /// Lệnh chạy hỏng.
    Run(String),
}

/// Liệt kê (khi không có `name`) hoặc chạy một lệnh; trả câu để in.
async fn execute(modules: &Modules, name: Option<&str>, raw: &[String]) -> Result<String, Failure> {
    let tasks = modules.tasks().map_err(|error| Failure::Run(error.to_string()))?;
    let Some(name) = name else {
        return Ok(list(&tasks));
    };
    let Some((_, task)) = tasks.iter().find(|(full, _)| full == name) else {
        return Err(Failure::Usage(format!(
            "there is no task `{name}`; run `autoxide task` to list them"
        )));
    };
    let args = task.parse(raw).map_err(|error| Failure::Usage(error.to_string()))?;
    tracing::info!(task = name, "running task");
    task.run(&args).await.map_err(|error| Failure::Run(error.to_string()))
}

/// Danh sách lệnh: tên đầy đủ, cách gọi, mô tả, từng tham số.
fn list(tasks: &[(String, Task)]) -> String {
    if tasks.is_empty() {
        return "No module declares a task.".to_owned();
    }
    let mut out = String::from("Tasks (run one with `autoxide task <name> [argument=value …]`):\n");
    for (name, task) in tasks {
        out.push_str(&format!("\n{name} {}\n    {}\n", task.usage(), task.description));
        for argument in &task.arguments {
            let required = if argument.required { "required" } else { "optional" };
            out.push_str(&format!(
                "    {} ({required}): {}\n",
                argument.name, argument.description
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use autoxide_kernel::error::Error;
    use autoxide_kernel::module::Module;
    use autoxide_kernel::task::{TaskArgs, TaskArgument, TaskRun};

    use super::*;

    struct Count;

    #[async_trait::async_trait]
    impl TaskRun for Count {
        async fn run(&self, args: &TaskArgs) -> Result<String, Error> {
            if args.flag("fail", false)? {
                return Err(Error::internal("boom"));
            }
            Ok(format!("counted {}", args.get("what").unwrap_or("nothing")))
        }
    }

    struct Counter;

    impl Module for Counter {
        fn name(&self) -> &'static str {
            "counter"
        }

        fn tasks(&self) -> Vec<Task> {
            vec![
                Task::new("count", "Counts things.", Arc::new(Count))
                    .argument(TaskArgument::required("what", "What to count."))
                    .argument(TaskArgument::optional("fail", "`true` to fail.")),
            ]
        }
    }

    fn modules() -> Modules {
        Modules::new(vec![Box::new(Counter)]).unwrap()
    }

    fn raw(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[tokio::test]
    async fn liet_ke_va_chay() {
        let modules = modules();
        let listed = execute(&modules, None, &[]).await.unwrap();
        assert!(listed.contains("counter.count what=<…> [fail=<…>]"), "{listed}");
        assert!(listed.contains("what (required): What to count."), "{listed}");

        let done = execute(&modules, Some("counter.count"), &raw(&["what=apples"])).await;
        assert_eq!(done, Ok("counted apples".to_owned()));
    }

    #[tokio::test]
    async fn goi_sai_va_chay_hong_tach_rieng() {
        let modules = modules();
        let unknown = execute(&modules, Some("counter.nope"), &[]).await.unwrap_err();
        assert!(
            matches!(&unknown, Failure::Usage(message) if message.contains("no task `counter.nope`"))
        );
        let missing = execute(&modules, Some("counter.count"), &[]).await.unwrap_err();
        assert_eq!(missing, Failure::Usage("missing argument(s): what".to_owned()));
        let failed = execute(&modules, Some("counter.count"), &raw(&["what=x", "fail=true"])).await;
        assert!(matches!(failed, Err(Failure::Run(_))), "{failed:?}");
    }

    /// Lệnh của mọi module đăng ký trong binary khai hợp lệ, và `autoxide task` liệt kê được
    /// chúng trên database thật (cả hai database).
    #[tokio::test]
    async fn lenh_cua_binary_hop_le() {
        autoxide_testkit::TestDatabase::run(async |test| {
            let source =
                autoxide_kernel::config::ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
            let settings = crate::settings::read(&source).unwrap();
            let runtime = runtime::start(settings, Migrations::Ignore).await.unwrap();
            let listed = execute(&runtime.modules, None, &[]).await.unwrap();
            assert!(listed.contains("media.refresh_variants [dry_run=<…>]"), "{listed}");
            runtime.database.close().await.unwrap();
        })
        .await
        .unwrap();
    }

    #[test]
    fn khong_co_lenh_nao() {
        assert_eq!(list(&[]), "No module declares a task.");
    }
}
