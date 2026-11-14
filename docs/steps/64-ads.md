# Bước 64 — Quảng cáo trong sàn (`ads`)

> Lộ trình: `docs/ROADMAP.md` bước 64 · Quyết định: D-221 · Đặc tả liên quan: 46 (sàn), 57 (`analytics` — hiển
> thị, lượt bấm), 36 (phí, hoá đơn), 41 (tìm kiếm, danh sách)

## Mục tiêu

Người bán trên sàn trả tiền để sản phẩm hiện ở vị trí ưu tiên (đầu kết quả tìm kiếm, danh mục, trang chủ), có
nhãn "Quảng cáo" rõ ràng; tính phí theo lượt bấm hay lượt hiển thị trong ngân sách người bán đặt; sàn đo hiệu quả
và thu phí như một dịch vụ của sàn.

## Bắt đầu khi

Bước 46 (sàn), 57 (`analytics`), 36 (phí sàn, hoá đơn), 41 (tìm kiếm công khai).

## Thiết kế

- **Chiến dịch** (`ads_campaigns`: `seller_org_id`, sản phẩm, vị trí (`search`, `category`, `home`), từ khoá hay
  danh mục nhắm tới, cách tính (`cpc` / `cpm`), giá thầu tối đa, ngân sách ngày và tổng, lịch, `status`).
- **Chọn quảng cáo hiển thị:** khi `catalog` trả danh sách công khai, điểm cắm `ListingAugmenter` (catalog khai,
  `ads` gắn vào) chèn tối đa K vị trí quảng cáo; xếp theo giá thầu × mức liên quan (điểm tìm kiếm) × chất lượng
  (tỉ lệ bấm lịch sử). Phiên đấu giá giây thứ hai (người thắng trả mức vừa đủ vượt người kế + 1 đơn vị) — công
  thức ghi trong DECISIONS, không đổi ngầm. Sản phẩm hết hàng, gian hàng bị khoá, sản phẩm chưa qua kiểm duyệt
  → không chạy.
- **Nhãn bắt buộc:** mọi kết quả quảng cáo có `sponsored: true` trong API và mã nhãn `ad` để giao diện hiện
  "Quảng cáo / Được tài trợ"; không trộn lẫn mà không nhãn (Luật 19/2023, Luật Quảng cáo).
- **Đếm hiển thị và bấm:** qua `analytics` (bước 57) — sự kiện `ad_impression`, `ad_click` có mã phiên đấu giá
  ký HMAC (chống giả); lọc bot và bấm lặp (cùng phiên, 30 phút); gộp theo giờ.
- **Tính phí:** cuối ngày gộp số bấm / hiển thị hợp lệ → phí; trừ ngân sách bằng `UPDATE … WHERE spent + x <=
  budget`; phí là **doanh thu dịch vụ của sàn**, thu bằng trừ vào tiền quyết toán của người bán (`ledger`) hay hoá
  đơn định kỳ, có hoá đơn GTGT. **Không** có "ví quảng cáo" giữ tiền trả trước của người bán trên sổ sàn khi chưa
  đối chiếu luật (D-187); nạp trước chỉ qua đối tác có giấy phép.
- **Báo cáo** cho người bán: hiển thị, bấm, chi phí, đơn và doanh thu quy về quảng cáo (bấm trong 7 ngày trước
  khi mua — mô hình quy về ghi trong DECISIONS).

### API, MCP, quyền, lỗi

- REST `/api/v1/ads`: `campaigns` (người bán, phạm vi gian hàng), `reports`, `placements` (sàn cấu hình vị trí, số
  ô, giá sàn), `policies` (danh mục cấm quảng cáo).
- MCP: `campaign_report` (`read_only`), `pause_campaign`, `update_bid` (chờ duyệt với key agent khi tăng ngân sách).
- Permission: `ads.campaigns.manage` (người bán, scoped), `ads.platform.manage` (sàn).
- Event: `ads.campaign_changed`, `ads.budget_exhausted`, `ads.charges_posted`.
- Lỗi: `ads.budget_exhausted` (409), `ads.product_not_eligible` (422), `ads.category_restricted` (422).

## Test bắt buộc (cả hai database)

`quang_cao_luon_co_nhan`, `khong_vuot_ngan_sach_khi_bam_dong_thoi`, `bam_lap_trong_phien_khong_tinh_hai_lan`,
`ma_dau_gia_gia_bi_tu_choi`, `san_pham_het_hang_khong_chay`, `phi_ghi_so_cai_va_hoa_don`,
`gia_thau_giay_thu_hai_dung`.

## Căn cứ pháp lý

| Nội dung | Văn bản | Ghi chú |
|---|---|---|
| Nhận diện quảng cáo, nội dung quảng cáo trên môi trường mạng | Luật Quảng cáo 2012 và luật sửa đổi; NĐ 181/2013/NĐ-CP, NĐ 70/2021/NĐ-CP | **Cần đối chiếu bản hiện hành** |
| Minh bạch xếp hạng, không lừa dối người tiêu dùng | Luật 19/2023/QH15; Luật Thương mại điện tử 2025 | Cần đối chiếu |
| Hàng hoá cấm / hạn chế quảng cáo (thuốc, rượu…) | Luật Quảng cáo; danh mục là dữ liệu có ngày hiệu lực (bước 41) | Cần đối chiếu |

## Agent làm bước này tự quyết

Số ô quảng cáo mặc định; mô hình quy về đơn hàng; giá sàn tối thiểu. Ghi vào DECISIONS.
