## D-20261128-agent-approval-audit · Rà lệnh rủi ro: mọi lệnh xoá, chạm tiền, không hoàn tác chờ người duyệt khi agent gọi — 2026-11-28 — agent quyết

Chủ đề: MCP, AI, agent dùng API

Quyết định:

- Mọi `DELETE` của `/api/v1`, và mọi `POST` / `PUT` / `PATCH` mà đoạn cuối đường dẫn là động từ chạm tiền hay không hoàn
  tác được (`refunds`, `payouts`, `pay`, `mark-paid`, `void`, `reverse`, `erase`, `purge`, `issue`, `send`, `cancel`,
  `close`, `retire`, `suspend`, `merge`, `move`, `relocations`, `fulfil`, `replace`, `adjust(ments)`, `payments`, `delete`)
  mang `x-approval: "agent"` (bước 51 P12, D-365): agent gọi thì nhận 202 và yêu cầu duyệt. Tool MCP khai `destructive`
  thì khai `requires_approval_for_agent`; tool làm cùng việc với endpoint có duyệt (`invoices.issue`,
  `ledger.create_payout`, `newsletter.send_campaign`…) cũng khai, vì MCP không được làm điều REST cấm (luật 8).
- Hai luật gate `api-approval` (trên `docs/openapi.json`) và `mcp-approval` (trên `docs/mcp-tools.json`) giữ điều này cho
  endpoint và tool mới. Ngoại lệ ghi ở `xtask/approval-allow.txt`, bắt buộc lý do, chỉ ba loại: thu hồi quyền (phải tức
  thì; hoàn tác bằng cách cấp lại), dữ liệu nhỏ của chính người gọi, trạng thái quy trình không mất dữ liệu hay là hướng
  an toàn (dừng gửi, dừng khuyến mãi). Endpoint công khai (`x-public`) và SCIM đứng ngoài. Lệnh của module đang được chuyển
  sang lớp kernel ở phiên song song (comments, files, forms, mail, media, webhooks) ghi tạm trong file đó, gắn duyệt sau
  khi phiên kia merge — phần này chỉ giảm.
- Lệnh không đòi permission nào (việc của chính người gọi: huỷ hay trả tiền đơn của mình, xoá ghi chú của mình…): người
  duyệt là **chính chủ của agent**, trong phiên đăng nhập của họ. Người khác — kể cả có mọi quyền — không duyệt thay;
  agent và client OAuth không tự duyệt (như trước).
- `docs/mcp-tools.json` ghi đúng `requires_approval_for_agent` của tool (trước đây luôn `false`).
- Thay đổi phá vỡ trước 1.0 (D-20261125-api-break-before-1-0): agent chạy không người trông nay nhận 202 /
  `approval_required` ở các lệnh trên; ghi trong `xtask/api-breaking-allow.json` và changelog.

Lý do: trước đây chỉ một endpoint (`DELETE /content/entries/{id}`) có duyệt, nên agent có thể hoàn tiền, chi trả, phát
hành hoá đơn điện tử, xoá khách hay xoá dữ liệu mà không người nào thấy trước. Luật theo hình dạng đường dẫn bắt được lệnh
mới mà không cần ai nhớ; ngoại lệ phải nêu lý do nên người rà thấy ngay chỗ nào được miễn.

Đã cân nhắc: phân loại thủ công từng endpoint không có gate — loại vì endpoint mới sẽ quên. Duyệt mọi lệnh ghi — loại vì
agent không làm được việc thường ngày (sửa bài, chuyển trạng thái đơn). Duyệt cả lệnh thu hồi credential — loại vì làm
chậm ứng phó sự cố.
