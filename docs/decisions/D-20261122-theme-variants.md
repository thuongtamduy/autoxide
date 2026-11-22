## D-20261122-theme-variants · Chủ đề theo lịch: lịch JSON ba loại theo múi IANA của website, chọn đúng một bản, mẫu nhúng binary — 2026-11-22 — agent quyết

Chủ đề: Module nội dung, file, liên lạc

Quyết định (bước 67 mục "chủ đề theo mùa, ngày lễ, khung giờ", chi tiết hoá D-234 và mục 3a của
`docs/steps/67-page-builder.md`):

1. `content_theme_variants` theo website (nút; bản của nút cấp trên áp cả cho website bên dưới — chuỗi đặt Tết cho
   mọi cửa hàng): ghi đè token (cùng bộ kiểm với theme gốc, lỗi ghi `overrides.<token>`), hiệu ứng trong danh sách
   `snow` / `petals` / `lanterns` / `fireworks` / `confetti` / `hearts` (core chỉ trả tên), cường độ `low` /
   `medium`, lịch, ưu tiên 0–1000, trạng thái `draft` / `active` / `paused`. Tạo luôn là nháp; xoá bản đang bật →
   409 `content.theme_variant_active`; trần mỗi website `AUTOXIDE_CONTENT_THEME_VARIANTS_MAX` (50) → 409
   `content.theme_variants_limit` (hạn mức theo gói để khi bước 35 cần).
2. Lịch: `once` (`starts_at`, `ends_at` UTC — cột riêng để lọc bản đã hết trong database), `daily` (`start`,
   `end` giờ địa phương `HH:MM`, `end` trước `start` là qua nửa đêm, `from` / `until` tuỳ chọn), `weekly` (thêm
   `days`). Giờ địa phương theo `organizations.timezone` của website, đổi qua `jiff` (đúng giờ mùa hè; giờ không
   tồn tại do nhảy giờ thì bỏ lần đó); không có module `organizations`: UTC.
3. Chọn bản hiệu lực lúc `t`: bản `active` có lần chạy phủ `t`; so (ưu tiên, giờ bắt đầu của lần đang chạy, `id`)
   lớn nhất thắng — đúng một kết quả, mọi máy như nhau. Mốc đổi kế tiếp: mốc bắt đầu / kết thúc gần nhất của mọi
   bản đang bật trong 7 ngày tới (không quét xa hơn). Hàm nhận `now` để test.
4. Thư viện 9 chủ đề mẫu (`templates/theme-presets.json`, nhúng binary): lễ ngày cố định ghi `MM-DD`, lễ âm lịch
   và ngày đổi theo năm (Tết, Trung thu, Black Friday) ghi ngày từng năm 2027–2029 — dữ liệu, cần đối chiếu lịch
   chính thức mỗi lần cập nhật gói mẫu. Tạo từ mẫu: lịch `once` từ 00:00 ngày đầu tới 00:00 sau ngày cuối, giờ
   địa phương của website.
5. `GET /site/{org_id}/preview?at&variant_id` trả token đã ghép, hiệu ứng, bản đang chạy, `next_change_at`, cảnh
   báo tương phản; `variant_id` coi một bản (kể cả nháp) như đang bật. `GET /public/site` (mục sau) dùng cùng hàm
   chọn và thêm cache tới `next_change_at`.

Lý do: lịch là dữ liệu có schema, kiểm một lần, không phụ thuộc giờ máy chủ; chọn đúng một bản cho kết quả dễ
đoán; mẫu theo năm là dữ liệu dễ cập nhật mà không đổi code.

Đã cân nhắc: cron cho lịch (khó đọc với người không biết, khó kiểm qua nửa đêm và giờ mùa hè); chồng nhiều bản
cùng lúc (kết quả khó đoán); tính lịch âm trong code (thêm thư viện cho chín ngày mỗi năm).
