# Bước 63 — Đa tiền tệ và bán xuyên biên giới

> Lộ trình: `docs/ROADMAP.md` bước 63 · Quyết định: D-221, D-233 · Đặc tả liên quan: 36 (tiền, sổ cái, thuế,
> `PayoutGateway`), 37 (KYC, gói luật), 41 (giá), 44 (vận chuyển quốc tế), 45 (đơn), 46 (sàn, người bán)

## Mục tiêu

Cửa hàng bán cho người mua ở nước khác: hiện giá và thu tiền bằng tiền tệ của người mua, quy đổi về tiền tệ
ghi sổ đúng tỉ giá có ngày hiệu lực, thuế xuất khẩu, phí giao quốc tế. **Không** làm mua bán bằng ngoại tệ trong
lãnh thổ Việt Nam khi luật không cho phép.

Thêm (D-233): **người bán nước ngoài trên sàn** — tắt mặc định; điều kiện (giấy tờ, hiện diện tại Việt Nam, cách
xử lý thuế) là gói luật có ngày hiệu lực, chưa có gói thì không bật được; người mua thấy quốc gia người bán, nơi
gửi, thời gian giao, cách đổi trả; người bán nhận chi trả bằng ngoại tệ. Hàng gửi từ nước ngoài kèm thủ tục hải
quan, thuế nhập khẩu cho người mua **không** làm.

## Bắt đầu khi

Bước 36, 41, 44, 45 xong. Không cần module mới: mở rộng `ledger`, `tax`, `catalog`, `orders`, `shipping` và kernel.
Phần người bán nước ngoài cần thêm 37 (`compliance`) và 46 (`marketplace`); mở rộng `marketplace`, gói luật của
`compliance` và `tax` (D-233).

## Thiết kế

- **Tiền đã đúng từ đầu:** mọi số tiền là số nguyên đơn vị nhỏ nhất + mã ISO 4217 (`autoxide_kernel::money`); số
  chữ số thập phân theo bảng ISO (VND 0, USD 2, JPY 0, KWD 3) — kiểm lại khi làm.
- **Tỉ giá** (`ledger_fx_rates`: cặp tiền, tỉ giá dạng phân số nguyên `numerator/denominator`, nguồn, `effective_at`):
  nhập tay hay qua trait `ExchangeRateSource` (bộ nối Vietcombank / ngân hàng / dịch vụ tỉ giá là module riêng; bản giả
  cho test). Không dùng số thực; làm tròn theo quy tắc ghi trong DECISIONS (nửa lên, ở bước cuối). Trait `ExchangeRateSource` là trait của kernel ở đặc tả 36; 36 hiện đặt bảng tỉ giá ở `invoices_exchange_rates`. Khi làm bước này chốt **một** chỗ — khuyến nghị dời về `ledger` vì `ledger` đăng ký trước `invoices` và chi trả cần tỉ giá — và ghi quyết định (D-233).
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

### Người bán nước ngoài trên sàn (D-233)

**Tắt mặc định, chỉ bật khi có gói luật.** Cài đặt của sàn `marketplace.foreign_sellers` = cột
`marketplace_settings.foreign_sellers` (mặc định `false`). `PUT /api/v1/marketplace/settings/foreign-sellers
{enabled, countries?}` bật được chỉ khi `ComplianceRules::effective(tenant, "marketplace.foreign_sellers", now)`
có bản cho vùng pháp lý của khách; không có → `marketplace.foreign_sellers_rules_missing`. Gói `vn` đóng trong
crate `compliance` **không** có khoá này (nội dung chưa được người có chuyên môn pháp lý xác nhận): nhân sự nền
tảng nhập bản đã đối chiếu bằng `POST /api/v1/compliance/rules` (37); tệp mẫu chỉ nằm trong test. `countries` là
giới hạn thêm của sàn, nằm trong danh sách bản quy tắc cho phép. Sàn tắt lại, hay bản quy tắc hết hiệu lực mà không
có bản kế (job hằng ngày `marketplace.foreign_sellers_review`, chạy thêm khi nghe `compliance.rules_imported`) →
người bán `foreign` đang `active` chuyển `suspended` (lý do `foreign_sellers_disabled` / `foreign_rules_expired`),
báo người bán; quyết toán đã có vẫn giữ và chi như thường.

