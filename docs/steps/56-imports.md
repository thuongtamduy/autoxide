# Bước 56 — Chuyển dữ liệu từ nền tảng khác (`imports`)

> Lộ trình: `docs/ROADMAP.md` bước 56 · Luật chung: `docs/steps/README.md` · Quyết định: D-219, D-191,
> D-186 (pháp luật), D-193 (địa chỉ)

## Mục tiêu

Người bán đang dùng Shopify, WooCommerce, Haravan, Sapo hay KiotViet chuyển sang Autoxide bằng file xuất
của nền tảng cũ, mang theo:

- sản phẩm, biến thể, giá, tồn kho, ảnh;
- danh mục;
- khách hàng;
- lịch sử đơn hàng;
- đường dẫn cũ, chuyển hướng 301 để không mất thứ hạng tìm kiếm.

Chạy thử trước, chạy lại không trùng, không vi phạm luật dữ liệu cá nhân. Đây là việc đầu tiên mọi khách
chuyển nhà làm: nó quyết định họ có ở lại không.

## Bắt đầu khi

Bước 47 xong (`customers`). Trước đó đã có `catalog`, `inventory`, `orders`; `orders` có `OrderIntake::import`
cho đơn từ ngoài (bước 45, 48). Cần thêm `autoxide_kernel::net` (chặn SSRF, bước 40) và chuyển hướng của
`content`.

## Phạm vi

- **Module mới `imports`** (crate `modules/imports`, bảng `imports_…`, permission `imports.*`, lỗi
  `imports.*`). Đăng ký sau mọi module thương mại, trước `reporting`.
- **Sửa module khác:** chỉ thêm hàm nhập theo lô vào `api` khi chưa có, có test:
  - `catalog::api` — tạo / cập nhật sản phẩm theo mã nguồn;
  - `inventory::api` — đặt tồn ban đầu kèm lý do `import`;
  - `customers::api` — tạo hồ sơ khách.
- **Ngoài phạm vi:**
  - nhập trực tiếp qua API của nền tảng cũ. Cần tài khoản đối tác, nên làm sau như bộ nối (D-191);
  - nhập mật khẩu cũ;
  - nhập thẻ quà tặng và số dư (tắt mặc định, bước 47);
  - nhập điểm thưởng. Để làm tay, có báo cáo số dư cũ.

## Thiết kế

### Luồng

1. **Tạo lượt nhập:** chọn nền tảng nguồn, loại dữ liệu (sản phẩm, khách hàng, đơn hàng, danh mục, đường
   dẫn), tải file lên. File lưu qua `Storage` dưới tiền tố của khách (bước 33), hạn xoá sau khi xong.
2. **Nhận diện:**
   - bảng mã: UTF-8, UTF-8 BOM, Windows-1258 (Excel tiếng Việt cũ hay gặp);
   - dấu phân cách `,`, `;`, tab; hay file `.xlsx`.
3. **Ánh xạ:** áp hồ sơ ánh xạ mặc định của nền tảng. Người dùng sửa được ánh xạ từng cột trước khi chạy.
4. **Chạy thử** (`dry_run`): đọc hết file, kiểm từng dòng qua đúng hàm kiểm của module đích, không ghi gì.
   Trả số dòng sẽ tạo / cập nhật / bỏ qua và lỗi theo dòng + cột.
5. **Chạy thật:**
   - job theo lô (vd 200 dòng);
   - mỗi lô gọi `api` của module đích; ghi bảng ánh xạ ID trong cùng lượt;
   - tạm dừng / chạy tiếp được; lô hỏng ghi lỗi, các lô khác vẫn chạy.
6. **Sau khi nhập:**
   - tải ảnh (job riêng, qua `autoxide_kernel::net`, giới hạn cỡ, vào `media`);
   - tạo chuyển hướng 301;
   - báo cáo;
   - xoá file tải lên.

### Hồ sơ ánh xạ là dữ liệu

Mỗi nền tảng một tệp dữ liệu có phiên bản trong crate (`profiles/<nguồn>-<loại>-v<N>.json`):

| Trường | Nội dung |
|---|---|
| `source` | tên nền tảng |
| `kind` | loại dữ liệu |
| `version` | phiên bản hồ sơ |
| `columns` | tên cột nguồn → trường đích, kèm phép đổi: `trim`, `money_vnd`, `bool_yes_no`, `split:"|"`, `html_to_text`… |
| `group_by` | cột gom dòng thành một sản phẩm, vd `Handle` của Shopify, `Parent` của WooCommerce |
| `sample` | tên file mẫu dùng trong test |

