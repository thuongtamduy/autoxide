# Dùng thử RC, vòng 1 (agent AI đóng vai người dùng)

[English](README.md) · **Tiếng Việt**

**Lưu ý quan trọng:** người dùng thử là agent AI đóng vai, **không phải người thật**. Vòng 1 chạy trên `main` ở bản 0.19.0
(trước `1.0.0-rc.1`), như một lần kiểm sớm (chủ dự án giao agent AI đóng vai người dùng trước khi có người ngoài, vài vòng). Mỗi vai chỉ được đọc tài liệu công khai
(README, `docs/guides`, `docs/*.md`, OpenAPI, `examples/`), không đọc mã nguồn `crates/` hay `modules/` (vai người viết
module được đọc thêm `modules/<tên>/src/api.rs`). Subagent: vai người mới dùng model nhỏ (mô phỏng người ít kinh nghiệm),
bốn vai còn lại dùng model cỡ vừa. Không dùng secret thật, nhà cung cấp thật hay model AI thật.

Giới hạn của môi trường ảnh hưởng kết quả (không phải lỗi sản phẩm): máy 4 lõi, **không có Docker daemon** (đường
quickstart bằng Docker **chưa thử**), nên mọi vai dùng binary với SQLite hoặc PostgreSQL 16 cục bộ. Binary dựng sẵn một
lần (`cargo build -p autoxide`, debug, **10 phút 06 giây**, cache nguội); các vai tính giờ từ lúc "đã có binary". Số đo
là đồng hồ của chính agent, gần đúng khi có ghi "ước".

## Số đo cho G9 (agent đóng vai, không phải người)

| Chỉ số | Kết quả | Cách hiểu |
| --- | --- | --- |
| Phút tới request đầu tiên có dữ liệu (đã có binary, đã viết `.env`) | **dưới 1 phút** (36 giây từ migrate tới request có dữ liệu) | **Không** tính đọc tài liệu, viết `.env`, dựng binary. Dựng từ mã nguồn mất 10 phút 06 giây (debug, nguội). Quickstart Docker chưa đo. Ngưỡng G9 "dưới 5 phút từ lúc tải" chưa kết luận được cho tới khi đo đường binary dựng sẵn hoặc image chạy được. |
| Phút tới module đầu tiên (sinh, dựng, gọi được endpoint) | **khoảng 33 phút**, trong đó 32 phút 08 giây là biên dịch lõi vào thư mục target riêng của module | Bị chậm vì một lần build thừa trên cùng máy 4 lõi, nên chạy sạch sẽ nhanh hơn nhưng chưa đo. Sinh khung: 23 giây. Từ binary xong tới endpoint chạy: khoảng 1 phút. Ngưỡng "dưới 30 phút" **có nguy cơ** khi build nguội, trừ khi hướng dẫn có đường dùng bản dựng sẵn. |
| Chủ cửa hàng, từ sản phẩm tới hoá đơn điện tử | khoảng 15 phút (ước) | Khoảng 6,5 phút trong đó là mò các bước hướng dẫn còn thiếu (khuyến mại, sandbox, thanh toán giả). |
| Người vận hành, PostgreSQL từ đầu tới restore | 4 phút 16 giây | migrate 7 giây, backup 4 giây, restore 14 giây, dừng êm 2 giây. |
| Việc cho AI agent (MCP) | làm được 2,5 trên 3 việc nhiều bước | Việc thứ ba (khách, sản phẩm, đơn nháp) không làm được chỉ bằng tool MCP: thiếu tool cho khách, đơn và bảng giá. |

## Vai 1 — Người mới / code bằng AI (model nhỏ)

Mục tiêu: từ README tới request đầu tiên, rồi nhờ "AI" thêm một tính năng nhỏ.
Kết quả: phần 1 đạt; phần 2 **không đạt** (build module không xong sau 26 phút: máy bị các vai khác làm nặng, xem vai 2).

