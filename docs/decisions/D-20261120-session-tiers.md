## D-20261120-session-tiers · Phiên làm việc chọn model và mức nỗ lực theo loại việc — 2026-11-20 — chủ dự án quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: phiên điều phối mở phiên làm việc mới theo ba bậc:

| Loại việc | Model | Mức nỗ lực |
| --- | --- | --- |
| Thiết kế module mới; việc chạm tiền, pháp lý hay phân quyền (vd bước 45, 46, 54, 59, 65) | mạnh nhất | cao |
| Thêm mục vào module đã có (vd phần còn lại của bước 41, 47, 55, 69) | mạnh nhất | mặc định |
| Việc máy móc: tối ưu CI, viết tài liệu, đo hiệu năng | nhanh, rẻ hơn | mặc định |

Việc lẫn hai loại thì lấy bậc cao hơn. Gate và quy trình merge (D-20261119-merge-batches) giữ nguyên cho mọi bậc.

Lý do: trước đây mọi phiên chạy cùng model mạnh nhất ở mức mặc định, nhiều phiên song song làm hạn mức sử dụng gần chạm
trần. Việc khó và rủi ro cao vẫn được model mạnh nhất với mức nỗ lực cao; việc máy móc dùng model rẻ hơn.

Đã cân nhắc: giữ một model cho mọi phiên (đơn giản nhưng tốn hạn mức); để từng phiên tự chọn (không nhất quán).