Định dạng xuất của nền tảng đổi thì thêm hồ sơ phiên bản mới, không sửa code. Người làm phải:
- lấy file xuất **thật** (tạo cửa hàng dùng thử của từng nền tảng hay dùng tài liệu chính thức của họ);
- ghi nguồn và ngày vào hồ sơ;
- không đoán tên cột theo trí nhớ.

Gợi ý cột phải đối chiếu lại:
- **Shopify, sản phẩm:** `Handle`, `Title`, `Body (HTML)`, `Vendor`, `Type`, `Tags`, `Option1 Name`,
  `Option1 Value`, `Variant SKU`, `Variant Price`, `Variant Compare At Price`, `Variant Inventory Qty`,
  `Image Src`, `SEO Title`, `SEO Description`.
- **WooCommerce:** `ID`, `Type`, `SKU`, `Name`, `Parent`, `Regular price`, `Sale price`, `Categories`,
  `Images`, `Attribute 1 name`, `Attribute 1 value(s)`.
- **Haravan:** định dạng gần với Shopify.
- **Sapo, KiotViet:** file Excel có tên cột tiếng Việt.

Lượt nhập dùng job API chung của bước 51 (danh sách job, tiến độ, file kết quả, chạy lại — D-212):
không tự dựng màn theo dõi job riêng.

### Bảng dữ liệu

Mọi bảng có `tenant_id`; unique và index bắt đầu bằng nó.

| Bảng | Cột chính | Khoá / index |
|---|---|---|
| `imports_runs` | `tenant_id`, `id`, `source`, `kind`, `profile_version`, `mapping` text (JSON, ánh xạ đã chỉnh), `file_key`, `status` (`uploaded` / `previewed` / `running` / `paused` / `finished` / `failed` / `cancelled`), `totals` text (JSON: tạo, cập nhật, bỏ qua, lỗi), `consent_basis` text null (khách hàng), `created_by`, thời điểm | index (`tenant_id`, `status`) |
| `imports_problems` | `tenant_id`, `run_id`, `row_number`, `column`, `code`, `message` | index (`tenant_id`, `run_id`, `row_number`) |
| `imports_mappings` | `tenant_id`, `source`, `kind`, `source_id`, `target_id`, `run_id`, `created_at` | unique (`tenant_id`, `source`, `kind`, `source_id`) |
| `imports_paths` | `tenant_id`, `run_id`, `old_path`, `target_kind`, `target_id`, `status` (`redirected` / `unmapped`) | index (`tenant_id`, `run_id`) |

Chạy lại cùng file:
- `imports_mappings` làm dòng đã nhập thành "cập nhật" thay vì tạo bản thứ hai;
- unique giữ đúng điều đó khi hai lô cùng ghi.

### API REST

Permission:
- `imports.run`: tạo, chạy thử, chạy, dừng, huỷ;
- `imports.read`: xem lượt và báo cáo;
- nhập khách hàng cần thêm `imports.customers`, vì có dữ liệu cá nhân.

Phạm vi cửa hàng theo bước 34.

| Method + path | Việc |
|---|---|
| `GET /api/v1/imports/profiles` | Danh sách nền tảng / loại / hồ sơ có sẵn và cột mong đợi |
| `POST /api/v1/imports/runs` (multipart: `source`, `kind`, file) | Tạo lượt; trả cột tìm thấy và ánh xạ đề xuất. Giới hạn cỡ file theo cấu hình (`Setting`) và hạn mức gói (bước 35) |
| `PUT /api/v1/imports/runs/{id}/mapping` | Sửa ánh xạ (`If-Match`) |
| `POST /api/v1/imports/runs/{id}/preview` | Chạy thử. File lớn thì chạy bằng job, trả 202 + trạng thái |
| `POST /api/v1/imports/runs/{id}/start` · `/pause` · `/resume` · `/cancel` | Điều khiển. `start` cần `Idempotency-Key`; khách hàng cần `consent_basis` trong thân |
| `GET /api/v1/imports/runs`, `GET /api/v1/imports/runs/{id}` | Trạng thái, tổng số |
| `GET /api/v1/imports/runs/{id}/problems` (phân trang), `GET …/problems.csv` | Lỗi theo dòng. CSV qua bộ chống CSV injection của kernel (bước 41) |
| `GET /api/v1/imports/runs/{id}/paths` | Đường dẫn cũ: đã chuyển hướng / không ánh xạ được |