| # | Mức | Ở đâu | Chuyện gì | Gợi ý |
| --- | --- | --- | --- | --- |
| 1 | khó chịu | README "Getting started", "Configuration" | Mọi lệnh là `cargo run -p autoxide -- …`; người không có Docker hay Rust không biết lấy binary ở đâu. `docs/DEPLOY.md` mục B có nói nhưng README không trỏ tới. | Thêm đoạn "không có Docker": tải binary từ bản phát hành, hoặc `cargo build --release`, và link DEPLOY.md mục B. |
| 2 | khó chịu | README "Configuration" | Không nói `.env` cần hai secret (`AUTOXIDE_AUTH_TOKEN_SECRET`, `AUTOXIDE_SECRET_KEY`); `cp .env.example .env` để chúng bị comment. | Ghi hai lệnh `openssl` ngay bước đó. |
| 3 | khó chịu | README "Configuration" | Không có ví dụ GET có dữ liệu; người thử phải đoán `/api/v1/address/provinces`. | Thêm một ví dụ, và trỏ tới `docs/openapi.json`. |
| 4 | nhỏ | mọi lệnh CLI | Dòng WARN về SMTP / files hiện trước output của mọi lệnh (cả `task`, `user create`), trông như lỗi. | Chỉ hiện ở lệnh dùng tới mail hay files. |
| 5 | nhỏ | writing-a-module.md §1, §5 | Không ước lượng thời gian build lần đầu của module ngoài repo (dựng cả lõi, vào target riêng). | Ghi thời gian; gợi ý dùng chung `CARGO_TARGET_DIR`. |
| 6 | nhỏ | README "Try it with Docker" | Nhắc trang Releases mà không có link. | Thêm link. |

Lỗi thật: không thấy ở phần 1. Chưa kiểm: module ngoài, PostgreSQL.

## Vai 2 — Người viết module ngoài repo

Mục tiêu: theo `docs/guides/writing-a-module.md`, làm một module nhỏ ở thư mục ngoài repo, chạy `check-module`, test trên
SQLite và PostgreSQL. Kết quả: **đạt.** Module `links` được sinh bằng `cargo xtask new-module --external … links link
--core-path …` (đúng cách hướng dẫn nói); đó là bản đổi tên của module mẫu, người thử không tự viết thêm nghiệp vụ.
`check-module`: không vi phạm. Test: 8 + 4 bài xanh trên SQLite và trên PostgreSQL (cổng 15432). `POST` / `GET
/api/v1/links` chạy, trùng URL trả 409 `links.url_taken`.

Thời gian: sinh khung 0:23; build server lần đầu 32:08 (chờ biên dịch, bị chậm do build thừa); từ binary tới endpoint
chạy khoảng 1:00; `check-module` 1 giây khi máy rảnh; test SQLite 6:41 (biên dịch lại sau khi máy khởi động lại),
PostgreSQL 10 giây.

| # | Mức | Ở đâu | Chuyện gì | Gợi ý |
| --- | --- | --- | --- | --- |
| 1 | khó chịu | hướng dẫn §4 "Run it" | Ghi "lần build đầu mất một lúc" mà không nêu mức (khoảng 30 phút debug trên 4 lõi). | Ghi con số; gợi ý `--release` hoặc cache. |
| 2 | khó chịu | hướng dẫn §4 | Chỉ nêu `http://127.0.0.1:8080`, không nói biến đổi cổng. Người thử đoán `AUTOXIDE_PORT` / `AUTOXIDE_BIND` (bị bỏ qua im lặng; xem UT-8). | Thêm "đổi cổng: `AUTOXIDE_HTTP_ADDRESS`". |
| 3 | nhỏ | `migrate up` | In một dòng cho mỗi trong 395 migration, rồi log dài của job backup. | Mặc định chỉ tóm tắt, chi tiết khi `-v`. |
| 4 | nhỏ | hướng dẫn §5–6 | Gợi ý Docker cho PostgreSQL; không có gợi ý chạy PostgreSQL cục bộ không Docker. | Một dòng cho PostgreSQL cục bộ. |
| 5 | nhỏ | hướng dẫn §6 so với output của `new-module` | Hai cách gọi `check-module` khác nhau (`--manifest-path …/xtask/Cargo.toml` và `cargo xtask`). | Thống nhất một cách. |

Lỗi thật: không có.

## Vai 3 — Nối AI agent (MCP)

Mục tiêu: nối MCP bằng API key, liệt kê tool, làm ba việc nhiều bước, ghi tool nào mô tả không đủ.
Kết quả: **đạt một phần** (khoảng 4 phút, không chờ build). Đã làm: nội dung (bản nháp, tìm, sửa, revision, yêu cầu
duyệt); catalog + hoá đơn nháp (dòng thuế tính đúng); phân trang và lọc (ba trang, 3+3+1, đúng). Chưa làm: khách, sản
phẩm, đơn nháp (thiếu tool; xem UT-5).

