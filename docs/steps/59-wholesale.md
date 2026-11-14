# Bước 59 — Bán sỉ B2B (`wholesale`)

> Lộ trình: `docs/ROADMAP.md` bước 59 · Quyết định: D-221 · Đặc tả liên quan: 36 (công nợ, hoá đơn), 41 (bảng
> giá theo nhóm khách), 45 (đơn hàng), 47 (khách hàng)

## Mục tiêu

Bán cho doanh nghiệp trên cùng hệ thống bán lẻ: tài khoản công ty có nhiều người mua, giá sỉ và số lượng tối
thiểu, báo giá có hạn, đặt hàng trả chậm (công nợ) trong hạn mức tín dụng, hoá đơn GTGT ghi đúng mã số thuế
người mua, duyệt đơn nội bộ phía công ty mua.

## Bắt đầu khi

Bước 45 (đơn hàng) và 47 (khách hàng) xong; dùng sổ cái và hoá đơn của 36, bảng giá theo nhóm khách của 41.

## Phạm vi

Làm: công ty mua (tài khoản B2B), người mua thuộc công ty và quyền của họ, bảng giá sỉ, bậc giá theo số
lượng, số lượng tối thiểu / bội số, báo giá (yêu cầu → báo → chấp nhận thành đơn), điều khoản thanh toán
(trả trước / trả chậm N ngày), hạn mức tín dụng, tuổi nợ và nhắc nợ, duyệt đơn phía người mua, đặt hàng nhanh
theo SKU / CSV.

Không làm: đấu thầu, hợp đồng khung nhiều năm, EDI — để sau.

## Thiết kế

- **Công ty mua** (`wholesale_companies`: `tenant_id`, `org_id` của bên bán, `name`, `tax_id` (kiểm như hồ sơ
  thuế bước 36), địa chỉ hoá đơn và giao hàng (`Address` của bước 34), `price_list_id`, `payment_terms`
  (`prepaid` / `net_7` / `net_15` / `net_30` / `net_45`), `credit_limit_minor` + mã tiền tệ, `status`).
  Thành viên `wholesale_company_members` (`user_id` tài khoản người mua của bước 34, vai trò trong công ty:
  `buyer`, `approver`, `admin`, ngưỡng tự đặt không cần duyệt).
- **Giá:** bảng giá sỉ là bảng giá của `catalog` (bước 41) gắn cho công ty; thêm bậc giá theo số lượng
  (`catalog_price_tiers`) và `min_qty`, `qty_step` theo biến thể — `catalog` khai, `wholesale` chỉ chọn bảng giá.
  Giá sỉ **không** hiện công khai; chỉ người mua thuộc công ty đã duyệt thấy.
- **Báo giá:** `wholesale_quotes` (`number` đánh số của kernel, dòng, giá đề nghị, `valid_until`, `status`
  `requested/sent/accepted/expired/rejected/converted`). Chấp nhận → tạo đơn qua `OrderIntake` của 45 với giá đã
  chốt (không tính lại giá công khai); hết hạn → job đánh dấu.
- **Công nợ:** đơn `net_*` không thanh toán ngay; khi giao (hay khi lập hoá đơn, theo `invoice_timing`) ghi
  `receivable.customer[C]` trong sổ cái (36); hạn = ngày hoá đơn + số ngày. Kiểm hạn mức **trong transaction đặt
  đơn** bằng `UPDATE … SET used = used + x WHERE used + x <= limit` (không đọc rồi ghi). Thu tiền qua `ledger`
  (chuyển khoản / VietQR khớp tự động) trừ công nợ. Báo cáo tuổi nợ (0–30–60–90+), nhắc nợ qua `notifications`.
  Quá hạn quá N ngày → chặn đơn trả chậm mới (chính sách theo nút).
- **Duyệt phía người mua:** đơn vượt ngưỡng của người mua → `pending_buyer_approval`, người `approver` của công
  ty duyệt (cùng cơ chế duyệt của bước 51 P12, phạm vi công ty).
- **Đặt nhanh:** `POST /me/wholesale/quick-order {lines: [{sku, qty}]}` hay CSV qua `imports` (bước 56).

### API, MCP, quyền, lỗi

- REST `/api/v1/wholesale`: `companies` (CRUD, duyệt đăng ký), `members`, `quotes` (+ `send`, `accept`, `convert`),
  `credit` (tuổi nợ, điều chỉnh hạn mức có lý do và audit), `/me/wholesale/…` cho người mua.
- MCP: `list_quotes`, `send_quote`, `credit_report` (`read_only`), `adjust_credit_limit` (rủi ro — chờ duyệt
  với key agent).
- Permission: `wholesale.companies.manage`, `wholesale.quotes.manage`, `wholesale.credit.manage`,
  `wholesale.buy` (người mua). Tính năng gói `wholesale`.
- Lỗi: `wholesale.credit_limit_exceeded` (409, kèm còn lại), `wholesale.overdue_block` (409),
  `wholesale.min_qty` (422), `wholesale.quote_expired` (409), `wholesale.not_company_member` (403).

## Test bắt buộc (cả hai database)

`gia_si_khong_lo_cong_khai`, `bac_gia_theo_so_luong_dung`, `bao_gia_chap_nhan_thanh_don_gia_da_chot`,
`han_muc_khong_vuot_khi_dat_dong_thoi`, `qua_han_chan_don_tra_cham`, `thu_tien_tru_cong_no`,
`don_vuot_nguong_cho_nguoi_duyet_cua_cong_ty`, `hoa_don_ghi_ma_so_thue_cong_ty`.

## Căn cứ pháp lý

| Nội dung | Văn bản | Ghi chú |
|---|---|---|
| Hoá đơn GTGT cho người mua là tổ chức (tên, mã số thuế, địa chỉ) | NĐ 123/2020/NĐ-CP, sửa bởi NĐ 70/2025/NĐ-CP | Cần đối chiếu văn bản gốc |
| Hợp đồng mua bán hàng hoá, thanh toán trả chậm | Luật Thương mại 2005, Bộ luật Dân sự 2015 | Hợp đồng ký điện tử — chờ chủ dự án quyết mảng ký hợp đồng |
| Thanh toán không dùng tiền mặt cho giao dịch từ 5 triệu đồng (khấu trừ GTGT đầu vào) | Luật Thuế GTGT 48/2024/QH15 | Cần đối chiếu ngưỡng hiện hành |

## Agent làm bước này tự quyết

Module riêng hay mở rộng `customers`; mặc định điều khoản thanh toán; cách tính tuổi nợ. Ghi vào DECISIONS.
