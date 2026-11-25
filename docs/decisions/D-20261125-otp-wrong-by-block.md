## D-20261125-otp-wrong-by-block · Mã đăng nhập SMS nhập sai: đếm theo số + khối địa chỉ người xin mã, kèm trần chung 5× — 2026-11-25 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: hạn mức `AUTOXIDE_COMPLIANCE_OTP_WRONG_PER_PHONE_PER_DAY` (mặc định 10) của mã đăng nhập bằng số tính trên
những mã của số đó **xin từ cùng khối địa chỉ** (/24 IPv4, /64 IPv6) trong 24 giờ; cột mới
`compliance_phone_challenges.requester_block` giữ dấu HMAC của khối (`ip_hash::fingerprint`, phạm vi `compliance.otp`,
trống khi không biết địa chỉ — một nhóm riêng). Đủ hạn mức thì khối đó không xin và không kiểm mã cho số đó nữa. Thêm
trần chung: mọi khối cộng lại đủ 5 × hạn mức thì số bị khoá ở mọi nơi như trước. Mã xác thực số của người đã đăng nhập
không có khối (họ đã có tài khoản và có giới hạn theo người).

Lý do: pentest 2026-11 (PT-7) — hạn mức cũ tính trên mọi mã của số, nên người lạ xin 2 mã cho số của nạn nhân, nhập
sai 10 lần là khoá nạn nhân 24 giờ. Lần sai xảy ra trên mã do chính kẻ đó xin, nên gắn lần sai với khối của người xin
mã tách được kẻ phá khỏi chủ số mà không phải đổi hợp đồng `PhoneCodes::verify` (không mang địa chỉ). Chống đoán mã vẫn
giữ: mỗi mã tối đa `OTP_MAX_ATTEMPTS` lần, số mã mỗi giờ của một số và của một khối có giới hạn, và tổng lần sai mỗi
ngày của một số có trần hữu hạn (mặc định 50 trên 10^6 mã 6 chữ số, xác suất đoán trúng ~5·10^-5 mỗi ngày). Muốn khoá
chủ số, kẻ phá phải xin mã từ ít nhất 5 khối khác nhau (5 × 2 tin SMS) — đắt hơn và lộ hơn nhiều.

Đã cân nhắc: chỉ rút ngắn thời gian khoá (vẫn khoá được nạn nhân liên tục); đếm theo khối của người **nhập** mã (phải
thêm địa chỉ vào `PhoneCodes::verify`, đổi hợp đồng công khai của `auth`); bỏ hạn mức ngày (mở rộng lượt đoán phân tán
không giới hạn).