| # | Mức | Ở đâu | Chuyện gì | Gợi ý |
| --- | --- | --- | --- | --- |
| 1 | khó chịu | đọc `.env` | Mật khẩu có dấu cách phải bọc nháy kép; lỗi nêu số dòng, không nêu tên biến. | Chú thích trong `.env.example`; lỗi nêu tên biến. |
| 2 | khó chịu | `docs/guides/mcp.md` §6 | Không nói phải `migrate up` trước `serve` khi chạy binary trần (388 migration, khoảng 20 giây). | Thêm một dòng. |
| 3 | khó chịu | mcp.md §3 | Không giải thích cách làm lệnh ghi qua MCP an toàn khi gọi lại: header `Idempotency-Key` chạy trên `tools/call`, vài tool lại dùng trường `reference`; hai mã lỗi khác nhau (`idempotency_key_reused`, `reference_reused`). | Ghi cả hai cách và tool nào dùng cách nào. |
| 4 | khó chịu | `content.*` `version` | Giá trị là chuỗi ETag có nháy kép (`"\"1795…\""`); gửi bỏ nháy thì `precondition_failed`. Mô tả không nói. | Nhận cả hai dạng, hoặc ghi "gửi y nguyên như đã nhận". |
| 5 | khó chịu | `precondition_failed` qua MCP | Thông báo bảo gửi `If-Match` (header của REST). | Thông báo riêng cho MCP: "đọc lại và gửi `version` mới". |
| 6 | chặn (một phần) | `catalog.add_prices`, `resolve_price` | Cần `price_list_id` nhưng không tool nào liệt kê hay tạo bảng giá; cài mới thì chưa có. Không tool tạo khách hay đơn (đúng như mcp.md). Dòng "lập hoá đơn nháp theo giá niêm yết" ở mcp.md §6 không chạy trên cài mới. | Thêm `catalog.list_price_lists`; ghi việc phải làm trước bằng REST. |
| 7 | nhỏ | `tools/list` | Key chỉ có quyền nội dung vẫn thấy 49 tool, gồm `auth.create_api_key` / `auth.rotate_api_key` luôn lỗi với API key. | Ẩn chúng với principal là API key. |
| 8 | nhỏ | thiếu quyền | Tool bị ẩn trả "Unknown tool", giống tool không tồn tại; mcp.md không nói. | Ghi vào tài liệu. |
| 9 | nhỏ | schema tool | Nhiều tham số không có mô tả hay enum (`catalog.list_products` `readiness`, `category_id`, `kind`; `update_product`; `upsert_variant` `create`/`update` kiểu null; `add_prices`; `invoices.create_draft` `buyer_*`; `resolve_price`; kích thước và `meta_*` của variant). | Gate bắt buộc mô tả cho mọi tham số tool. |
| 10 | nhỏ | `content.create_draft` | `locale` để trống ra `en` dù tiêu đề tiếng Việt; không nêu mặc định. | Nêu mặc định. |
| 11 | nhỏ | `invoices.create_draft` | Sai cùng lúc kind, currency, đơn giá chỉ báo một lỗi (luật dự án: gom mọi lỗi một lần). | Gom lỗi. |
| 12 | nhỏ | cursor | Cursor vẫn được nhận khi bộ lọc đã đổi. | Gắn bộ lọc vào cursor. |
| 13 | nhỏ | `Idempotency-Key` trùng, khác thân | Trả HTTP 422 thô, không phải lỗi của tool (`isError`); client MCP thấy lỗi tầng HTTP. | Bọc thành lỗi tool. |
| 14 | nhỏ | rate limit | Tài liệu ghi 120/phút/key; nghỉ 62 giây rồi chỉ 32 trên 125 request qua, sau đó 429; thông báo nói "address" thay vì key; key đã thu hồi dùng 65 lần cũng ra 429 thay vì 401. | Đối chiếu tài liệu và hành vi. |

Lỗi thật:

- **Key quyền tối thiểu thu hồi được key khác của cùng người dùng.** Tạo key A (`content.read` + `content.write_own`) và
  key B (`audit.read` + `content.read`). Bằng B: `tools/call auth.revoke_api_key {"id":"<A>"}` trả `revoked`; A sau đó
  bị 401. Tool này không cần permission (tạo key thì cấm với key). Mong đợi: key chỉ thu hồi được chính nó, hoặc thu hồi
  cần permission riêng. (Bảo mật: phải qua kiểm độc lập.)
