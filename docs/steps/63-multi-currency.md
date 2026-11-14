# Bước 63 — Đa tiền tệ và bán xuyên biên giới

> Lộ trình: `docs/ROADMAP.md` bước 63 · Quyết định: D-221 · Đặc tả liên quan: 36 (tiền, sổ cái, thuế), 41 (giá),
> 44 (vận chuyển quốc tế), 45 (đơn)

## Mục tiêu

Cửa hàng bán cho người mua ở nước khác: hiện giá và thu tiền bằng tiền tệ của người mua, quy đổi về tiền tệ
ghi sổ đúng tỉ giá có ngày hiệu lực, thuế xuất khẩu, phí giao quốc tế. **Không** làm mua bán bằng ngoại tệ trong
lãnh thổ Việt Nam khi luật không cho phép.

## Bắt đầu khi

Bước 36, 41, 44, 45 xong. Không cần module mới: mở rộng `ledger`, `tax`, `catalog`, `orders`, `shipping` và kernel.

## Thiết kế

- **Tiền đã đúng từ đầu:** mọi số tiền là số nguyên đơn vị nhỏ nhất + mã ISO 4217 (`autoxide_kernel::money`); số
  chữ số thập phân theo bảng ISO (VND 0, USD 2, JPY 0, KWD 3) — kiểm lại khi làm.
- **Tỉ giá** (`ledger_fx_rates`: cặp tiền, tỉ giá dạng phân số nguyên `numerator/denominator`, nguồn, `effective_at`):
  nhập tay hay qua trait `FxRateSource` (bộ nối Vietcombank / ngân hàng / dịch vụ tỉ giá là module riêng; bản giả
  cho test). Không dùng số thực; làm tròn theo quy tắc ghi trong DECISIONS (nửa lên, ở bước cuối).
- **Giá theo thị trường:** "thị trường" = (quốc gia, tiền tệ, ngôn ngữ) gắn với website / tên miền của nút (bước
  34, 55). Giá: bảng giá riêng cho thị trường (41) hay quy đổi tự động có làm tròn giá đẹp (99, 990…) — chọn
  theo thị trường. Báo giá giỏ chốt tỉ giá tại lúc đặt, lưu vào đơn.
- **Sổ cái:** mỗi sổ có **tiền tệ ghi sổ** (VND với khách Việt Nam); giao dịch ngoại tệ ghi cả số gốc và số quy
  đổi theo tỉ giá ngày giao dịch; chênh lệch tỉ giá khi tiền về ghi vào tài khoản lãi / lỗ tỉ giá (36).
- **Thuế:** xuất khẩu hàng hoá hưởng thuế suất GTGT 0% khi đủ hồ sơ (Luật 48/2024 — cần đối chiếu) — `tax` thêm quy
  tắc theo quốc gia người mua; hàng bán cho người mua trong nước luôn tính và niêm yết bằng VND.
- **Chặn trái luật:** khách có `jurisdiction = VN` và người mua ở Việt Nam → chỉ VND; thị trường ngoại tệ chỉ cho
  địa chỉ giao / thanh toán ngoài Việt Nam (kiểm ở checkout). Lỗi `orders.currency_not_allowed`.
- **Thanh toán:** qua `PaymentGateway` hỗ trợ đa tiền tệ (36); tiền về tài khoản của đối tác / người bán, không
  qua nền tảng (D-187).
- **Vận chuyển quốc tế:** `shipping` thêm vùng quốc tế, hồ sơ hải quan tối thiểu (mã HS trên sản phẩm, xuất xứ,
  giá trị khai báo) — `catalog` thêm `hs_code`, `origin_country`.

### API, quyền, lỗi

- `GET/POST /api/v1/ledger/fx-rates`, `GET /public/markets` (thị trường của website), tham số `market` / `currency`
  trên API công khai của `catalog` và giỏ hàng.
- Permission: `ledger.fx.manage`, `catalog.markets.manage`.
- Lỗi: `ledger.fx_rate_missing` (409 — chưa có tỉ giá hiệu lực), `orders.currency_not_allowed` (422).

## Test bắt buộc (cả hai database)

`quy_doi_khong_dung_so_thuc`, `don_chot_ti_gia_luc_dat`, `chenh_lech_ti_gia_ghi_dung_tai_khoan`,
`nguoi_mua_trong_nuoc_chi_vnd`, `xuat_khau_thue_0_khi_du_ho_so`, `tien_te_khong_co_so_le_lam_tron_dung`.

## Căn cứ pháp lý

| Nội dung | Văn bản | Ghi chú |
|---|---|---|
| Hạn chế niêm yết, thanh toán bằng ngoại tệ trên lãnh thổ Việt Nam | Pháp lệnh Ngoại hối 2005 sửa 2013; Thông tư 32/2013/TT-NHNN (đã sửa đổi) | **Cần đối chiếu văn bản gốc** trước khi bật |
| Thuế GTGT 0% cho hàng xuất khẩu | Luật Thuế GTGT 48/2024/QH15 và văn bản hướng dẫn | Cần đối chiếu điều kiện hồ sơ |
| Kế toán ngoại tệ, chênh lệch tỉ giá | Luật Kế toán 2015; thông tư chế độ kế toán doanh nghiệp | Cần người có chuyên môn kế toán xác nhận |

## Agent làm bước này tự quyết

Quy tắc làm tròn; nguồn tỉ giá mặc định; danh sách tiền tệ bật sẵn. Ghi vào DECISIONS.
