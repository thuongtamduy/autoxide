## D-20261123-property-fuzzing · Thử phá có chủ đích bằng `proptest` trên stable, chạy dài theo lịch đêm — 2026-11-23 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: fuzz và property test trước 1.0 (P12 của bước 70) dùng **một** công cụ là `proptest` (đã có từ đặc tả
31 B3), chạy trên toolchain stable đang ghim, viết trong `#[cfg(test)]` cạnh hàm được thử như test thường:

- Gate (`./gate.sh --quick` và đầy đủ) chạy chúng với mặc định 256 ca mỗi test; đích là hàm thuần (không database),
  nên luật "cả hai database" không áp. Lỗi tìm ra chạm database thì test tái hiện đi qua `TestDatabase::run` như mọi
  test khác.
- Đích nhận dữ liệu từ ngoài (chữ ký và thân webhook của bộ nối, token, con trỏ, bộ lọc, file nhập) có bất biến tối
  thiểu: **không panic với đầu vào bất kỳ** (kể cả số biên như `i64::MIN`), **đúng thì nhận / sửa một byte thì từ
  chối**, mã hoá rồi đọc lại nguyên vẹn. Đích tính tiền có bất biến: không âm, tổng khớp, không tràn (lỗi thay vì
  panic).
- Workflow `.github/workflows/property.yml` chạy hằng đêm (và chạy tay) mọi unit test của workspace với
  `PROPTEST_CASES=20000`; không chặn PR. Ca hỏng được proptest ghi vào `proptest-regressions/` cạnh file nguồn — commit
  thư mục đó (corpus nhỏ, chạy lại mãi trong gate).

Lý do: `cargo-fuzz` (libFuzzer) cần toolchain nightly và `-Z sanitizer`, tức một toolchain thứ hai, một bộ harness
thứ hai (`fuzz/` với crate riêng) và đích phải mở `pub` ra ngoài crate — trái luật "crate module chỉ mở `api` và
`module`". Đích của dự án là code Rust an toàn (`unsafe` bị cấm), nên lợi thế lớn nhất của libFuzzer (bắt lỗi bộ
nhớ) không áp; thứ cần bắt là panic, tràn số và bất biến nghiệp vụ sai — proptest bắt được, thu gọn ca hỏng và chạy
ngay trong `cargo test` trên stable. Một cách làm cho mỗi việc.

Đã cân nhắc: `cargo-fuzz` ở job nightly riêng (loại vì lý do trên); `bolero` (một harness chạy được cả proptest-like
lẫn libFuzzer — thêm thư viện thứ hai cho cùng việc, chế độ fuzz vẫn cần nightly); `quickcheck` (thu gọn kém hơn,
repo đã dùng proptest).