- `catalog.create_product` nhận `"tax_category":"nope"` và lưu (mô tả nói mã phải thuộc `GET /api/v1/tax/categories`).
- `catalog.list_products readiness:"x"`, `audit.list_entries module:"nope"` hay `subject:"bad"` trả danh sách rỗng thay
  vì lỗi (`status` sai thì có báo lỗi): không nhất quán.
- Bản ghi audit của agent có `actor_kind:"user"` kèm `actor_api_key_id`, không có tên key; mcp.md nói "dưới tên của key".

## Vai 4 — Người vận hành

Mục tiêu: dựng với PostgreSQL 16 (không có Docker daemon), migrate, doctor, metric, backup, restore, dừng êm, nâng cấp.
Kết quả: **đạt** (4 phút 16 giây). Mọi lệnh tài liệu nêu mà chạy được đều đúng như tài liệu. Restore: 460 bảng, số dòng
khớp (trừ vài dòng do chính người thử đăng nhập thêm), mật khẩu admin cũ dùng được, 3355 đơn vị địa chỉ và 8 mức thuế còn
nguyên. Chưa thử (không có Docker, hoặc ngoài phạm vi): `docker compose` / `docker run`, `gh attestation verify`,
systemd, `copy` từ SQLite sang PostgreSQL, nhiều khách, OTLP, backup mã hoá / PITR.

Nâng cấp: không có bản phát hành cũ để nâng cấp thật. Đã giả lập đường tắt/bật module mà tài liệu mô tả: `serve` cảnh báo
`modules_turned_off`, `doctor` cảnh báo, `autoxide modules` báo "1 table(s) with rows", `backup` từ chối kèm hướng dẫn;
`migrate down customers` rồi `up` lại sạch.

| # | Mức | Ở đâu | Chuyện gì | Gợi ý |
| --- | --- | --- | --- | --- |
| 1 | khó chịu | DEPLOY.md §B | Không có mục "binary + PostgreSQL từ đầu"; §B không nêu `AUTOXIDE_HTTP_ADDRESS` (người thử gõ nhầm `AUTOXIDE_HTTP_PORT`). | Thêm mục ngắn "B+". |
| 2 | nhỏ | `config`, `serve` | Biến `AUTOXIDE_*` viết sai tên bị bỏ qua; `config` báo "valid"; server nghe cổng mặc định 8080. | Cảnh báo tên `AUTOXIDE_*` lạ. |
| 3 | nhỏ | `migrate down … --steps 0` | In "no applied migrations" cho module có 13 migration. | Từ chối `--steps 0` bằng câu rõ. |
| 4 | nhỏ | `backup`, `restore` | 434 dòng INFO "table copied" che mất kết quả. | Chỉ in tóm tắt. |
| 5 | nhỏ | log khi drain | Mỗi `/health/ready` 503 `shutting_down` ghi mức ERROR; `tenant_id=` lặp hai lần trên dòng log request. | Hạ mức; bỏ trường trùng. |
| 6 | nhỏ | PostgreSQL không tới được | Sai cổng: "pool timed out" sau 10 giây, không có "connection refused"; vài lỗi có đuôi lạ "at line 331". | Nêu nguyên nhân gốc. |
| 7 | nhỏ | thông báo lỗi và CHANGELOG (~dòng 396–397) | Ghi `autoxide backup create`; lệnh thật là `autoxide backup <thư mục>`. | Sửa câu chữ. |
| 8 | nhỏ | DEPLOY.md "Settings worth knowing" | Ghi `AUTOXIDE_AUTH_TOKEN_SECRET` "required in production", nhưng thiếu nó server vẫn chạy với khoá ngẫu nhiên và một WARN. | Ghi "recommended". |