**Người bán.** `marketplace_sellers` thêm `residency` (`domestic` mặc định / `foreign`) và quốc gia
(`country_code`, ISO 3166-1 alpha-2), khai khi đăng ký (`POST /sellers`, `PUT /sellers/me`). `foreign` chỉ nhận khi
sàn đã bật (không thì `marketplace.foreign_sellers_disabled`) và quốc gia được phép
(`marketplace.seller_country_not_allowed`). Đổi `residency` / quốc gia sau khi `active` → về `pending_review`.
Người bán trong nước không đổi gì.

**Điều kiện là dữ liệu có ngày hiệu lực.** Thân bản quy tắc `marketplace.foreign_sellers` (gói luật của 37; mọi giá
trị dưới đây chỉ là ví dụ — **cần đối chiếu** Luật TMĐT 122/2025, NĐ 85/2021, NĐ 117/2025, Thông tư 80/2021):

```json
{ "countries": "*",
  "seller_types": { "enterprise": { "documents": ["foreign_business_registration"],
                                    "local_presence": ["representative", "legal_entity"],
                                    "tax_mode": "seller_self_registers" },
                    "individual": { "documents": ["passport"], "local_presence": ["none"],
                                    "tax_mode": "platform_withholds" } },
  "presence_documents": { "representative": ["vn_representative_appointment"], "legal_entity": [] },
  "ships_from": ["vn", "abroad_seller_handled"],
  "vn_invoice_available": false,
  "legal_basis": "…" }
```

`SellerGate::ensure_can_sell` với người bán `foreign` kiểm thêm, theo bản hiệu lực lúc `now`: sàn đang bật; quốc
gia được phép; `seller_type` có trong bản; mọi giấy tờ `documents` + `presence_documents[local_presence]`
`approved`, chưa hết hạn (`marketplace_seller_documents`, `category_id` NULL); `local_presence` thuộc danh sách
cho phép, có thông tin đại diện / pháp nhân tại Việt Nam khi cần; `tax_mode` đủ dữ liệu (xem "Thuế"); `ships_from`
thuộc danh sách; đã khai chính sách đổi trả. `POST /sellers/me/submit` thiếu →
`marketplace.foreign_seller_requirements` (422, liệt kê **mọi** thứ thiếu một lần, D-083); checkout vẫn là
`marketplace.seller_not_active` như người bán trong nước. Bản quy tắc mới đòi thêm → job
`marketplace.foreign_sellers_review` chuyển `suspended` kèm danh sách thiếu. `marketplace_sellers.rules_version` ghi
bản đã kiểm, để biết người bán được duyệt theo bản nào.
`GET /foreign-sellers/requirements?country=&seller_type=` trả điều kiện hiện hành cho người bán chuẩn bị hồ sơ.

**KYC.** Qua cổng của 37 như người bán trong nước: `KycGate::require_verified(tenant, KycSubject::Org(seller),
Activity::Sell)`; chưa `verified` thì không đăng bán, không nhận đơn. `compliance` nhận thêm loại hồ sơ nước ngoài
(`document_kind`): `foreign_business_registration` (doanh nghiệp, cách xác thực `BusinessRegistry` khi có nhà cung
cấp tra sổ đăng ký nước ngoài, không thì `Manual` — nhân sự sàn duyệt bản quét, xoá theo hạn của 37) và `passport`
(cá nhân, `Ekyc` / `Manual`). Danh sách loại hồ sơ cho phép là dữ liệu của gói luật, không viết cứng.

**Thuế.** `tax_mode` lấy từ bản quy tắc theo `seller_type`:
- `platform_withholds` — sàn khấu trừ, nộp thay như với hộ / cá nhân trong nước: `TaxEngine::withholding` với
  `payee_regime` `foreign_individual` / `foreign_organization` (dòng mới trong `tax_withholding_rates` của gói `vn`
  của `tax`, cần đối chiếu). Không có tỉ lệ hiệu lực → điều kiện thiếu `tax_rate_missing`, không đoán tỉ lệ.
