## D-20261128-source-timeout-setting · Trần thời gian của nguồn module thành cấu hình; testkit đặt trần rộng — 2026-11-28 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: trần của mỗi nguồn module trong hộp việc, tìm chung, số liệu tóm tắt, bước thiết lập và mở theo mã đọc từ
`AUTOXIDE_SOURCE_TIMEOUT_MS` (50–60000; không đặt thì giữ `work::SOURCE_TIMEOUT`: 300 ms, 1500 ms ở bản debug), đọc một
lần ở `configure` của kernel và truyền vào state của năm endpoint. `TestApp` đặt 60000 khi bài test không tự đặt; bài
test về nguồn chậm đặt trần ngắn (`.setting("AUTOXIDE_SOURCE_TIMEOUT_MS", "5000")`) và cho nguồn chậm không bao giờ trả
lời kịp. Gỡ các vòng "hỏi lại tới khi đủ nguồn" trong test (`content` find, `comments`, kernel relocation).

Lý do: trần là đồng hồ thật. Đã dựng lại được: hạ ưu tiên Postgres và chạy song song, `GET /kernel/work` trả
`{"incomplete":["kernel.approvals","content.review","content.site_health"],"items":[]}` — bài
`content::http::tests::bulk::hoan_giao_viec_va_buoc_thiet_lap` đỏ 4/18 lần; sau sửa 18/18 xanh cùng điều kiện. Nâng trần
debug từ 300 lên 1500 ms trước đây không đủ, và ba bài test đã phải vá bằng vòng hỏi lại. Trần là cơ chế đúng cho
production (một nguồn chậm không làm treo hộp việc), nên không bỏ; chỉ cho cấu hình được — người vận hành có database
chậm cũng chỉnh được.

Đã cân nhắc:
- Nâng hằng số debug thêm nữa: vẫn là đồng hồ, máy bận hơn lại đỏ; bài test nguồn chậm chạy lâu hơn.
- Biến toàn cục cho testkit: các `TestApp` chạy song song trong một tiến trình sẽ ghi đè nhau.
- Hỏi lại trong mỗi bài test: là retry, che nguyên nhân, phải nhớ thêm ở mọi bài mới.
