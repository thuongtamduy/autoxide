//! `cargo xtask doctor`: kiểm máy của người phát triển đã đủ đồ để làm việc với repo này chưa (bước 71, G6).
//!
//! Mỗi công cụ có ba mức: `required` (thiếu thì không build/gate được), `recommended` (chạy được nhưng chậm hay
//! thiếu một phần gate), `optional` (chỉ cần cho việc riêng). Mỗi dòng thiếu kèm MỘT lệnh cài. Lệnh chỉ đọc —
//! không cài gì, không ghi gì. Khác `autoxide doctor` (kiểm một bản cài đặt đang chạy): đây kiểm cái máy.

use std::path::Path;
use std::process::{Command, ExitCode};

/// Mức cần thiết của một công cụ.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Need {
    Required,
    Recommended,
    Optional,
}

struct Tool {
    program: &'static str,
    args: &'static [&'static str],
    need: Need,
    /// Dùng vào việc gì.
    why: &'static str,
    /// Cách cài, một lệnh hay một đường dẫn.
    install: &'static str,
}

const TOOLS: &[Tool] = &[
    Tool {
        program: "rustc",
        args: &["--version"],
        need: Need::Required,
        why: "builds everything",
        install: "install rustup (https://rustup.rs); the pinned toolchain in rust-toolchain.toml is fetched on first use",
    },
    Tool {
        program: "cargo",
        args: &["--version"],
        need: Need::Required,
        why: "builds and runs `cargo xtask`",
        install: "install rustup (https://rustup.rs)",
    },
    Tool {
        program: "git",
        args: &["--version"],
        need: Need::Required,
        why: "the gate compares against origin/main",
        install: "install git (https://git-scm.com)",
    },
    Tool {
        program: "cargo-nextest",
        args: &["nextest", "--version"],
        need: Need::Recommended,
        why: "runs the tests faster (the gate falls back to `cargo xtask` without it)",
        install: "cargo install cargo-nextest --locked",
    },
    Tool {
        program: "cargo-deny",
        args: &["deny", "--version"],
        need: Need::Recommended,
        why: "licence and advisory check of `./gate.sh`",
        install: "cargo install cargo-deny --locked",
    },
    Tool {
        program: "gitleaks",
        args: &["version"],
        need: Need::Recommended,
        why: "secret scan of `./gate.sh` (Docker can stand in for it)",
        install: "install gitleaks (https://github.com/gitleaks/gitleaks) or Docker",
    },
    Tool {
        program: "docker",
        args: &["--version"],
        need: Need::Optional,
        why: "the PostgreSQL half of the full gate, and `docker run` of the image",
        install: "install Docker (https://docs.docker.com/get-docker/) or point GATE_POSTGRES_URL at a PostgreSQL",
    },
    Tool {
        program: "node",
        args: &["--version"],
        need: Need::Optional,
        why: "builds and checks the TypeScript SDK (sdk/typescript)",
        install: "install Node.js LTS (https://nodejs.org)",
    },
];

/// Kết quả kiểm một công cụ.
#[derive(Debug, PartialEq, Eq)]
enum Found {
    Version(String),
    Missing,
}

fn probe(tool: &Tool) -> Found {
    // `cargo-nextest` chạy qua `cargo nextest`: thử chương trình con của cargo, không tìm file trong PATH.
    let (program, args): (&str, &[&str]) = match tool.program.strip_prefix("cargo-") {
        Some(_) => ("cargo", tool.args),
        None => (tool.program, tool.args),
    };
    match Command::new(program).args(args).output() {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            Found::Version(text.lines().next().unwrap_or("").trim().to_owned())
        }
        _ => Found::Missing,
    }
}

/// Phiên bản Rust ghim trong `rust-toolchain.toml` (`channel = "…"`).
fn pinned_channel(toolchain_file: &str) -> Option<String> {
    toolchain_file.lines().find_map(|line| {
        let value = line.trim().strip_prefix("channel")?.trim_start().strip_prefix('=')?;
        Some(value.trim().trim_matches('"').to_owned())
    })
}

/// `rustc 1.100.0 (hash date)` → `1.100.0`.
fn rustc_version(line: &str) -> Option<&str> {
    line.split_whitespace().nth(1)
}

pub fn run() -> ExitCode {
    let mut failed = false;
    println!("xtask doctor: checking this machine for working on Autoxide\n");
    for tool in TOOLS {
        match probe(tool) {
            Found::Version(version) => println!("  ok       {:<14} {version}", tool.program),
            Found::Missing => {
                let label = match tool.need {
                    Need::Required => "MISSING ",
                    Need::Recommended => "missing ",
                    Need::Optional => "optional",
                };
                println!("  {label} {:<14} {}", tool.program, tool.why);
                println!("           fix: {}", tool.install);
                failed |= tool.need == Need::Required;
            }
        }
    }
    if let Some(problem) = toolchain_problem() {
        println!("  MISSING  toolchain      {problem}");
        failed = true;
    }
    println!();
    if failed {
        println!("Install the tools marked MISSING, then run `cargo xtask doctor` again.");
        ExitCode::FAILURE
    } else if let gate_missing @ [_, ..] = gate_missing().as_slice() {
        // `./gate.sh` kiểm các công cụ này ngay đầu và dừng nếu thiếu: không nói "sẵn sàng" khi gate chưa chạy được.
        println!(
            "You can build and run the tests, but `./gate.sh` cannot run yet: it needs {}.",
            gate_missing.join(" and ")
        );
        println!("Try `cargo run -p autoxide -- demo`.");
        ExitCode::SUCCESS
    } else {
        println!(
            "Ready: `./gate.sh --quick` and `cargo test` can run. Try `cargo run -p autoxide -- demo`."
        );
        ExitCode::SUCCESS
    }
}

/// Công cụ mà `gate.sh` bắt buộc (gate.sh, khối `need`): `cargo-deny`, và `gitleaks` hoặc `docker`.
fn gate_missing() -> Vec<&'static str> {
    let found = |program: &str| {
        TOOLS
            .iter()
            .find(|tool| tool.program == program)
            .is_some_and(|tool| probe(tool) != Found::Missing)
    };
    let mut missing = Vec::new();
    if !found("cargo-deny") {
        missing.push("cargo-deny");
    }
    if !found("gitleaks") && !found("docker") {
        missing.push("gitleaks (or Docker)");
    }
    missing
}

/// `None` khi `rustc` đang dùng đúng bản ghim (hay không đọc được để so).
fn toolchain_problem() -> Option<String> {
    let pinned = pinned_channel(&std::fs::read_to_string(Path::new("rust-toolchain.toml")).ok()?)?;
    let Found::Version(line) = probe(&TOOLS[0]) else { return None };
    let current = rustc_version(&line)?;
    (current != pinned).then(|| {
        format!("rustc is {current} but rust-toolchain.toml pins {pinned}; run `rustup toolchain install {pinned}`")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_phien_ban_ghim() {
        let file = "[toolchain]\nchannel = \"1.100.0\"\ncomponents = [\"rustfmt\"]\n";
        assert_eq!(pinned_channel(file).as_deref(), Some("1.100.0"));
        assert_eq!(pinned_channel("[toolchain]\n"), None);
    }

    #[test]
    fn doc_phien_ban_rustc() {
        assert_eq!(rustc_version("rustc 1.100.0 (abc 2026-11-01)"), Some("1.100.0"));
    }

    #[test]
    fn moi_cong_cu_co_cach_cai() {
        for tool in TOOLS {
            assert!(!tool.install.is_empty() && !tool.why.is_empty(), "{}", tool.program);
        }
    }
}