- `seller_self_registers` — người bán tự đăng ký, khai, nộp qua cổng thông tin điện tử dành cho nhà cung cấp nước
  ngoài (Thông tư 80/2021) và khai `foreign_tax_id`; sàn không khấu trừ.

Cả hai trường hợp, báo cáo thuế theo kỳ của 46 có dòng người bán kèm quốc gia, `tax_mode`, `foreign_tax_id`.
`vn_invoice_available = false` → trang sản phẩm và checkout báo người mua không nhận được hoá đơn GTGT Việt Nam
cho phần hàng của người bán đó.

**Công bố cho người mua (Luật 19/2023).** `PublicSellerProfile` (46) thêm `residency`, `country_code`,
`ships_from_country`, `delivery_days {min, max}`, `return_policy {mode, address?, summary}`, `vn_invoice_available`;
trang sản phẩm công khai (qua `SellerDisclosure`), `GET /public/sellers/{slug}` và bước xem lại ở checkout đều
hiện. Đổi trả: `return_mode = vn_address` (địa chỉ nhận hàng trả tại Việt Nam, `Address`) hay `international` (quy
trình trả hàng quốc tế: ai trả phí gửi, thời hạn — người bán khai, sàn duyệt). Fact
`marketplace.seller_info_on_product` (37) với người bán `foreign` chỉ đúng khi đủ các trường này.

**Nơi gửi hàng.** `ships_from`: `vn` — hàng có sẵn tại Việt Nam (kho của người bán tại Việt Nam hay kho của sàn,
`fulfillment = platform`), giao như người bán trong nước; `abroad_seller_handled` — người bán tự lo giao quốc tế và
thủ tục, phương thức giao là `self` (người bán nhập mã vận đơn), trang sản phẩm ghi rõ "gửi từ {quốc gia}", thời
gian giao, và người mua có thể phải nộp thuế, phí nhập khẩu theo luật. **Ngoài phạm vi:** tính thuế nhập khẩu,
khai hải quan, làm thủ tục thay người mua — kéo theo Luật Hải quan, luật thuế xuất nhập khẩu và quy định về hàng giá
trị nhỏ gửi qua chuyển phát nhanh, cần đối chiếu riêng; core không có dữ liệu để làm đúng (D-233).

**Tiền.** Người mua trong nước trả **VND** (luật "Chặn trái luật" ở trên áp nguyên); quyết toán của 46 ghi VND.
Chi trả: `marketplace_sellers.payout_currency` (mặc định `VND`), chọn trong tiền tệ cổng chi trả hỗ trợ (không thì
`marketplace.payout_currency_unsupported`). Kernel (36) thêm cho `PayoutGateway`, đều có cài đặt / giá trị mặc
định để bộ nối cũ không phải sửa (D-038):

```rust
// autoxide_kernel::payment (phác thảo — tra API thật bằng skill `check-library-api`)
pub struct PayoutCapabilities { pub currencies: Vec<Currency> /* mặc định [VND] */, pub cross_border: bool }
// trong trait PayoutGateway:
fn capabilities(&self) -> PayoutCapabilities { PayoutCapabilities::domestic_vnd() }
// PayoutIntent thêm: beneficiary_country: String /* ISO 3166-1 alpha-2 */, base_amount: Money /* VND trừ khỏi seller_payable */,
//                    fx_rate_id: Option<Uuid>
```

