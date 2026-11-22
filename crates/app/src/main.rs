//! Binary `autoxide` chính thức: mọi module chính thức, không thêm gì. Toàn bộ chương trình nằm ở
//! thư viện (`lib.rs`), để binary của người viết module ở repo riêng chạy đúng cùng một code.

use std::process::ExitCode;

fn main() -> ExitCode {
    autoxide::Autoxide::new().run()
}
