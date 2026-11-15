//! Lệnh quản trị do module khai: `autoxide task <module>.<tên> [khoá=giá trị …]` (D-169, mượn
//! tasks của Loco và management commands của Django — D-164).
//!
//! Dành cho việc người vận hành chạy tay, thỉnh thoảng: làm lại dữ liệu dẫn xuất sau khi đổi cấu
//! hình, dọn một lần, kiểm số liệu… Thêm lệnh không phải sửa binary: module khai qua
//! [`Module::tasks`](crate::module::Module::tasks). Lệnh chạy với quyền của người vận hành máy chủ
//! (như `autoxide user create`), nên chỉ làm điều REST với quyền quản trị làm được (luật 8).

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use crate::error::Error;

/// Một tham số lệnh nhận, dạng `tên=giá trị` trên dòng lệnh.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TaskArgument {
    pub name: &'static str,
    /// Câu tiếng Anh nói tham số làm gì và nhận giá trị nào.
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

/// Tham số đã kiểm: chỉ có tên đã khai, đủ tham số bắt buộc.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskArgs(BTreeMap<String, String>);

impl TaskArgs {
    /// Giá trị của tham số `name`, nếu người gọi đưa.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    /// Tham số dạng `true` / `false`; không đưa thì `default`.
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

/// Lỗi về cách gọi lệnh (tên sai, thiếu hay thừa tham số). Lỗi khi chạy là [`Error`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskError(String);

impl fmt::Display for TaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TaskError {}

/// Phần chạy của một lệnh. Trả câu báo kết quả (tiếng Anh) để in ra.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`.
#[async_trait::async_trait]
pub trait TaskRun: Send + Sync {
    async fn run(&self, args: &TaskArgs) -> Result<String, Error>;
}

/// Một lệnh module khai. Tên khai ngắn, `snake_case` (`refresh_variants`); kernel thêm tiền tố
/// `<module>.`.
#[derive(Clone)]
#[non_exhaustive]
pub struct Task {
    pub name: &'static str,
    /// Câu tiếng Anh nói lệnh làm gì, có thay đổi dữ liệu không.
    pub description: &'static str,
    pub arguments: Vec<TaskArgument>,
    runner: Arc<dyn TaskRun>,
}

impl Task {
    pub fn new(name: &'static str, description: &'static str, runner: Arc<dyn TaskRun>) -> Self {
        Task { name, description, arguments: Vec::new(), runner }
    }

    /// Thêm một tham số lệnh nhận.
    pub fn argument(mut self, argument: TaskArgument) -> Self {
        self.arguments.push(argument);
        self
    }

    /// Kiểm `raw` (mỗi phần tử `tên=giá trị`) theo các tham số đã khai.
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

    /// Chạy với tham số đã kiểm bằng [`parse`](Self::parse).
    pub async fn run(&self, args: &TaskArgs) -> Result<String, Error> {
        self.runner.run(args).await
    }

    /// `name=<…> [other=<…>]`, hoặc `(no arguments)`.
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

/// Tên lệnh hợp lệ: chữ thường, số, `_`, bắt đầu bằng chữ.
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