`marketplace.build_payouts`: tổng VND của người bán → quy đổi sang `payout_currency` bằng tỉ giá hiệu lực của
`ledger` (`ledger_fx_rates`, ngày dựng lệnh chi; phân số nguyên, làm tròn nửa lên ở bước cuối), lưu ảnh chụp tỉ
giá trên `marketplace_payouts`; gửi lại cùng `reference` dùng đúng số đã chụp, không quy đổi lại. Chưa có tỉ giá
→ bỏ qua người bán lần này, quyết toán vẫn `releasable`, cảnh báo (`ledger.fx_rate_missing` trong log, số đo
`marketplace_payouts_skipped_total{reason}`). Khi `paid`: Nợ `seller_payable` (VND gốc) / Có `partner_collection`
(VND); đối tác báo số VND thực trừ khác số đã ghi → chênh lệch vào `fx.gain_loss` (36). Tiền vẫn chỉ đi qua đối
tác có giấy phép chuyển tiền ra nước ngoài (D-187); `cross_border = false` mà người bán ở nước ngoài → không chi.

| Bảng (migration **mới** của `marketplace`, có `down`) | Cột thêm | Khoá / index |
|---|---|---|
| `marketplace_settings` | `foreign_sellers` BOOL (`false`), `foreign_seller_countries` JSON NULL | |
| `marketplace_sellers` | `residency` (`domestic`/`foreign`, mặc định `domestic`), `country_code` (`VN`), `ships_from` (`vn`/`abroad_seller_handled`, mặc định `vn`), `ships_from_country` (`VN`), `delivery_days_min` NULL, `delivery_days_max` NULL, `local_presence` (`none`/`representative`/`legal_entity`, mặc định `none`), `local_representative` JSON NULL (tên, `Address` tại Việt Nam, liên hệ), `tax_mode` (`domestic`/`platform_withholds`/`seller_self_registers`, mặc định `domestic`), `foreign_tax_id` NULL, `return_mode` (`vn_address`/`international`) NULL, `return_address` JSON NULL, `return_policy_summary` NULL, `payout_currency` (`VND`), `rules_version` NULL | index (`tenant_id`,`residency`,`status`) |
| `marketplace_payouts` | `payout_amount_minor`, `payout_currency`, `fx_rate_id` NULL, `fx_rate_numerator` NULL, `fx_rate_denominator` NULL, `settled_base_minor` NULL (VND thực trừ đối tác báo) | |
| `marketplace_seller_documents.doc_type` | thêm mã `foreign_business_registration`, `passport`, `vn_representative_appointment` (danh mục giấy tờ là dữ liệu) | |

### API, quyền, lỗi

- `GET/POST /api/v1/ledger/fx-rates`, `GET /public/markets` (thị trường của website), tham số `market` / `currency`
  trên API công khai của `catalog` và giỏ hàng.
- Permission: `ledger.fx.manage`, `catalog.markets.manage`.
- Lỗi: `ledger.fx_rate_missing` (409 — chưa có tỉ giá hiệu lực), `orders.currency_not_allowed` (422).

**Người bán nước ngoài** (`/api/v1/marketplace`, D-233):

| Method + path | Permission | Ghi chú |
|---|---|---|
| `PUT /settings/foreign-sellers` | `marketplace.foreign_sellers.manage` (nhân sự sàn) | `{enabled, countries?}`; bật cần bản quy tắc hiệu lực; `If-Match` |
| `GET /foreign-sellers/requirements?country=&seller_type=` | `marketplace.sell` / `marketplace.sellers.read` | điều kiện hiện hành, kèm `rules_version`, `legal_basis` |
| `POST /sellers`, `PUT /sellers/me` | `marketplace.sell` (sẵn có) | thêm `residency`, `country_code`, `ships_from`, đại diện, `tax_mode`, đổi trả, `payout_currency` |
| `GET /sellers?residency=` | `marketplace.sellers.read` (sẵn có) | lọc người bán nước ngoài |
| `GET /public/sellers/{slug}` | công khai (sẵn có) | thêm các trường công bố |

- Tool MCP (cùng service, cùng permission): `marketplace.foreign_seller_requirements` (`read_only`); `get_seller`,
  `decide_seller` sẵn có trả / kiểm thêm trường mới. Không có tool bật tính năng.
- Event: `marketplace.foreign_sellers_toggled` (`enabled`, `rules_version`); người bán bị tạm khoá vì luật dùng
  `marketplace.seller_status_changed` sẵn có với lý do mới. Nghe thêm `compliance.rules_imported`.
