# Đề bài: API cho trợ giúp trong giao diện

Đề bài này giao cho một agent làm việc trong repo Autoxide. Đọc `CLAUDE.md` trước; mọi luật ở đó áp
dụng (cả SQLite và Postgres, module sở hữu bảng, OpenAPI đầy đủ, i18n, gate).

Bối cảnh: thiết kế giao diện 2 thêm trợ giúp ngay trong giao diện. Giao diện gồm nút ⓘ giải thích cạnh
từng mục, bảng "Trợ giúp" theo trang, và thẻ giới thiệu tính năng mới tắt được. Thiết kế và luật viết nội
dung ở `docs/research/ui-design.md` mục 6. Phần lớn là việc của repo giao diện. Máy chủ chỉ cần hai thứ
dưới đây.

## Việc phải làm

1. Ghi quyết định bằng skill `record-decision`. Lấy số kế tiếp chưa dùng trong `docs/DECISIONS.md`; số
   D-202 đã dành cho đề bài `docs/prompts/ui-api-gaps.md`.
2. Thêm hai mục vào bước 51 trong `docs/ROADMAP.md` và đặc tả `docs/steps/51-ui-contracts.md`, rồi làm
   code. Cả hai đều thuộc kernel và không cần module mới.
3. Gate đầy đủ (`./gate.sh`) xanh trên cả hai database. Sinh lại `docs/openapi.json`. Thêm bản dịch
   `locales/vi.json` cho mọi i18n key mới. Chỉ thêm API mới, không đổi hành vi endpoint đang có (D-059).

## A. Trạng thái gợi ý theo người dùng

Thẻ "Mới: lưu view" có nút "Đã hiểu". Danh sách "Có gì mới" cần biết người dùng đã xem tới đâu. Nếu chỉ
lưu ở trình duyệt thì đổi máy là gợi ý hiện lại, gây phiền.

- **Bảng.** Thêm bảng `kernel_user_flags` bằng migration mới, có `down`. Các cột:
  - `tenant_id`, `user_id`, `key`: `key` dài tối đa 100 ký tự, chỉ gồm `a-z`, `0-9`, `.`, `_`, `-`;
  - `value`: chuỗi ngắn, tối đa 200 ký tự, ví dụ `dismissed` hay mốc phiên bản đã xem;
  - `updated_at`.

  Khoá là (`tenant_id`, `user_id`, `key`). Đây là bảng cấp khách hàng, mọi truy vấn lọc theo khách qua
  công cụ của kernel.
- **API.**
  - `GET /api/v1/me/flags?prefix=hint.`: trả về các flag của chính người gọi.
  - `PUT /api/v1/me/flags/{key}` với thân `{ "value": "dismissed" }`: ghi đè, gọi lại nhiều lần vẫn an
    toàn.
  - `DELETE /api/v1/me/flags/{key}`.

  Chỉ đọc và ghi flag của chính mình; không có endpoint đọc flag của người khác.
- **Giới hạn.** Mỗi người tối đa 500 flag mỗi khách, cấu hình được bằng `Setting`. Vượt mức thì trả lỗi
  có `code` riêng, kèm câu nói rõ phải làm gì.
- **Dữ liệu cá nhân.** Flag là dữ liệu cá nhân, nên phải nằm trong gói "xuất dữ liệu của tôi" và bị xoá
  khi xoá tài khoản. Nối vào cơ chế sẵn có của bước 37; chưa có thì khai điểm nối và ghi rõ trong quyết
  định.
- **Test**, chạy trên cả hai database. Các test tối thiểu:
  - `flag_ghi_lai_cung_key_la_ghi_de`;
  - `khong_doc_duoc_flag_cua_nguoi_khac`;
  - `flag_cua_khach_nay_khong_lo_sang_khach_khac`;
  - `key_sai_dinh_dang_bi_tu_choi`;
  - `vuot_han_muc_flag_bao_loi_ro`;
  - `xoa_tai_khoan_xoa_flag`.

  Thêm một test đồng thời: hai lệnh `PUT` cùng key cùng lúc không tạo hai dòng.

## B. Định nghĩa của số liệu tóm tắt

Hộp ⓘ "Doanh thu hôm nay tính thế nào?" phải nói đúng cách máy chủ tính. Vì vậy câu định nghĩa nằm cùng
chỗ với code tính số, không viết riêng trong repo giao diện.

- **Trait.** Thêm vào trait `SummaryMetric` (P4 bước 51) hàm `fn definition(&self) -> Option<&'static str>`
  trả về i18n key, có cài đặt mặc định `None` (D-038, không bắt module cũ sửa).
- **Endpoint.** `GET /api/v1/kernel/summary` trả thêm trường `definition` (câu đã dịch theo ngôn ngữ của
  request), hoặc `null` khi số liệu không khai định nghĩa. Mô tả trường này trong OpenAPI.
- **Câu định nghĩa** nói rõ bốn điều, đúng với code tính:
  - tính từ lúc nào, theo múi giờ nào;
  - gồm hay không gồm thuế;
  - có trừ hoàn tiền không;
  - đơn trả khi nhận hàng (COD) tính lúc nào.
- **Gate.** Thêm một bài kiểm trong gate: mọi `SummaryMetric` của module đang có đều khai `definition`,
  và key đó có bản dịch.

## Báo cáo khi xong

Báo cáo bằng tiếng Việt, lời thường, cho chủ dự án mới học Rust. Nội dung báo cáo:

- đã thêm gì;
- gate có xanh trên cả hai database không;
- mục nào đã vào lộ trình.

Commit trên nhánh làm việc. Không push khi chủ dự án chưa yêu cầu.
