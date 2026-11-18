## D-20261118-decision-ids · Quyết định mới mỗi mục một file, ID theo ngày và tên; mục lục tự sinh — 2026-11-18 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. **Mục mới là một file** `docs/decisions/D-<YYYYMMDD>-<tên-ngắn>.md`, chứa đúng một mục theo mẫu cũ (tiêu đề `## <ID> ·
   <tiêu đề> — <YYYY-MM-DD> — <ai quyết>`, rồi `Chủ đề:`, `Quyết định:`, `Lý do:`, `Đã cân nhắc:`, `Thay:`). ID = `D-` + ngày
   của mục + `-` + tên ngắn tiếng Anh hay tiếng Việt không dấu, chữ thường `a-z0-9` nối bằng `-`, tối đa 40 ký tự (vd
   `D-20261118-decision-ids`). Tham chiếu viết nguyên ID: `(D-20261118-decision-ids)`.
2. **`docs/DECISIONS.md` là nhật ký cũ, đóng**: D-001…D-381 giữ nguyên số, nội dung và mọi tham chiếu; không thêm mục vào
   đó. Mục số tuần tự mà nhánh cắt trước quy ước này đã thêm (số lớn hơn 381, vd của phiên làm bước 51/52) vẫn hợp lệ nếu không trùng.
3. **Mục lục** `docs/decisions/INDEX.md` sinh bằng `cargo xtask decisions` từ cả hai nguồn: gom theo chủ đề (dòng `Chủ đề:`
   của mục, phải là một tên trong mục lục; không có thì đoán từ tên module và từ khoá trong tiêu đề), mỗi dòng có ID,
   tiêu đề, ngày, dấu "chủ dự án quyết", và "thay bởi" / "thay một phần bởi" suy từ đoạn `Thay…` của các mục sau.
4. **Gate** (`cargo xtask check`): `decisions-index` — mục lục lệch thì đỏ, thông báo ghi lệnh sinh lại; `decisions-id` —
   ID trùng, ID sai dạng, file không đúng tên ID hay không đúng một mục, `Chủ đề:` lạ; `decisions-ref` — `D-xxx` /
   `D-<ngày>-<tên>` nhắc tới trong file văn bản của repo (code, tài liệu, cấu hình) mà không có mục nào.

Lý do: hai phiên agent chạy song song trên hai nhánh cùng gốc đều thấy cùng "số cuối" nên luôn chọn trùng số kế tiếp — D-365,
D-372/373, D-377 đã trùng và phải đổi số tay khi gộp, kèm mọi chỗ code trích số đó. ID theo ngày và tên do chính phiên chọn
không cần phối hợp; mỗi mục một file nên git không xung đột khi hai nhánh cùng thêm mục (thêm vào cuối một file chung luôn
xung đột). Hai phiên trùng cả ngày lẫn tên (hiếm, và chỉ khi cùng việc) thì git báo "cùng thêm một file" và luật không
trùng bắt nốt. Nhật ký 9400 dòng không có mục lục thì agent không biết mục nào đã bị thay nếu không đọc hết.

Đã cân nhắc: số ngẫu nhiên hay hash (không trùng nhưng không đọc được, agent không đoán được nội dung từ ID); dải số riêng
cho từng phiên (phải phối hợp trước, phiên mới không biết dải nào còn trống); chuyển cả 381 mục cũ sang file riêng (đổi
mọi đường dẫn đang trích, lợi ít — mục cũ không còn bị thêm vào); viết tay mục lục (lệch ngay sau mục mới đầu tiên).