- Job: `marketplace.foreign_sellers_review` (hằng ngày; `unique_key` `marketplace.foreign_sellers_review:<ngày>`).
- Cấu hình: không có biến môi trường mới — cài đặt là dữ liệu theo khách (`marketplace_settings`).
- Permission: `marketplace.foreign_sellers.manage`.

| Mã | HTTP | Tiếng Việt |
|---|---|---|
| `marketplace.foreign_sellers_disabled` | 409 | Sàn chưa nhận người bán nước ngoài. |
| `marketplace.foreign_sellers_rules_missing` | 409 | Chưa có quy định hiệu lực về người bán nước ngoài cho {jurisdiction}; cần nhập gói luật trước khi bật. |
| `marketplace.seller_country_not_allowed` | 422 | Sàn chưa nhận người bán từ {country}. |
| `marketplace.foreign_seller_requirements` | 422 | Người bán nước ngoài còn thiếu: {missing}. |
| `marketplace.payout_currency_unsupported` | 422 | Cổng chi trả chưa hỗ trợ chi bằng {currency}. |

## Thứ tự làm

1. Tỉ giá (`ledger_fx_rates`, nguồn tỉ giá), quy đổi bằng phân số nguyên, làm tròn theo bảng ISO.
2. Thị trường, giá theo thị trường, chốt tỉ giá vào đơn, chặn ngoại tệ với người mua trong nước.
3. Sổ cái ngoại tệ và chênh lệch tỉ giá; thuế xuất khẩu; vận chuyển quốc tế (`hs_code`, `origin_country`).
4. Người bán nước ngoài (D-233): cài đặt sàn + kiểm gói luật; cột người bán (migration mới của `marketplace`);
   điều kiện trong `SellerGate`, job rà soát; loại hồ sơ nước ngoài ở `compliance`; `tax_mode` và tỉ lệ khấu trừ
   của `tax`; công bố trên trang sản phẩm, checkout; `PayoutCapabilities` ở kernel + bộ giả đa tiền tệ trong
   testkit, chi trả ngoại tệ.

## Test bắt buộc (cả hai database)

`quy_doi_khong_dung_so_thuc`, `don_chot_ti_gia_luc_dat`, `chenh_lech_ti_gia_ghi_dung_tai_khoan`,
`nguoi_mua_trong_nuoc_chi_vnd`, `xuat_khau_thue_0_khi_du_ho_so`, `tien_te_khong_co_so_le_lam_tron_dung`.

Người bán nước ngoài (D-233): `nguoi_ban_nuoc_ngoai_tat_mac_dinh` (đăng ký `foreign` khi sàn chưa bật →
`foreign_sellers_disabled`), `thieu_goi_luat_khong_bat_duoc` (không có bản quy tắc, hay bản đã hết hiệu lực →
`foreign_sellers_rules_missing`), `nguoi_ban_nuoc_ngoai_chua_du_ho_so_khong_dang_ban` (KYC chưa `verified`, thiếu
giấy tờ / đại diện / `tax_mode`: sản phẩm không lên `active`, checkout `seller_not_active`, `submit` liệt kê mọi
thứ thiếu), `trang_san_pham_co_quoc_gia_nguoi_ban` (quốc gia, nơi gửi, thời gian giao, đổi trả, hoá đơn),
`chi_tra_dung_tien_te` (lệnh chi bằng `payout_currency` theo tỉ giá của `ledger`, bút toán VND khớp, chênh lệch vào
`fx.gain_loss`, gửi lại không quy đổi lại), `cong_chi_tra_khong_ho_tro_tien_te_bi_tu_choi`,
`goi_luat_het_hieu_luc_tam_khoa_nguoi_ban_nuoc_ngoai`, `nguoi_mua_trong_nuoc_tra_vnd_cho_nguoi_ban_nuoc_ngoai`,
`khau_tru_thay_dung_ti_le_nguoi_ban_nuoc_ngoai`.

## Chỗ dễ sai

