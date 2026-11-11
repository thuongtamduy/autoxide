//! Lệnh `autoxide config`: kiểm cấu hình và in mọi biến kèm giá trị đang có hiệu lực.

use std::process::ExitCode;

use autoxide_kernel::config::{Origin, SettingValue, render_env_example};

use super::report_invalid_configuration;
use crate::settings;

#[derive(Debug, clap::Args)]
pub struct Arguments {
    /// Print the contents of `.env.example` (every setting with its default)
    /// instead of the current values.
    #[arg(long)]
    env_example: bool,
}

pub fn run(arguments: &Arguments) -> ExitCode {
    if arguments.env_example {
        // Không phụ thuộc cấu hình hiện tại: in được cả khi cấu hình đang sai.
        print!("{}", render_env_example(&settings::describe()));
        return ExitCode::SUCCESS;
    }
    match settings::load() {
        Ok(settings) => {
            print!("{}", render_values(&settings.values));
            ExitCode::SUCCESS
        }
        Err(errors) => report_invalid_configuration(&errors),
    }
}

/// Bảng các biến: tên, giá trị đang có hiệu lực, nguồn, mô tả.
fn render_values(values: &[SettingValue]) -> String {
    let mut output = String::from("Configuration is valid.\n");
    for value in values {
        let origin = match value.origin {
            Origin::Configured => "set",
            Origin::Default => "default",
            Origin::Unset => "not set",
        };
        output.push_str(&format!(
            "\n{} = {}  [{origin}]\n    {}\n",
            value.setting.name,
            value.display_value(),
            value.setting.description
        ));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::config::ConfigSource;

    fn values(pairs: &[(&str, &str)]) -> Vec<SettingValue> {
        settings::read(&ConfigSource::from_pairs(pairs.iter().copied())).unwrap().values
    }

    #[test]
    fn in_gia_tri_mac_dinh_va_nguon() {
        let rendered = render_values(&values(&[]));
        assert!(rendered.starts_with("Configuration is valid.\n"));
        assert!(
            rendered
                .contains("\nDATABASE_URL = sqlite://data/autoxide.sqlite?mode=rwc  [default]\n"),
            "{rendered}"
        );
    }

    #[test]
    fn secret_da_dat_thi_bi_che() {
        let rendered = render_values(&values(&[("DATABASE_URL", "postgres://app:s3cr3t@db/app")]));
        assert!(rendered.contains("\nDATABASE_URL = (set, hidden)  [set]\n"), "{rendered}");
        assert!(!rendered.contains("s3cr3t"), "{rendered}");
    }

    #[test]
    fn file_env_example_trong_repo_khop_voi_code() {
        // `.env.example` là tài liệu cấu hình cho người cài đặt; nó phải luôn khớp
        // các biến khai trong code. Lệch thì sinh lại:
        //   cargo run -p autoxide -- config --env-example > .env.example
        let committed = include_str!("../../../../.env.example");
        assert_eq!(
            committed,
            render_env_example(&settings::describe()),
            "`.env.example` is out of date; regenerate it with \
             `cargo run -p autoxide -- config --env-example > .env.example`"
        );
    }
}
