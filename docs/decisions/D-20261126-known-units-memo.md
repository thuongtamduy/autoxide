## D-20261126-known-units-memo · Bộ nhớ "đơn vị có trong cây" của `Principal` sống một request — đánh đổi khi tách / gộp khách — 2026-11-26 — agent quyết

Chủ đề: Đa khách, tổ chức, gói

Quyết định: `Principal` nhớ các đơn vị đã tra thấy có trong cây (`OrgTree::contains`, D-20261125-org-must-exist) theo
(khách, đơn vị), chung cho các bản clone, nên bộ nhớ sống đúng một request hay một lần chạy job; chỉ nhớ kết quả "có",
trần 256 mục. Một request kiểm nhiều permission ở cùng đơn vị (điều hướng, việc cần làm, tìm, tổng quan, `can`) tra cây
một lần thay vì một lần mỗi permission.

Lý do: D-20261125-org-must-exist thêm một truy vấn closure cho người có quyền toàn khách; các đường lặp qua mọi nguồn /
mục với cùng một đích nhân truy vấn đó lên N lần. Đơn vị không bị xoá khi làm việc thường (chỉ đổi trạng thái); chỉ
tách / gộp khách (relocate) xoá cây của khách nguồn. Request hay job đang chạy đúng lúc đó có thể còn nhớ đơn vị vừa bị
dời đi tới khi nó kết thúc — chấp nhận được: lúc đó dữ liệu của đơn vị cũng đã dời sang khách đích, request không còn
gì để ghi vào đơn vị ấy ở khách nguồn.

Đã cân nhắc: kiểm một lần trước mỗi vòng lặp ở từng chỗ gọi (năm chỗ hôm nay, chỗ mới quên); cache chung cả tiến trình
theo (khách, đơn vị) (phải huỷ khi tách / gộp khách, dễ sai giữa nhiều máy chủ).