- Viết cứng điều kiện với người bán nước ngoài (giấy tờ, đại diện, tỉ lệ thuế) trong code — phải đọc bản quy tắc
  hiệu lực lúc kiểm (D-233).
- Đóng sẵn bản quy tắc chưa đối chiếu vào gói `vn` — sàn bật được khi luật chưa rõ (D-233).
- Hiện giá hay thu tiền bằng ngoại tệ của người mua trong nước chỉ vì người bán ở nước ngoài (D-233).
- Quy đổi chi trả bằng số thực, làm tròn từng bước, hay quy đổi lại khi gửi lại lệnh chi (đối tác chi số khác lần
  trước) (D-233).
- Gọi nguồn tỉ giá hay `PayoutGateway` trong transaction ghi (D-233).
- Người bán gửi từ nước ngoài mà trang sản phẩm không nói rõ nơi gửi, thời gian giao, khả năng phát sinh thuế phí
  nhập khẩu (D-233).

## Căn cứ pháp lý

| Nội dung | Văn bản | Ghi chú |
|---|---|---|
| Hạn chế niêm yết, thanh toán bằng ngoại tệ trên lãnh thổ Việt Nam | Pháp lệnh Ngoại hối 2005 sửa 2013; Thông tư 32/2013/TT-NHNN (đã sửa đổi) | **Cần đối chiếu văn bản gốc** trước khi bật |
| Thuế GTGT 0% cho hàng xuất khẩu | Luật Thuế GTGT 48/2024/QH15 và văn bản hướng dẫn | Cần đối chiếu điều kiện hồ sơ |
| Kế toán ngoại tệ, chênh lệch tỉ giá | Luật Kế toán 2015; thông tư chế độ kế toán doanh nghiệp | Cần người có chuyên môn kế toán xác nhận |
| Người bán nước ngoài: xác thực, hiện diện tại Việt Nam, nghĩa vụ của sàn | Luật Thương mại điện tử 122/2025/QH15 (từ 01/7/2026); NĐ 85/2021/NĐ-CP (thương nhân nước ngoài có website tiếng Việt / tên miền .vn / giao dịch tại Việt Nam) | **Cần đối chiếu văn bản gốc**; điều kiện là gói luật `marketplace.foreign_sellers` (D-233) |
| Thuế của người bán nước ngoài: sàn khấu trừ thay hay người bán tự đăng ký | NĐ 117/2025/NĐ-CP; Thông tư 80/2021/TT-BTC (nhà cung cấp nước ngoài đăng ký, khai, nộp qua cổng điện tử) | Cần đối chiếu; tỉ lệ là dữ liệu của `tax` (D-233) |
| Công bố thông tin người bán, đổi trả cho người tiêu dùng | Luật Bảo vệ quyền lợi người tiêu dùng 19/2023/QH15; NĐ 55/2024/NĐ-CP | Cần đối chiếu (D-233) |
| Chuyển tiền chi trả cho người bán ở nước ngoài | Pháp lệnh Ngoại hối 2005 (sửa 2013) — giao dịch vãng lai; NĐ 52/2024/NĐ-CP | Chỉ qua đối tác có giấy phép (D-187); cần đối chiếu (D-233) |
| Hàng gửi từ nước ngoài: hải quan, thuế nhập khẩu, hàng giá trị nhỏ | Luật Hải quan 2014; Luật Thuế xuất khẩu, thuế nhập khẩu 2016 và văn bản hướng dẫn | Ngoài phạm vi — cần đối chiếu riêng trước khi làm (D-233) |

## Agent làm bước này tự quyết

Quy tắc làm tròn; nguồn tỉ giá mặc định; danh sách tiền tệ bật sẵn. Với người bán nước ngoài (D-233): tỉ giá nào
cho chi trả (mua / bán / trung bình của nguồn); phí chuyển tiền quốc tế do ai chịu (khuyến nghị: phí của sàn theo
`marketplace_fee_rules`, hiện trên bảng đối soát); danh sách quốc gia mặc định khi sàn bật (khuyến nghị: theo bản
quy tắc). Ghi vào DECISIONS.
