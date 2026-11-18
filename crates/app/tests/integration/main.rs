//! Mọi test tích hợp của crate trong một chương trình: mỗi file `tests/*.rs` riêng là một chương trình
//! test riêng, link lại toàn bộ thư viện, nên tốn đĩa và thời gian link gấp nhiều lần (D-20261118-build-size).
//! Thêm test: tạo file trong thư mục này rồi khai `mod` ở đây.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod cli;
mod upgrade;
