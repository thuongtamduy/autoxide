//! Chặn mật khẩu phổ biến hay đã lộ (NIST SP 800-63B-4 §3.1.1.2, bước 71 G7, D-20261127-breached-passwords).
//!
//! NIST yêu cầu so mật khẩu mới với danh sách giá trị "hay dùng, dễ đoán hay đã lộ" và từ chối nếu trùng.
//! Danh sách đi kèm binary (`data/common-passwords.txt`), kiểm hoàn toàn offline: không gửi mật khẩu hay
//! một phần hash của nó ra ngoài. Nguồn: `xato-net-10-million-passwords.txt` của SecLists (giấy phép MIT), chỉ
//! giữ mục dài 15 đến 128 ký tự (ngắn hơn đã bị luật độ dài chặn, giữ thì chỉ tốn chỗ), lấy theo thứ tự hay gặp
//! nhất cho tới khi file đủ 1 MB, đổi sang chữ thường, bỏ trùng, sắp theo byte — cách dựng lại ghi ở quyết định.
//!
//! So không phân biệt hoa thường: `Password123456` cũng bị chặn như `password123456` (NIST: đổi hoa thường
//! không làm mật khẩu khó đoán hơn với kẻ thử theo danh sách).

use std::sync::LazyLock;

/// Danh sách, mỗi dòng một mật khẩu chữ thường, đã sắp theo byte.
const LIST: &str = include_str!("../data/common-passwords.txt");

// `LazyLock`: tách danh sách thành mảng ở lần dùng đầu tiên (một lần cho cả tiến trình), không phải ở
// mỗi request. Mảng chỉ mượn chuỗi nhúng sẵn trong binary (`&'static str`), không chép.
static ENTRIES: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| LIST.lines().filter(|line| !line.is_empty()).collect());

/// `password` nằm trong danh sách mật khẩu phổ biến / đã lộ, hoặc chỉ là một ký tự lặp lại
/// (`aaaaaaaaaaaa` — NIST xếp ký tự lặp vào nhóm giá trị dễ đoán).
pub(crate) fn is_common(password: &str) -> bool {
    let mut characters = password.chars();
    if let Some(first) = characters.next()
        && characters.all(|character| character == first)
    {
        return true;
    }
    // Tra cả bản bỏ khoảng trắng hai đầu: `"password123456789 "` cũng là mật khẩu trong danh sách. Chỉ để tra;
    // chuỗi được hash vẫn là chuỗi người dùng gửi.
    let lowered = password.to_lowercase();
    let listed = |candidate: &str| {
        ENTRIES.binary_search_by(|entry| entry.as_bytes().cmp(candidate.as_bytes())).is_ok()
    };
    listed(&lowered) || listed(lowered.trim())
}

#[cfg(test)]
mod tests;
