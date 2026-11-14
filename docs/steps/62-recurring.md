# Bước 62 — Sản phẩm bán theo kỳ (`recurring`)

> Lộ trình: `docs/ROADMAP.md` bước 62 · Quyết định: D-221 · Đặc tả liên quan: 35 (`plans` — thuê bao của khách
> với nền tảng, **khác** bước này), 36 (thanh toán định kỳ qua đối tác), 41 (sản phẩm), 45 (đơn hàng)

## Mục tiêu

Cửa hàng bán theo kỳ cho người mua: hộp hàng tháng, cà phê giao mỗi 2 tuần, gói dịch vụ trả theo tháng, thành
viên trả phí. Mỗi kỳ tự tạo đơn và tự thu tiền qua đối tác có giấy phép; người mua tạm dừng, bỏ qua một kỳ, đổi
sản phẩm, huỷ dễ dàng.

`plans` (bước 35) là thuê bao **khách hàng trả cho nền tảng**; `recurring` là **người mua trả cho cửa hàng**. Dùng
chung bộ tính lịch kỳ trong kernel (`autoxide_kernel::billing_cycle` — thứ / tháng, cuối tháng, múi giờ) nếu bước
35 đã có, không viết hai lần.

## Bắt đầu khi

Bước 45 (đơn) và 36 (thanh toán định kỳ: `PaymentGateway` hỗ trợ token hoá phương thức thanh toán hay ủy quyền
trích nợ); 47 cho ưu đãi thành viên.

## Thiết kế

- **Gói bán theo kỳ** (`recurring_plans`: sản phẩm / biến thể của `catalog`, chu kỳ (`every N days|weeks|months`),
  giá mỗi kỳ hay giảm % so giá lẻ, số kỳ tối thiểu (0 = không ràng buộc), dùng thử).
- **Đăng ký của người mua** (`recurring_subscriptions`: `status` `trialing/active/paused/past_due/cancelled`,
  `next_run_at`, `payment_method_ref` — **chỉ mã tham chiếu** của đối tác, không lưu số thẻ, địa chỉ giao, dòng hàng).
- **Mỗi kỳ** (job `recurring.renew`, khoá chống chạy trùng theo (`subscription_id`, `period`)): tạo đơn qua
  `OrderIntake` của 45 (giữ hàng, giá theo gói), thu tiền qua `ledger` bằng phương thức đã lưu. Thất bại → `past_due`,
  thử lại theo lịch (1, 3, 7 ngày), báo người mua cập nhật phương thức; hết lượt → tạm dừng (không huỷ ngay).
- **Người mua tự làm:** tạm dừng / tiếp tục, bỏ qua kỳ kế, đổi biến thể / số lượng / ngày giao, đổi phương thức
  thanh toán, huỷ — **huỷ dễ như đăng ký** (một bước, không bắt gọi điện).
- **Thông báo trước khi trừ tiền** (theo cấu hình, mặc định 3 ngày) và sau mỗi kỳ; thông báo trước khi hết dùng
  thử và khi giá đổi (giá mới chỉ áp từ kỳ sau, cần đồng ý lại khi tăng — chính sách theo nút).
- Thuế và hoá đơn mỗi kỳ như đơn thường (36).

### API, MCP, quyền, lỗi

- REST `/api/v1/recurring`: `plans`, `subscriptions` (lọc trạng thái, kỳ kế), `POST /subscriptions/{id}/pause|resume|skip|cancel`;
  `/me/subscriptions` cho người mua; công khai: gói của sản phẩm trên trang sản phẩm.
- MCP: `list_subscriptions`, `subscription_metrics` (MRR, tỉ lệ rời bỏ — `read_only`), `pause_subscription`,
  `cancel_subscription` (rủi ro — chờ duyệt với key agent).
- Permission: `recurring.plans.manage`, `recurring.subscriptions.manage`; người mua dùng `/me`.
- Event: `recurring.subscription_created`, `…_renewed`, `…_payment_failed`, `…_paused`, `…_cancelled`.
- Lỗi: `recurring.payment_method_required` (422), `recurring.min_cycles_not_met` (409),
  `recurring.payment_method_unsupported` (422 — đối tác không hỗ trợ trích tiền định kỳ).

## Test bắt buộc (cả hai database)

`gia_han_khong_tao_hai_don_khi_job_chay_lai`, `thanh_toan_loi_thu_lai_roi_tam_dung`, `bo_qua_mot_ky`,
`huy_mot_buoc_co_hieu_luc_ngay_ky_sau`, `bao_truoc_khi_tru_tien`, `cuoi_thang_31_sang_thang_2_dung_ngay`,
`khong_luu_so_the`.

## Căn cứ pháp lý

| Nội dung | Văn bản | Ghi chú |
|---|---|---|
| Thanh toán định kỳ, ủy quyền trích nợ, token hoá thẻ — qua tổ chức có giấy phép | NĐ 52/2024/NĐ-CP; quy định của NHNN về ủy quyền trích nợ tự động | **Cần đối chiếu văn bản gốc**; core không giữ tiền (D-187) |
| Hợp đồng giao kết từ xa, thông tin trước khi giao kết, quyền huỷ | Luật 19/2023/QH15; NĐ 55/2024 | Cần đối chiếu |

## Agent làm bước này tự quyết

Lịch thử lại; mặc định báo trước; có gộp nhiều thuê bao cùng ngày thành một đơn. Ghi vào DECISIONS.
