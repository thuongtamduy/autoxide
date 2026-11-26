## D-20261126-official-modules-in-repo · Module chính thức ở lại repo này tới 1.0, xem lại sau 1.0 — 2026-11-26 — chủ dự án quyết

Chủ đề: Kiến trúc module và kernel

Quyết định:

1. Mọi module do dự án viết (`modules/*`, mọi hạng: lõi, chính thức, thử nghiệm, kể cả bộ nối nhà cung cấp) ở lại repo
   này tới bản 1.0. Không tách sang repo riêng trước 1.0.
2. Ranh giới giữ bằng luật đã có, không bằng repo: mỗi module một crate, chỉ mở `api` và `module()`, không đọc bảng của
   module khác, không foreign key xuyên module (`cargo xtask check`); người dùng chỉ chạy module mình cần bằng
   `AUTOXIDE_MODULES` (bước 70 P1). Core vẫn trung lập với nghiệp vụ.
3. Module của người ngoài nằm ở repo của họ, ghép bằng crate `autoxide` như thư viện (D-20261121-external-modules); ứng
   dụng tham chiếu (bước 71 G11) thử đúng đường này.
4. Sau 1.0 xem lại, ghi thành quyết định mới: có tách nhóm module (vd thương mại) sang repo riêng với nhịp phát hành độc
   lập hay không, dựa trên số người dùng thật, nhịp thay đổi của từng nhóm và chi phí CI.

Lý do: bước 71 G2 chuyển 39 module sang lớp mới của kernel (khoảng 6.000 chỗ gọi). Cùng một repo thì mỗi thay đổi kernel
sửa module trong cùng pull request, một CI kiểm, không lệch phiên bản giữa các repo. 39 module thật chạy trên hợp đồng
mới là bằng chứng hợp đồng đủ dùng cho người ngoài. Các dự án cùng loại thường giữ module chính thức trong một repo, mỗi
module đăng thành gói riêng.

Đã cân nhắc: tách nhóm thương mại sang repo riêng ngay (mỗi đổi của kernel trước 1.0 phải đi qua nhiều repo, chậm và dễ
lệch); tách mọi module, chỉ giữ kernel (cùng vấn đề, nặng hơn; người mới phải ghép nhiều repo mới chạy được bản đầy đủ).
