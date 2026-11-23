## D-20261123-security-review-2026-11 · Rà soát bảo mật trước 1.0: báo cáo công khai, sửa hết mức cao/trung bình, role đòi MFA chặn ở cửa đăng nhập — 2026-11-23 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định:

1. Kết quả rà soát bảo mật P6 (bước 70) ghi ở `docs/security/review-2026-11.md` (tiếng Anh, công khai), nêu rõ là
   rà soát nội bộ có AI hỗ trợ, không phải kiểm toán bên thứ ba; `SECURITY.md` trỏ tới. Lỗ hổng chưa sửa chỉ
   mô tả ở mức đủ để người vận hành tự bảo vệ, không ghi cách khai thác.
2. Chủ dự án yêu cầu sửa hết mức cao và trung bình trước 1.0, mỗi cụm một PR kèm test tái hiện trên cả hai
   database; mức thấp sửa luôn nếu rẻ và an toàn, còn lại ghi lý do trong báo cáo và vào lộ trình ("Nợ bảo mật
   sau rà soát 2026-11").
3. M-8 — "role đòi MFA" kiểm tài khoản *đã bật* yếu tố thứ hai chứ không kiểm *phiên* đã dùng nó; đăng nhập
   bằng mật khẩu chỉ hỏi mã khi có TOTP, nên tài khoản chỉ có passkey mở được phiên mang quyền của role bằng
   mật khẩu. Sửa ở cửa đăng nhập: tài khoản giữ role đòi MFA mà yếu tố thứ hai duy nhất là passkey thì mọi cách
   đăng nhập không qua yếu tố thứ hai (mật khẩu, link, nhà cung cấp ngoài) trả `403 auth.passkey_required`; chỉ
   passkey (hay mật khẩu + mã khi đã bật TOTP) mở phiên. Uỷ quyền / quyền hỗ trợ áp cùng luật role đòi MFA như
   ở khách nhà (M-4).

Lý do: repo công khai và sắp 1.0; người dùng cần biết đã rà gì, sửa gì, còn gì. M-8 chặn ở cửa đăng nhập thay vì
ghi cách đăng nhập (`amr`) lên phiên rồi dựng quyền theo phiên: cách sau phải sửa kernel (`Principal`), token và
mọi đường cấp phiên, và làm người vừa bật TOTP mất quyền của role tới lần đăng nhập sau; cách chặn ở cửa giữ nguyên
hành vi cho người dùng TOTP và chỉ đổi đúng trường hợp có lỗ.

Đã cân nhắc: không tính passkey là MFA (khoá người dùng chỉ có passkey khỏi role của họ); MFA theo phiên (như
trên); chờ kiểm toán bên ngoài (chưa có ngân sách, D-20261121-vn-providers).
