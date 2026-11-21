## D-20261121-grant-scope-check · Cấp role: so cả phạm vi của permission, kiểm lại người mời lúc nhận lời mời — 2026-11-21 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: "Chỉ cấp được quyền mình có" (`grant_problems`, dùng cho membership, lời mời, luật nhóm SCIM, role
mặc định của nhà cung cấp OIDC) so từng permission của role KÈM độ sâu mà role sẽ phủ khi gán ở nút được chọn:
người gán phải có permission đó với một grant bao được — `tenant` bao mọi thứ; cần `tenant` thì chỉ `tenant` bao
được; `subtree` ở nút đó hay một tổ tiên bao `subtree` / `unit` / `own`; `unit` ở đúng nút bao `unit` / `own`; `own`
ở đúng nút chỉ bao `own`. API key gắn nút không cấp được grant `tenant` hay ở nút ngoài rào; role `admin` chỉ người
toàn quyền không bị rào cấp được. Lúc chấp nhận lời mời, quyền HIỆN TẠI của người mời được đọc lại và kiểm như lúc
mời (còn hoạt động, còn `auth.invitations.manage` ở nút, còn bao được các role); không còn thì lời mời không dùng
được (`auth.invitation_invalid`, `reason = inviter_not_allowed`). Lời mời do lệnh CLI tạo không kiểm.

Lý do: so tên thôi thì người có một permission ở một nhánh cấp được role phủ cả khách với cùng permission đó.
Permission không theo nút luôn được cấp ở độ sâu `tenant` (D-303 mục 3) nên so độ sâu trực tiếp là đủ, không cần
danh mục permission. Kiểm lại lúc nhận vì giữa lúc mời và lúc nhận người mời có thể mất quyền hay role được sửa rộng
hơn.

Đã cân nhắc: chỉ so phạm vi ở luồng lời mời — bỏ sót membership, SCIM và OIDC dùng chung hàm; chuyển hàm so phạm vi
vào kernel (`Principal`) — chỉ `auth` cấp role, để trong `auth` không phải đổi kernel.
