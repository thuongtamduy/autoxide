## D-20261121-plans-reconcile-snapshot · Đối soát hạn mức của `plans` theo kiểu lạc quan: chụp sổ trước khi đếm, ghi dưới khoá chỉ khi sổ không đổi — 2026-11-21 — agent quyết

Chủ đề: Database, dữ liệu, job, event

Quyết định:

1. Job `plans.reconcile_usage` không ghi tuyệt đối `used = số thật + chỗ giữ đã đọc từ trước` nữa. Với mỗi khoá hạn
   mức (và mỗi phần `key.<phần>`): (a) chụp sổ của khách — `used`, tổng và số chỗ `held`, số chỗ `confirmed`;
   (b) đếm số thật ở database của khách (ngoài transaction, có thể ở máy chủ khác); (c) mở transaction ghi trên mặt
   điều khiển, khoá dòng `plans_usage` (`rows::lock_one`), chụp lại sổ. Hai ảnh giống nhau thì không có lần giữ /
   xác nhận / trả nào chen vào lúc đếm: ghi `used = số thật + Σ held`. Khác thì bỏ lượt, thử lại tối đa 3 lần, còn
   lại để lượt hằng ngày sau.
2. Mọi chỗ `held` đều tính (kể cả đã quá hạn): job `plans.expire_holds` trả chúng sau và trừ lại; trước đây chỗ quá
   hạn bị bỏ khỏi đích rồi còn bị trừ lần nữa khi trả.

Lý do: rà đồng thời tháng 11/2026 mục 7. Số thật nằm ở database của khách, không khoá chung được với sổ ở database
điều khiển, và không được gọi mạng trong transaction ghi. Ghi tuyệt đối sau khi đếm thì lần giữ chỗ chen giữa bị xoá
khỏi sổ — khách vượt hạn mức (cả SQLite lẫn Postgres). Chụp sổ hai lần biến khoảng hở thành phép kiểm: dưới khoá dòng
lượng dùng, lần giữ / trả đang chờ cộng trừ dòng đó phải đợi và cộng trừ tương đối sau khi ghi, nên vẫn đúng.

Đã cân nhắc:
- Cộng thêm các chỗ vừa xác nhận sau lúc bắt đầu đếm (trừ hao lệch đồng hồ): đếm thừa kéo dài tới lượt sau (một
  ngày), chặn nhầm khách đang sát hạn mức.
- Đếm hai lần và lấy số lớn: không bao được trường hợp vừa tạo vừa xoá trong khoảng hở.
- Chỉ ghi khi đích lớn hơn: không sửa được lệch lên do module xoá bản ghi mà quên trả chỗ — đúng việc đối soát cần làm.

Hệ quả: khách có lượt giữ / trả liên tục có thể không được đối soát trong một lượt (bỏ sau 3 lần thử); số liệu vẫn đúng
theo sổ. Bản ghi tạo ra giữa lúc giữ chỗ và lúc xác nhận (vài mili giây) có thể bị đếm hai lần tới lượt sau — lệch
về phía chặn, không bao giờ về phía cho vượt.
