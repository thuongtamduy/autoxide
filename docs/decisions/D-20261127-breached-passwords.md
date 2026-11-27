## D-20261127-breached-passwords · Mật khẩu theo NIST SP 800-63B-4: chặn mật khẩu đã lộ offline, tối thiểu 15 ký tự — 2026-11-27 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: bước 71 G7. Mọi chỗ đặt mật khẩu (tự đăng ký, quản trị / `autoxide user create` / bootstrap tạo tài khoản,
đặt mật khẩu đầu tiên, đổi, đặt lại, nhận lời mời; SCIM không đặt mật khẩu) đi qua hai hàm của `modules/auth/src/password.rs`
(`Passwords::problem`, `check_rules`) và nay kiểm theo thứ tự:

1. **Độ dài 15 đến 128 ký tự** (code point). `MIN_LENGTH` từ 12 lên 15; `auth.password_min_length` của khách nhận
   15–128, giá trị 12–14 đã lưu được nâng thành 15 (clamp sẵn có). Mật khẩu đang dùng vẫn đăng nhập được.
2. **Không nằm trong danh sách mật khẩu phổ biến / đã lộ** nhúng trong binary: `modules/auth/data/common-passwords.txt`
   (46.146 mục, ~675 KB), đọc bằng `include_str!`, tách một lần (`LazyLock`), tìm nhị phân. So **không phân biệt hoa
   thường** (`to_lowercase` cả hai phía), cả chuỗi chứ không tìm chuỗi con. Thêm luật "chỉ một ký tự lặp lại"
   (`aaaaaaaaaaaaaaa`). Sai thì `422 validation_failed`, lỗi trường mã `common_password`, câu nói lý do (NIST: "SHALL
   provide the reason"), có bản dịch `field:` trong `locales/vi.json`; mô tả 422 của sáu endpoint đặt mật khẩu nêu mã
   này. Không ghi mật khẩu vào log hay vào lỗi.

**Nguồn danh sách:** `Passwords/Common-Credentials/xato-net-10-million-passwords-1000000.txt` của SecLists
(<https://github.com/danielmiessler/SecLists>, giấy phép **MIT**, © Daniel Miessler; danh sách gốc là bộ 10 triệu mật
khẩu Mark Burnett công bố năm 2015 từ các vụ lộ dữ liệu). Ghi ở `REUSE.toml` (bảng riêng cho file, `MIT`). Dựng lại:
lấy file đó, bỏ dòng không phải UTF-8 hợp lệ / có ký tự điều khiển / có khoảng trắng hai đầu, giữ mục dài 12–128 ký tự
(ngắn hơn luôn bị luật độ dài chặn; giữ từ 12 để danh sách không phải dựng lại nếu mức tối thiểu đổi), đổi chữ thường,
bỏ trùng, sắp theo byte, mỗi dòng một mục, kết thúc bằng `\n`. Bài `breached::tests::danh_sach_da_sap_va_khong_trung`
chặn file sắp sai.

Lý do: NIST SP 800-63B-4 (bản chính thức 2025-08) §3.1.1.2: verifier SHALL so mật khẩu mới với blocklist "commonly used,
expected, or compromised" (ví dụ: mật khẩu từ các vụ lộ, từ điển, từ theo ngữ cảnh); SHALL yêu cầu tối thiểu 15 ký tự
khi mật khẩu là yếu tố duy nhất (ở đây mật khẩu một mình đăng nhập được, trừ khi role buộc MFA); SHOULD cho tối đa ít
nhất 64 (đang 128); SHALL NOT ép luật ghép ký tự hay đổi định kỳ (đã đúng từ D-044). Kiểm offline: không gửi gì ra
ngoài, không phụ thuộc mạng lúc đăng ký, chạy giống nhau trên cả SQLite và Postgres (không chạm database). Danh sách
46 nghìn mục là trên ngưỡng đoán trực tuyến khi đã có login throttling (NIST: blocklist "SHOULD be of sufficient size"
so với giới hạn thử).

Đã cân nhắc:
- Kiểm qua dịch vụ Have I Been Pwned (k-anonymity, gửi 5 ký tự đầu SHA-1): phủ hàng trăm triệu mật khẩu nhưng gọi mạng
  ở đường đăng ký, cần xử lý khi dịch vụ chậm / hỏng, và gửi dữ liệu ra bên thứ ba (bảo vệ dữ liệu cá nhân). Để **tuỳ
  chọn sau 1.0** (một `Setting` bật thêm, không thay danh sách offline).
- Danh sách NCSC 100k (từ dữ liệu HIBP, CC BY 4.0): chỉ ~1.300 mục dài từ 12 ký tự, giấy phép khác MIT/Apache.
- Danh sách `Pwdb_top-1000000` của SecLists: nguồn gốc dữ liệu không rõ bằng bộ của Mark Burnett.
- Nén danh sách (bloom filter, fst): thêm thư viện hay code tự viết để tiết kiệm ~600 KB binary; chưa đáng.
- Giữ tối thiểu 12: lệch một yêu cầu SHALL của chính tiêu chuẩn mà mục lộ trình nêu tên.
- Chặn từ theo ngữ cảnh (email, tên người dùng, tên dịch vụ — NIST nêu là ví dụ, không bắt buộc): cần truyền email
  vào mọi đường kiểm; để sau (tên `autoxide` ngắn hơn 15 ký tự nên đã bị luật độ dài chặn).
- Chuẩn hoá Unicode NFC trước khi hash (NIST SHOULD): đổi hash của mật khẩu đã lưu có ký tự tổ hợp; cần quyết định
  riêng kèm đường chuyển tiếp.

Thay: phần "Luật mật khẩu: chỉ độ dài, 12–128 ký tự" của D-044
