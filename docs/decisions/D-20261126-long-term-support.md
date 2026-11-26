## D-20261126-long-term-support · Hỗ trợ dài hạn từ 1.0: vá bảo mật theo con số cụ thể, vá ngược từ nhánh release/X.Y — 2026-11-26 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: Từ 1.0, mỗi bản lớn có hỗ trợ dài hạn theo các con số ghi ở `SECURITY.md` (mục "Supported versions") và
`docs/COMPATIBILITY.md` (mục "Support"):

- Bản minor mới nhất của bản lớn hiện tại nhận mọi bản sửa (lỗi và bảo mật).
- Bản minor liền trước nhận vá **bảo mật** trong **3 tháng** sau khi bản minor kế tiếp ra.
- Khi bản lớn mới ra, bản minor cuối của bản lớn trước nhận vá bảo mật trong **6 tháng**.
- Bản cũ hơn không được vá. Trước 1.0 chỉ bản minor mới nhất của dòng 0.x được vá.
- Vá ngược: bản sửa làm trên `main` trước; người duy trì cherry-pick vào `release/X.Y` cắt từ tag `vX.Y.Z` mới nhất của dòng đó,
  ra bản patch kế tiếp chỉ chứa bản sửa, sau gate đầy đủ trên hai database (`docs/RELEASING.md`, "Dòng hỗ trợ dài hạn").
- Mức nghiêm trọng cao và nghiêm trọng: phát hành ngoài lịch, cho mọi dòng còn hỗ trợ.

Lý do: dự án cộng đồng nhỏ, không SLA: cam kết phải là thứ người duy trì làm được thật. 6 tháng cho bản lớn trước đủ để người
dùng nâng cấp theo công cụ `autoxide upgrade` (G3) mà không buộc bảo trì song song nhiều dòng; con số này là điểm xuất phát,
tăng được (nhiều nhất 12 tháng) bằng một quyết định mới khi có người dùng thực sự bị kẹt ở bản lớn cũ.

Đã cân nhắc: 12 tháng ngay từ đầu (gấp đôi công vá ngược khi chưa có người dùng cần); chỉ bản mới nhất được vá (người dùng
bị ép nâng ngay cả khi chỉ cần vá lỗi bảo mật); LTS riêng theo bản minor chọn (thêm một loại dòng để nhớ).
