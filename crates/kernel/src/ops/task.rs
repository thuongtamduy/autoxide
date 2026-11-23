//! Admin commands declared by modules: `autoxide task <module>.<name> [key=value …]` (D-169, borrowing
//! Loco's tasks and Django's management commands — D-164).
//!
//! For things an operator runs by hand, occasionally: rebuilding derived data after a configuration change, a
//! one-off cleanup, checking figures… Adding a command does not require editing the binary: a module declares
//! it through [`Module::tasks`](crate::module::Module::tasks). A command runs with the server operator's
//! authority (like `autoxide user create`), so it may only do what REST can do with admin permission (rule
//! 8).

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use crate::error::Error;

/// An argument a command accepts, in the form `name=value` on the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TaskArgument {
    pub name: &'static str,
    /// English sentence saying what the argument does and which values it accepts.
    pub description: &'static str,
    pub required: bool,
}

impl TaskArgument {
    pub const fn required(name: &'static str, description: &'static str) -> Self {
        TaskArgument { name, description, required: true }
    }

    pub const fn optional(name: &'static str, description: &'static str) -> Self {
        TaskArgument { name, description, required: false }
    }
}

/// Validated arguments: only declared names, with all required arguments present.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskArgs(BTreeMap<String, String>);

impl TaskArgs {
    /// The value of argument `name`, if the caller supplied it.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    /// A `true` / `false` argument; `default` if not supplied.
    pub fn flag(&self, name: &str, default: bool) -> Result<bool, Error> {
        match self.get(name) {
            None => Ok(default),
            Some("true") => Ok(true),
            Some("false") => Ok(false),
            Some(other) => Err(Error::bad_request(
                "task_argument_invalid",
                "The task argument `{name}` must be `true` or `false`, not `{value}`.",
            )
            .with_param("name", name)
            .with_param("value", other)),
        }
    }
}

/// Error in how the command is invoked (wrong name, missing or extra argument). An error while running is [`Error`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskError(String);

impl fmt::Display for TaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TaskError {}

/// The running part of a command. Returns the result message (in English) to print.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`.
#[async_trait::async_trait]
pub trait TaskRun: Send + Sync {
    async fn run(&self, args: &TaskArgs) -> Result<String, Error>;
}

/// A command a module declares. The declared name is short, `snake_case` (`refresh_variants`); the kernel
/// adds the prefix `<module>.`.
#[derive(Clone)]
#[non_exhaustive]
pub struct Task {
    pub name: &'static str,
    /// English sentence saying what the command does and whether it changes data.
    pub description: &'static str,
    pub arguments: Vec<TaskArgument>,
    runner: Arc<dyn TaskRun>,
}

impl Task {
    pub fn new(name: &'static str, description: &'static str, runner: Arc<dyn TaskRun>) -> Self {
        Task { name, description, arguments: Vec::new(), runner }
    }

    /// Adds an argument the command accepts.
    pub fn argument(mut self, argument: TaskArgument) -> Self {
        self.arguments.push(argument);
        self
    }

    /// Validates `raw` (each element `name=value`) against the declared arguments.
    pub fn parse(&self, raw: &[String]) -> Result<TaskArgs, TaskError> {
        let mut args = BTreeMap::new();
        for item in raw {
            let Some((name, value)) = item.split_once('=') else {
                return Err(TaskError(format!("`{item}` must be written `name=value`")));
            };
            if !self.arguments.iter().any(|argument| argument.name == name) {
                return Err(TaskError(format!(
                    "`{name}` is not an argument of this task; it takes: {}",
                    self.usage()
                )));
            }
            if args.insert(name.to_owned(), value.to_owned()).is_some() {
                return Err(TaskError(format!("`{name}` is given twice")));
            }
        }
        let missing: Vec<&str> = self
            .arguments
            .iter()
            .filter(|argument| argument.required && !args.contains_key(argument.name))
            .map(|argument| argument.name)
            .collect();
        if !missing.is_empty() {
            return Err(TaskError(format!("missing argument(s): {}", missing.join(", "))));
        }
        Ok(TaskArgs(args))
    }

    /// Runs with arguments validated by [`parse`](Self::parse).
    pub async fn run(&self, args: &TaskArgs) -> Result<String, Error> {
        self.runner.run(args).await
    }

    /// `name=<…> [other=<…>]`, or `(no arguments)`.
    pub fn usage(&self) -> String {
        if self.arguments.is_empty() {
            return "(no arguments)".to_owned();
        }
        self.arguments
            .iter()
            .map(|argument| {
                if argument.required {
                    format!("{}=<…>", argument.name)
                } else {
                    format!("[{}=<…>]", argument.name)
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl fmt::Debug for Task {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Task").field("name", &self.name).finish_non_exhaustive()
    }
}

/// A valid command name: lowercase letters, digits, `_`, starting with a letter.
pub(crate) fn valid_name(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;

    #[async_trait::async_trait]
    impl TaskRun for Echo {
        async fn run(&self, args: &TaskArgs) -> Result<String, Error> {
            let dry = args.flag("dry_run", false)?;
            Ok(format!("{} dry={dry}", args.get("email").unwrap_or("-")))
        }
    }

    fn task() -> Task {
        Task::new("echo", "Echo.", Arc::new(Echo))
            .argument(TaskArgument::required("email", "An address."))
            .argument(TaskArgument::optional("dry_run", "`true` to change nothing."))
    }

    fn raw(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[tokio::test]
    async fn chay_voi_tham_so_hop_le() {
        let task = task();
        let args = task.parse(&raw(&["email=a@example.com", "dry_run=true"])).unwrap();
        let done = task.run(&args).await.unwrap();
        assert_eq!(done, "a@example.com dry=true");
        assert_eq!(task.usage(), "email=<…> [dry_run=<…>]");
    }

    #[test]
    fn tu_choi_tham_so_sai_thieu_thua_trung() {
        let task = task();
        let error = |items: &[&str]| task.parse(&raw(items)).unwrap_err().to_string();
        assert!(error(&["email"]).contains("name=value"));
        assert!(error(&["email=a", "color=red"]).contains("`color` is not an argument"));
        assert!(error(&["email=a", "email=b"]).contains("given twice"));
        assert_eq!(error(&[]), "missing argument(s): email");
    }

    #[tokio::test]
    async fn co_sai_gia_tri_thi_bao_ro() {
        let task = task();
        let args = task.parse(&raw(&["email=a", "dry_run=yes"])).unwrap();
        let error = task.run(&args).await.unwrap_err();
        assert_eq!(error.code(), "task_argument_invalid");
        assert!(
            error.to_string().contains("`dry_run` must be `true` or `false`, not `yes`"),
            "{error}"
        );
    }

    #[test]
    fn ten_lenh() {
        assert!(valid_name("refresh_variants"));
        assert!(
            !valid_name("Refresh") && !valid_name("1x") && !valid_name("a-b") && !valid_name("")
        );
    }
}