### Tool MCP · Event · Job · Cấu hình · Mã lỗi

- **Tool MCP:** `imports.list_profiles`, `imports.preview`, `imports.status`. Không có tool `start`: chạy
  thật ghi dữ liệu hàng loạt nên con người bấm, như luật AI của D-109.
- **Event:** `imports.run_finished` (tổng số), `imports.run_failed`.
- **Job:**
  - `imports.preview`, `imports.batch` (một lô), `imports.fetch_images`, `imports.redirects`,
    `imports.cleanup` (xoá file);
  - `unique_key` gồm `run_id` + số lô (README).
- **Cấu hình:** `AUTOXIDE_IMPORTS_MAX_FILE_MB` (mặc định 50), `AUTOXIDE_IMPORTS_MAX_IMAGE_MB`
  (mặc định 10). Sinh lại `.env.example`.
- **Mã lỗi:**

  | Mã | Câu tiếng Việt |
  |---|---|
  | `imports.unknown_source` | — |
  | `imports.file_too_large` | — |
  | `imports.unreadable_file` | "Không đọc được file: kiểm bảng mã và định dạng." |
  | `imports.missing_column` | "Thiếu cột `{column}` mà hồ sơ `{profile}` cần." |
  | `imports.mapping_invalid` | — |
  | `imports.run_not_found` | — |
  | `imports.invalid_state` | — |
  | `imports.consent_basis_required` | "Nhập khách hàng cần ghi căn cứ xử lý dữ liệu cá nhân." |

  Lỗi theo dòng dùng chính mã lỗi của module đích (vd `catalog.sku_taken`), để người dùng thấy cùng câu
  như khi tạo tay.

## Thứ tự làm

1. Crate `imports`, bảng, tạo lượt + đọc file:
   - CSV qua `autoxide_kernel::csv`;
   - `.xlsx` qua thư viện đọc Excel: chọn sau khi kiểm (vd `calamine`), ghi `STACK.md`;
   - nhận diện bảng mã;
   - ánh xạ.

   → mục 1 của ROADMAP.
2. Chạy thử: gọi hàm kiểm của module đích. Nếu module chưa có hàm kiểm không ghi, thêm vào `api` của module
   đó, vd `Catalog::validate_import`. → mục 2.
3. Chạy thật theo lô; bảng ánh xạ ID; dừng / chạy tiếp; ảnh. → mục 2 và 3.
4. Hồ sơ sản phẩm + danh mục cho Shopify, WooCommerce, Haravan, Sapo, KiotViet, mỗi hồ sơ có file mẫu thật
   trong `tests/fixtures`. → mục 1.
5. Đường dẫn cũ → chuyển hướng 301 của `content` (đích là đường dẫn mới của sản phẩm / danh mục / bài).
   → mục 4.
6. Khách hàng: căn cứ xử lý, không mang đồng ý quảng cáo, không mật khẩu. Đơn hàng lịch sử qua
   `OrderIntake::import` với cờ `historical`: không trừ kho, không gửi thông báo, không ghi sổ cái tiền đã
   thu (chỉ thống kê). → mục 5.
7. Báo cáo, tool MCP, hướng dẫn chuyển từ từng nền tảng (EN + VI), CHANGELOG. → mục 6.

## Test bắt buộc

Cả SQLite và Postgres.

- `chay_thu_khong_ghi_gi`: so số dòng mọi bảng đích trước và sau.
- `chay_lai_cung_file_khong_tao_trung`: chạy hai lần → lần hai toàn "cập nhật".
- `hai_lo_dong_thoi_cung_ma_nguon`: hai job cùng ghi một `source_id` → một sản phẩm.
- `dung_va_chay_tiep_khong_mat_dong`: tổng tạo + bỏ qua + lỗi = số dòng.
- `file_mau_tung_nen_tang_doc_dung`: mỗi hồ sơ một file mẫu thật, kiểm số sản phẩm, biến thể, giá (đơn vị
  nhỏ nhất, không số thực), ảnh.
- `bang_ma_windows_1258_doc_dung_dau_tieng_viet`.
- `anh_tro_toi_dia_chi_noi_bo_bi_chan`: `http://127.0.0.1/…`, `http://169.254.169.254/…` → lỗi dòng,
  không tải.
- `nhap_khach_hang_khong_mang_dong_y_quang_cao`; `nhap_khach_hang_thieu_can_cu_bi_tu_choi`.
- `don_lich_su_khong_tru_kho_khong_gui_thong_bao`.
- `duong_dan_cu_thanh_chuyen_huong_301`, và đường dẫn trùng đường dẫn đang có nội dung thì báo `unmapped`
  (chuyển hướng của `content` đã chặn trường hợp này).