Lỗi thật: không có lỗi cho kết quả sai (chỉ #3, thông báo sai). Làm tốt: thông báo lỗi nêu lệnh cần chạy, `doctor` gợi ý
cách sửa từng mục, khởi động 1,1 giây, `/metrics` ở cổng riêng, drain đúng như mô tả.

## Vai 5 — Chủ cửa hàng dùng API (curl và SDK TypeScript)

Mục tiêu: khách, sản phẩm, kho, khuyến mại, đơn hàng, thanh toán giả, hoá đơn điện tử nháp. Kết quả: **đạt một phần.**
Cả luồng bán hàng chạy bằng curl và bằng SDK; tiền kiểm tay khớp (300.000; 480.000 với thuế 43.636; 210.000 với thuế
19.091), là chuỗi số nguyên kèm `VND`; bốn người mua song song chiếc cuối chỉ một đơn thành công; hoàn tiền một phần, huỷ
đơn đã trả, gọi lại hoàn tiền đều không nhân đôi. Khoảng 15 phút (ước); khoảng 6,5 phút là mò các bước hướng dẫn thiếu.
SDK chạy thẳng bằng Node 22, không cần `npm install`.

| # | Mức | Ở đâu | Chuyện gì | Gợi ý |
| --- | --- | --- | --- | --- |
| 1 | chặn (một phần) | shop.md, sandbox.md, README | Chỉ có hướng dẫn nhà cung cấp thật. Sandbox nói "thanh toán thành công", nhưng khoản `momo` đứng `pending`; không có cách chính thức để hoàn tất thanh toán giả (người thử tự ký thông báo MoMo bằng key bịa). | Ghi cách chính thức hoàn tất thanh toán giả, hoặc cho sandbox tự thành công. |
| 2 | khó chịu | shop.md "What you need" | Không có đường thử không cần tài khoản nhà cung cấp: sandbox cần `AUTOXIDE_TENANCY_MODE=multi` và `autoxide tenant create`, không có trong quickstart. | Thêm mục "thử không cần tài khoản thật". |
| 3 | khó chịu | sandbox.md | Sandbox mới tạo là rỗng (không sản phẩm, giá, kho, vận chuyển, hồ sơ thuế) và không có hướng dẫn dựng cửa hàng trong đó; dùng `uuidgen` (máy không có); không nói đường công khai cũng cần `Autoxide-Tenant: <slug>`. | Nói rõ phần không sao chép; dùng `openssl rand -hex 16`. |
| 4 | khó chịu | shop.md | Thiếu bước: phí ship thủ công (zone, method, rates); hồ sơ thuế (`PUT /tax/profiles/{org}`: thiếu thì thuế = 0 một cách im lặng); dãy hoá đơn (thiếu thì không phát hành, chỉ một dòng log `invoices.series_not_found`); `jq` bắt buộc; giá trị của `business_lines`; `address` là object. | Thêm các bước. |
| 5 | khó chịu | PROMOTIONS.md "Turning on" | Không chỉ cách tạo mã (`POST /promotions/{id}/codes` cần `code` + `purpose:"redeem"`), nơi đặt `promotions.notice_province` (setting của đơn vị: `PUT /organizations/units/{id}/settings/…`), hay cách ghi thông báo tỉnh (`PATCH …/notices/province:01`). | Chuỗi lệnh curl đầy đủ. |
| 6 | khó chịu | PROMOTIONS.md | Khuyến mại bị từ chối `promotions.reason.no_reference_price` nếu giá của biến thể bắt đầu sau `starts_at` của khuyến mại; cửa hàng vừa đặt giá thì hôm nay chưa giảm giá được. Tài liệu không nói; lý do không nêu cách sửa. | Ghi vào tài liệu; viết lý do rõ hơn. |
| 7 | khó chịu | providers/momo.md | Không có công thức ký hay nghĩa của `orderId` để thử thông báo cục bộ. | Thêm ví dụ thử. |
| 8 | khó chịu | shop.md bước 4–5 | Khách vãng lai không nhận `Order-Token` khi đặt; chỉ có qua tra cứu SMS (`lookup`, rồi `confirm`, trường `access_token`). Hướng dẫn nói gửi `Order-Token` mà không nói lấy ở đâu. | Mô tả luồng. |
| 9 | nhỏ | shop.md | Giỏ liệt kê `payment_methods` không có `cod` / `wallet`, nhưng cả hai đều được nhận. | Làm hai danh sách khớp nhau. |
| 10 | nhỏ | checkout | `bank_transfer` khi chưa có tài khoản ngân hàng: `ledger.bank_account_not_found` không nói phải tạo (`POST /ledger/bank-accounts`). | Thông báo rõ hơn. |
| 11 | nhỏ | sandbox.md | Ghi số hoá đơn bắt đầu bằng `SANDBOX-`; thực tế `number` là `1`, chỉ `provider_ref` / `lookup_code` / `authority_code` mới thế. | Sửa câu. |
| 12 | nhỏ | `GET /promotions/notice-rules` | Trường `note` chứa ID quyết định nội bộ "(D-225)". | Bỏ khỏi dữ liệu pack. |
| 13 | nhỏ | sdk/typescript README | Import `@autoxide/sdk`, nhưng chép thư mục `src` không tạo ra tên đó; không có ví dụ header tenant. | Import tương đối; ví dụ bọc `fetch`. |
| 14 | nhỏ | `…/issue` | Khi có hai dãy cùng loại thì lỗi `series_id: is required`; việc tự phát hành lúc giao hàng chỉ ghi log `validation_failed` không nêu lý do. | Thông báo nêu cách sửa (PATCH), log nêu lý do. |

Lỗi thật:

- **L1 (nặng nhất): tồn kho `on_hand` không bao giờ giảm sau khi giao hàng.** Tenant sandbox, một biến thể nhập 10, đặt 2,
  thanh toán (thông báo), tạo shipment, `POST /shipping/shipments/{id}/status` lần lượt `ready_to_ship`, `picked_up`,
  `in_transit`, `delivered` (hoặc `POST /orders/{id}/transitions` sang `preparing`, `shipping`, `delivered`). Sau khi
  đợi 20 giây thấy: đơn `delivered` nhưng `GET /inventory/levels` vẫn `on_hand 10, available 8, reserved 2`;
  `GET /inventory/movements` chỉ có dòng điều chỉnh `+10`, không có `sale`; reservation vẫn `confirmed`;
  `fulfillment_status` của đơn vẫn `unfulfilled`. Mong đợi (INVENTORY.md, mục Reservations: "fulfil — writes `sale`
  lines"): `sale −2`, `on_hand 8`, `reserved 0`, `fulfilled`. `available` đúng nên không bán lố, nhưng sổ kho sai.
- L2: `payment_actions[].payment_id` trong checkout (và trong `POST /orders/{id}/payment-link`) không phải id khoản
  thanh toán của ledger: `GET /ledger/payments/{id}` trả 404 `ledger.payment_not_found`; id đúng chỉ thấy ở
  `GET /ledger/payments` (lọc `source_id`).
- L3 (nhỏ): gọi lại checkout, hay huỷ đơn khách vãng lai, cùng `Idempotency-Key` trả 200 (lần đầu 201) nhưng không có
  header `Idempotent-Replayed: true`, trái retries.md và mô tả OpenAPI.
- L4 (nhỏ): `POST /orders/{id}/payment-link` sau khi thanh toán lần đầu lỗi trả lại `payment_id` và `status_token` của lần
  đầu (đã lỗi); token đó vẫn `pending` trong khi ledger có khoản mới với id khác.
- L5 (nhỏ): thông báo MoMo sai số tiền đặt `needs_review` (đúng tài liệu); thông báo đúng gửi sau đó không đưa về
  `succeeded` và đơn kẹt `pending_payment`; tài liệu không nói người vận hành gỡ thế nào.

## Tổng hợp theo ưu tiên

1. **Chặn 1.0:** UT-1 (tồn kho không giảm khi giao hàng), UT-2 (key quyền tối thiểu thu hồi được key khác).
2. **Khó chịu, nên sửa trước khi mở kênh phản hồi RC:** UT-3 (đường nhanh nhất khi không có Docker: binary, secret,
   `migrate up`, cổng, PostgreSQL), UT-4 (không thử được bán hàng nếu chưa có nhà cung cấp thật; hướng dẫn shop thiếu
   bước), UT-5 (tool MCP: thiếu mô tả tham số, bảng giá / khách, kiểm giá trị), UT-6 (id thanh toán, header idempotency,
   payment-link gọi lại), UT-7 (thời gian build lần đầu của module ngoài repo).
3. **Nhỏ:** UT-8 (thông báo và log cho người vận hành), UT-9 (tài liệu rate limit lệch hành vi, ID nội bộ trong dữ liệu công khai).

Ghi chú: thời gian build lần đầu của module (khoảng 30 phút) khiến ngưỡng G9 "module đầu tiên dưới 30 phút" có nguy cơ.

Dọn dẹp: một lần build thừa do vai 1 để lại tranh CPU với vai 2 gần hết thời gian chờ; phiên này không dừng được nó. File
chỉ nằm trong `/tmp` của máy dùng thử; không sửa mã sản phẩm. Vai 4 và vai 2 để lại PostgreSQL cục bộ đang chạy (cổng
15434 và 15432).