- `ro_ri_giua_khach`: bài chung của testkit cho `imports_*`.

## Tiêu chí xong

- [ ] Mọi mục của bước 56 trong ROADMAP `[x]`.
- [ ] `./gate.sh` đầy đủ xanh.
- [ ] `docs/openapi.json` sinh lại.
- [ ] `.env.example` sinh lại.
- [ ] Bản dịch mã lỗi.
- [ ] Hướng dẫn EN + VI cho từng nền tảng: cách xuất file ở nền tảng cũ, cách nhập, việc còn phải làm tay.
- [ ] CHANGELOG.
- [ ] Quyết định (thư viện Excel, các hồ sơ và nguồn của chúng) ghi DECISIONS.
- [ ] Phát hành theo `docs/RELEASING.md`.

## Chỗ dễ sai

- **Tiền:**
  - file nguồn có `100.000`, `100,000`, `100000.00`, `1e5`;
  - phép đổi `money_vnd` phải từ chối giá trị mơ hồ thay vì đoán;
  - không bao giờ qua `f64`.
- **Một sản phẩm nhiều dòng** (Shopify: mỗi biến thể / ảnh một dòng, cùng `Handle`). Gom theo `group_by`
  trước khi gọi `catalog`; lô không được cắt giữa một nhóm.
- **HTML mô tả từ nền tảng cũ:** lọc qua bộ lọc HTML có sẵn (bước 32), bỏ `script` / `style` / thuộc tính
  sự kiện. Ảnh nhúng trong mô tả tải về như ảnh sản phẩm.
- **Transaction:** tải ảnh là gọi mạng nên không đặt trong transaction ghi (luật dữ liệu). Ghi sản phẩm
  trước, ảnh gắn sau bằng job.
- **Nhập lớn làm nghẽn khách khác:** job chia lượt công bằng giữa các khách (bước 33); lô nhỏ; không giữ
  transaction dài.
- **Mã SKU trùng trong file nguồn:** báo lỗi dòng, không ghi đè lẫn nhau.

## Căn cứ pháp lý

- **Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 và NĐ 356/2025** (từ 01/01/2026): nhập dữ liệu khách hàng là
  xử lý dữ liệu cá nhân, phải có căn cứ. Người nhập ghi căn cứ, lưu ở `imports_runs.consent_basis` và sổ
  căn cứ của bước 37.
  - Đồng ý nhận quảng cáo không tự chuyển sang nếu không có bằng chứng (NĐ 91/2020 về tin nhắn, thư điện
    tử quảng cáo).
  - Cần đối chiếu văn bản gốc.
- **Mật khẩu không nhập:** băm của nền tảng khác không dùng được và không nên mang theo. Khách đặt lại qua
  link (bước 34, đăng nhập bằng link).
- **File tải lên chứa dữ liệu cá nhân:** xoá sau khi xong (`imports.cleanup`), không vào backup lâu hơn
  thời hạn lưu (bước 37, 39).

## Agent làm bước này tự quyết

- **Thư viện đọc Excel:** khuyến nghị `calamine` nếu kiểm thấy hợp. Ghi `STACK.md` và DECISIONS.
- **Nền tảng nào trước:** khuyến nghị theo thị phần ở Việt Nam — Haravan, Sapo, KiotViet, rồi Shopify,
  WooCommerce.
- **Có nhập đánh giá sản phẩm cũ không:** chỉ khi giữ được tên người viết và ngày, và ghi rõ "nhập từ
  <nền tảng>". Không được làm như đánh giá mới (Luật 19/2023: đánh giá phải trung thực).

## Bổ sung theo D-220

- Nguồn thêm **`autoxide`**: CSV cùng cột với xuất chung của bước 51 (P16), để chuyển dữ liệu giữa hai khách
  hay sửa hàng loạt trên bảng tính rồi nhập lại.
- Đích thêm ngoài thương mại: `content` (bài, trang, kèm chuyển hướng 301), `media` (tải từ URL qua
  `autoxide_kernel::net`, kiểm loại thật như tải lên), `newsletter` (người đăng ký chỉ khi có bằng chứng đồng ý
  đầy đủ). Phần này làm được khi bước 51 P16 đã có, không chờ bước 47.
- Đây là cách nhập duy nhất của dự án (bước 51 P17 trỏ về đây); không module nào tự làm nhập riêng.
