# Lộ trình

Mỗi bước xong khi `./gate.sh --quick` xanh; agent đánh dấu `[x]`, commit, merge vào `main`. Gate đầy đủ (**cả SQLite
và Postgres**) bắt buộc xanh trước mỗi lần phát hành (D-300). Chi tiết
thư viện: [`STACK.md`](../STACK.md). Cách làm từng bước: [`docs/steps/`](steps/README.md).

Trạng thái: **bước 0–38 và 40 xong** — đã chuyển sang [`docs/roadmap/done.md`](roadmap/done.md) (D-228);
**kế tiếp bước 71** (đường tới 1.0, D-20261126-road-to-1-0) cùng phần còn mở của bước 70; đóng băng tính năng mới tới
1.0; bước 50 chờ chủ dự án công khai repo (D-199); bản
phát hành mới nhất `v0.19.0`. CI trên GitHub (`.github/workflows/ci.yml`) chạy gate đầy đủ trên mỗi pull request và mỗi lần push lên `main` (D-175, D-197); `./gate.sh` trên máy vẫn là định nghĩa "xong".

**Số bước là mã cố định, không phải thứ tự** (D-228): bước mang số lớn vẫn có thể làm trước. Các bước dưới đây
xếp theo **thứ tự làm**; bước kế tiếp là bước đầu tiên (từ trên xuống) còn mục `[ ]` mà các bước nó phụ
thuộc đã xong. **Bước ghi "tạm dừng tới sau 1.0" không bao giờ là bước kế tiếp** (D-20261121-community-first).

## Thứ tự làm (D-228, D-20261121-community-first)

**Từ ngày 26/11/2026 (chủ dự án quyết, D-20261126-road-to-1-0):** bước 71 (đường tới 1.0) và các mục còn mở của bước
70 (P4, P5, P6, P9, P16, P17) làm trước mọi bước khác; không mở tính năng nghiệp vụ mới tới 1.0. Các mục còn mở của 54
và 52 cũng làm trước 1.0, sau bước 71 G1–G2 (D-20261126-steps-52-54-before-1-0). Điều kiện ra 1.0 ghi ở đầu bước 71. Đoạn "Từ ngày 21/11/2026" dưới đây giữ để tra lịch sử; thứ tự
1–5 của nó đã được thay.

**Từ ngày 21/11/2026:** bước 70 (P1–P3) làm trước mọi bước còn mở. Sau đó xếp theo kịch bản "một cửa hàng bán online
chạy thật" (D-20261121-shop-first-order):

1. Bán được hàng thật: bước 70 P7 (bộ nối nhà cung cấp) → hai mục còn mở của 45 không chờ 67 → bước 70 P8 (luồng bán
   hàng mẫu chạy trọn).
2. Mặt tiền cửa hàng: 67, phần sổ khối và khối của `catalog` / `promotions` / `orders` trước (đuôi 41, 43, 45), rồi
   phần còn lại của 67.
3. Phân quyền 54: xác thực lại cho việc nhạy cảm và quyền theo trường dữ liệu trước, rồi các mục còn lại.
4. Quản trị nâng cao 52.
5. Bản 1.0: bước 70 P6 rồi P4.

Không dựng module mới trước bản 1.0: các bước 46, 48, 49, 56, 59, 62, 63, 64, 65 tạm dừng. Sau 1.0, bước 65 mở đầu tiên
(D-20261125-miniapp-platform). Ba mục cuối của 67 (thử
A/B, AI tạo ảnh, khối mã tự viết) cũng để sau 1.0 (D-20261122-step67-after-1-0). Mục còn mở mà chờ bước tạm
dừng hay mốc thời gian (P17 của 51, banner người bán của 55, gỡ menu cũ của 68, cache quyền của 54) không chặn 1.0.
Bảng dưới là thứ tự gốc của D-228, giữ để tra phụ thuộc.

| Giai đoạn | Bước, theo thứ tự | Vì sao ở đây |
|---|---|---|
| A. Hợp đồng và nền lõi | 33 → 34 → 35, 36, 37 (song song) → 40 | giữ API không vỡ trước khi sửa mọi bảng; đa khách, tổ chức, tiền, tuân thủ là nền của mọi thứ sau |
| B. Lớp cho giao diện và phân quyền | 51 → 68 → 54 → 52 | module thương mại khai luôn phần của mình (51 nên xong trước 41); 68 (menu của website) sau P6 của 51, trước 41; 54 cần P12 của 51 |
| C. Vận hành | 38 → 39 | báo cáo xuyên database, backup, hiệu năng trước khi có dữ liệu thương mại thật |
| D. Module nền dùng chung | 53, 57, 58, 67, 66, 65, 69 | hỗ trợ khách (45 và 61 dùng), phân tích (64 dùng), tự động hoá, trang dựng từ khối và AI tạo trang, SCIM, chợ ứng dụng |
| E. Thương mại | 41 → 42, 43, 44 → 45 → 46, 47, 48, 49, 55 → 56, 59, 60, 61, 62, 63, 64 | module nghiệp vụ cắm vào core đã xong |
| Chờ chủ dự án | 50 | chỉ làm khi chủ dự án nói repo đã công khai |

Bước lớn nhất là 33 (sửa mọi bảng của mọi module): một người làm, không chia song song. Trong một giai đoạn,
bảng phụ thuộc ngay dưới cho biết bước nào chạy song song được.

## Phụ thuộc giữa các bước (D-192)

| Bước | Bắt đầu khi | Song song được với |
|---|---|---|
| 33 Khách hàng, vị trí dữ liệu | 31 và nhóm A của 32 xong (31 giữ API và bản nâng cấp không vỡ khi mọi bảng đổi) | hai mục độc lập của 40 |
| 34 Tổ chức, quyền, danh tính | 33 | — |
| 35 Gói, thuê bao · 36 Tiền, thuế · 37 Tuân thủ | 34 | nhau; hai mục cuối của 35 chờ 36 và 37 |
| 38 Xuyên database, báo cáo gộp | 34 và 36 | 40, 41–45 (39 làm sau: rà index cả bảng của 38) |
| 39 Vận hành, hiệu năng | 38 | 40, 41–46 |
| 40 Năng lực còn thiếu | header bảo mật và MCP: ngay; còn lại: 34 | 35–39 |
| 41 Sản phẩm | 35 và 36 | 37–40 |
| 42 Kho · 43 Khuyến mại · 44 Vận chuyển | 41 | nhau |
| 45 Giỏ hàng, đặt hàng, đơn hàng | 42, 43, 44 và 37 (thông báo) | — |
| 46 Sàn · 47 Khách hàng · 48 Đa kênh · 49 Tiếp thị liên kết | 45 (46 và 49 cần thêm 37) | nhau |
| 50 Công khai repo | chủ dự án nói đã công khai repo (D-199) | mọi bước |
| 51 Hợp đồng cho giao diện (D-200) | nhúng bản ghi: sau 33; hộp việc, tìm, số liệu, hoạt động: sau 34; menu: sau 35 | 36–49 — nên làm trước 41 để module thương mại khai luôn phần của mình |
| 52 Trải nghiệm quản trị nâng cao (D-201) | 51 | 41–49 |
| 68 Menu của website (D-238) | phần chính: đã làm trước theo lệnh chủ dự án; theo khách: 33; theo website: 34; gỡ menu cũ: sau 2027-05-14 | 51–67 |
| 53 Hỗ trợ khách hàng (D-204) | 37 (thông báo, vụ việc khiếu nại) | 41–49 |
| 54 Phân quyền chuẩn hàng đầu (D-205) | 34; P6, P7 cần P12 của 51; P1 làm ở bước 31 | 41–49, 53 |
| 55 Giao diện cửa hàng (D-219, D-236) | 41 (banner và thông tin cửa hàng làm được từ sau 34); khối vùng banner: 67 | 42–49 |
| 56 Chuyển dữ liệu từ nền tảng khác (D-219) | 47 | 48, 49 |
| 57 Phân tích truy cập (D-220) | 37 (đồng ý), 34 | 41–49 |
| 58 Tự động hoá quy trình (D-220) | 51 (P12 duyệt, P15 job) | 41–49, 57 |
| 59 Bán sỉ B2B (D-221) | 45, 47; ký điện tử: 37; mời thầu: 46 | 48, 49 |
| 60 Đặt lịch dịch vụ (D-221) | 34, 36 (37 cho nhắc lịch) | 41–49 |
| 61 Chat chăm sóc khách (D-221) | 40 (SSE), 53, 37 (thông báo) | 41–49 |
| 67 Trang dựng từ khối, AI tạo trang (D-229, D-231, D-234, D-235) | 51 (gồm P19 cho loại nội dung), 37 (nhãn AI, đồng ý, chân trang pháp lý), 34 (website, múi giờ); thử A/B: 57 | 41–49 (module thương mại khai khối ở bước của nó) |
| 62 Sản phẩm bán theo kỳ (D-221) | 45, 36 | 46–49 |
| 63 Đa tiền tệ (D-221) | 36, 41, 44, 45; người bán nước ngoài: 37, 46 | 47–49 |
| 64 Quảng cáo trong sàn (D-221) | 46, 57 | 47–49 |
| 65 Chợ ứng dụng OAuth (D-221) | 54, 33; bán ứng dụng trả phí: 35, 36 | 41–49 |
| 66 SCIM (D-221) | 34, 40 (OIDC) | 35–49 |
| 69 Nền cho ứng dụng đa nền tảng (D-20261119-step69-multiplatform) | SDK, hướng dẫn: ngay; ngoại tuyến: 33–34; WebSocket: 40; mua trong ứng dụng: 35, 36 (đều đã xong) | 41–49, 61 |

Bước lớn nhất là 33 (sửa mọi bảng của mọi module): một người làm, không chia song song.

## Giai đoạn G — Đường tới 1.0 (làm trước, chủ dự án quyết ngày 26/11/2026, D-20261126-road-to-1-0)

## 71. Đường tới 1.0: chuẩn tương lai, chuẩn AI agent, nâng cấp một nút, mọi trình độ dùng được, chuẩn cộng đồng, sẵn sàng phát hành

Nguồn: rà soát core ngày 26/11/2026 (bảy hướng). Mọi mục làm trước khi phát hành 1.0, chọn hướng dài hạn (không làm đối
phó); mục lớn có quyết định thiết kế (skill `record-decision`) trước khi viết code. Mục chạm kernel, bảo mật, tiền, thuế
qua kiểm độc lập (D-20261126-independent-review).

**Điều kiện ra 1.0:** mọi mục `[ ]` dưới đây, P4, P6, P9, P16, P17 của bước 70 và các mục còn mở của 54, 52 xong; gate đầy đủ xanh trên SQLite và
Postgres; vòng kiểm thử bảo mật kiểu bên ngoài cuối cùng không còn lỗi mức cao trở lên; vòng rà pháp lý cuối cùng không còn
điểm chặn; vòng dùng thử bản RC cuối cùng không còn lỗi hợp đồng mở; các chỉ số của G9 đạt ngưỡng. Ba vòng này do phiên AI
đóng vai làm (D-20261126-ai-role-reviews); rà bởi luật sư và đội kiểm thử là con người được khuyến nghị nhưng không chặn 1.0.

### G1 — Phạm vi và hợp đồng 1.0

- [ ] Hai hạng hợp đồng: kernel tách lõi ổn định khỏi phần thử nghiệm (hoá đơn điện tử, ZNS, thoại, khối trang, phân tích,
      `bench_seed`… sau feature hay mô-đun `unstable`); module gắn nhãn hạng; `docs/COMPATIBILITY.md` (EN + VI) nói rõ hạng
      nào được cam kết gì
- [ ] Hợp đồng `Module` một cách làm cho mỗi việc: gom 49 hàm của trait và ~50 hàm đăng ký của `StartContext` thành một
      cách khai báo (manifest hay trait con theo nhóm), mỗi khái niệm một đường đăng ký; giữ đường cũ `#[deprecated]` tới khi
      mọi module đã chuyển; gỡ `TenantContext::legacy_default`
- [ ] `#[non_exhaustive]` cho enum và struct công khai sẽ còn lớn thêm (kernel, testkit, `api` của module); mọi `#[deprecated]`
      viết tiếng Anh, nêu cái thay thế
- [ ] Tách `autoxide-host` (phần chỉ binary dùng) khỏi kernel; file trên 880 dòng tách theo trách nhiệm (phần đa khách
      ~21.700 dòng). Tách phần còn lại của kernel theo lớp: **sau 1.0** — đo 26/11/2026 chỉ rút 6–9% thời gian build lại, dưới
      ngưỡng 20% (D-20261126-kernel-crates; mặt tiền giữ đường dẫn nên tách sau không phá vỡ)
- [ ] Cách ly khách khi chạy, không chỉ lint đọc mã nguồn: lớp kết nối gắn khách (hay row-level security tuỳ chọn trên
      Postgres) chặn truy vấn thiếu `tenant_id`; khách của request chưa xác thực không lấy từ gợi ý chưa kiểm cho việc
      tính hạn mức
- [ ] Hợp nhất hai hệ cài đặt theo khách (cài đặt khách của kernel và cài đặt theo cây của `organizations`): một hệ, kiểm
      chéo lúc lưu, có cache

### G2 — Không để thư viện níu chân (D-20261126-library-isolation)

- [ ] Sửa `docs/COMPATIBILITY.md`, `docs/MODULE_GUIDE.md` (EN + VI) theo quyết định: bỏ "thư viện đổi bản lớn thì
      Autoxide đổi bản lớn"
- [ ] Quyết định thiết kế lớp của kernel cho: truy vấn và entity, route và schema API, kiểm đầu vào, migration; nêu kiểu nào
      bọc, kiểu nào để thư viện, đường chuyển
- [ ] Làm lớp của kernel; chữ ký công khai hạng ổn định của kernel và testkit không còn kiểu của SeaORM, sea-query, axum,
      utoipa (gate kiểm)
      - [ ] Chặn 1.0: gỡ cầu tạm `ConnectionTrait` trên handle của `db` (các hàm kernel nhận handle thay connection) và
            `TenantContext::database()` thô; baseline `raw-database` của `xtask/library-baseline.txt` về rỗng
            (D-20261126-kernel-data-layer)
- [ ] Chuyển 39 module sang lớp của kernel theo cụm; `cargo xtask check` và `check-module` chặn phụ thuộc trực tiếp vào
      thư viện đã có lớp
- [ ] Nhịp và công cụ nâng thư viện: `cargo xtask upgrade` hỗ trợ bản lớn (chạy thử, báo chỗ vỡ), giảm crate trùng phiên
      bản trong `Cargo.lock`, theo dõi hai cảnh báo RUSTSEC đang bỏ qua của `webauthn_rp`

### G3 — Nâng cấp một nút

- [x] Ngay: `compose.yml`, manifest Kubernetes và `AUTOXIDE_MIGRATE_ON_START` nâng cả database riêng của khách
      (`--all`); `up_all` thử mọi nơi đặt dữ liệu, lỗi một khách vẫn làm tiếp, báo lỗi từng khách
- [ ] `autoxide upgrade`: kiểm tương thích theo bảng phiên bản ghi trong binary (nâng từ bản nào được, bước tay bắt buộc),
      tự sao lưu mọi nơi đặt dữ liệu và kho file (hay từ chối nếu không có `--no-backup`), migrate database chính rồi mọi
      khách song song kèm báo cáo JSON tiếp tục được, chạy `doctor` kiểm lại, ghi kế hoạch quay về; `doctor` báo có bản mới;
      compose và Kubernetes dùng lệnh này làm một bước duy nhất
- [ ] `autoxide rollback --to X.Y.Z`: ghi phiên bản binary cạnh mỗi migration đã chạy; lùi theo bản phát hành qua mọi
      module và mọi khách đúng thứ tự phụ thuộc; CI kiểm binary cũ chạy lại được sau khi lùi
- [ ] Ma trận test nâng cấp trong CI: từ bản trước và từ bản cũ nhất còn hỗ trợ, nhiều khách có database riêng, bản cũ và
      bản mới chạy song song sau migration mở rộng, lùi về bản trước
- [ ] Luật an toàn migration: `cargo xtask check` cảnh báo xoá cột / bảng, đổi tên, thêm cột NOT NULL không mặc định trong
      migration mới trừ khi ghi rõ là bước thu hẹp của mở rộng–thu hẹp
- [ ] Ghi chú nâng cấp hạng nhất: mục changelog "cần làm khi nâng cấp", báo cáo semver nêu đúng item và cách sửa, hướng dẫn
      chuyển cho người viết module mỗi bản, `Setting::renamed_from` (tên cũ vẫn đọc được, có cảnh báo) như
      `docs/COMPATIBILITY.md` đã hứa

### G4 — Chuẩn AI agent

- [ ] Đăng nhập chuẩn cho MCP theo đặc tả mới nhất: máy chủ uỷ quyền OAuth 2.1 tích hợp (PKCE, Client ID Metadata
      Documents / đăng ký động), Protected Resource Metadata (RFC 9728) và `WWW-Authenticate` có `resource_metadata`,
      `/.well-known/*`; Claude, ChatGPT và các client MCP khác kết nối được không cần dán khoá
- [ ] Uỷ quyền cho agent: thông tin đăng nhập gắn với một người, phạm vi hẹp, có hạn, thu hồi được (token exchange RFC 8693
      hay tương đương); hạn mức và ngân sách theo từng agent; nhật ký hành động của agent ghi rõ agent nào làm thay ai
- [ ] Người duyệt: rà mọi lệnh xoá, lệnh chạm tiền và lệnh không hoàn tác được để gắn `x-approval` khi agent gọi
- [ ] Chống chèn lệnh qua dữ liệu (prompt injection): kết quả tool và API đánh dấu phần nội dung do người dùng cuối nhập là
      chưa tin cậy; hướng dẫn cho người viết agent
- [ ] Hợp đồng tool MCP đủ: `outputSchema` khớp `structuredContent`, `title`, `idempotentHint`, `openWorldHint`; phân
      trang `tools/list`; báo tiến độ, huỷ, việc chạy lâu (xuất dữ liệu, dời khách, nhập dữ liệu); hỏi lại người dùng
      (elicitation) khi thiếu thông tin
- [ ] Cho máy tự khám phá: `llms-full.txt`, `/.well-known` cho OpenAPI và MCP, mô tả quy trình nhiều bước bằng Arazzo; lỗi
      `Problem` có `type` là URI ổn định cho mỗi mã và gợi ý sửa; khoá chống gọi trùng bắt buộc trên mọi lệnh ghi
- [ ] Giao tiếp giữa agent (A2A): đánh giá đặc tả, thêm agent card nếu đặc tả đã ổn định; ghi quyết định
- [ ] Bộ test bằng agent thật: 10–20 việc mẫu làm qua MCP và REST với mô hình cố định, chạy hằng đêm, đo tỉ lệ xong và chi
      phí; dùng để chấm chất lượng mô tả tool
- [ ] SDK Python sinh từ OpenAPI, có ví dụ và kiểm trong CI như SDK TypeScript

### G5 — Mở rộng không cần Rust

- [ ] Quyết định thiết kế plugin WebAssembly (WASI component model, giao diện WIT, quyền và hạn mức tài nguyên, cách ly
      khách, chạy trên cả hai database, nâng cấp plugin độc lập với core)
- [ ] Máy chạy plugin trong kernel cùng bộ công cụ viết plugin, ví dụ bằng Rust, TypeScript, Python, Go; test trong CI
- [ ] Đường không viết code: quy tắc tự động (`automations`) thành tính năng chính thức, trường tuỳ biến dùng chung trong
      core, công thức nhận webhook "nếu A thì làm B"

### G6 — Mọi trình độ dùng được (người mới, người code bằng AI)

- [ ] `autoxide demo` và một dòng `docker run`: có dữ liệu mẫu, in mật khẩu quản trị ra màn hình, bật trang xem API
- [ ] Binary dựng sẵn cho Linux, macOS, Windows; cấu hình `.devcontainer` / Codespaces
- [ ] `new-module --minimal` và hướng dẫn "module đầu tiên" dưới 50 dòng, thêm từng phần sau; sổ công thức (thêm trường,
      thêm danh sách, thêm quyền, thêm việc nền, thêm tool MCP)
- [ ] Công cụ dòng lệnh độc lập (`autoxide new`, `autoxide check`, cài bằng `cargo install`): tạo và kiểm module không cần
      tải mã nguồn core
- [ ] Rào chắn cho AI: `AGENTS.md` tiếng Anh, luật cho Cursor / Copilot ở gốc repo và trong module sinh ra, skill đi kèm;
      hướng dẫn xử lý các lỗi biên dịch thường gặp
- [ ] Mỗi vi phạm của `xtask check` / `check-module` có một dòng "cách sửa"; lệnh kiểm môi trường cài đặt cho người phát
      triển

### G7 — Chuẩn giao thức còn thiếu

- [ ] Header `RateLimit` / `RateLimit-Policy` (bản nháp IETF mới nhất) và 429 kèm `Retry-After` khai trong OpenAPI cho mọi
      thao tác
- [ ] ETag: khác nhau khi nén khác nhau (hay dùng ETag yếu); `If-Match` nhận danh sách
- [ ] OpenAPI: bản mới nhất mà bộ sinh hỗ trợ (3.2), `info.license`, `webhooks` cho payload gửi đi, `components.responses`,
      cách xác thực API key riêng
- [ ] OpenTelemetry: tên thuộc tính theo semantic conventions, truyền `traceparent` khi gọi ra ngoài và qua job / event,
      xuất metrics và log qua OTLP
- [ ] Mật khẩu: chặn mật khẩu đã lộ theo NIST SP 800-63B-4; HSTS bật mặc định ở môi trường production
- [ ] Image: nền distroless hay tĩnh, kiểm sức khoẻ bằng chính binary (`HEALTHCHECK`, healthcheck trong compose)

### G8 — Chuẩn cộng đồng mã nguồn mở

- [x] CodeQL (Rust, JS/TS, workflow), OpenSSF Scorecard (`codeql.yml`, `scorecard.yml`); huy hiệu OpenSSF Best Practices và
      branch protection do chủ dự án làm trên web, hướng dẫn ở `docs/OPENSSF.md` (EN + VI)
- [x] Chính sách đóng góp do AI viết, DCO (`DCO`, job `dco` của CI, CODEOWNERS), commit convention,
      `SUPPORT.md`, `GOVERNANCE.md` (vai trò, quyết định, RFC, bộ nhãn), `docs/rfcs/`, `CODEOWNERS`, `contact_links` trong
      mẫu issue; chủ dự án tạo bộ nhãn trong cài đặt repo (bảng nhãn ở `GOVERNANCE.md`)
- [x] SBOM (`cargo auditable` trong Dockerfile, CycloneDX `autoxide.cdx.json` trong bản phát hành), build tái lập
      (`SOURCE_DATE_EPOCH`, `--remap-path-prefix`, tar xếp theo tên), job CI dựng bằng MSRV, mục MSRV / feature / build tái lập /
      SBOM trong `docs/COMPATIBILITY.md`
- [x] Bản tiếng Anh cho người ngoài: `docs/ROADMAP.public.md` (lộ trình công khai, tóm tắt luật đóng góp), RFC ở `docs/rfcs/`
- [x] Hệ sinh thái module: `docs/MODULE_ECOSYSTEM.md` (tên `autoxide-*`, topic `autoxide-module`, manifest
      `autoxide-module.toml` đề xuất, bảng tương thích, danh sách cộng đồng), `TRADEMARK.md` (EN + VI)
- [x] `REUSE.toml` + `LICENSES/` (job `reuse`); `fuzz/` (cargo-fuzz: con trỏ trang, header chọn khách, câu tìm kiếm, CSV,
      số thập phân / tiền, thời gian) + `fuzz.yml` chạy hằng tuần. Chưa phủ: `Idempotency-Key` và thân JSON (hàm riêng của tầng
      HTTP) — thêm khi phần phân tích đó lộ ra thành hàm công khai

### G9 — Chất lượng thành quy trình và đo bằng con số

- [x] Kiểm độc lập bắt buộc theo D-20261126-independent-review (mẫu pull request có mục kết quả kiểm)
- [ ] Mutation testing (`cargo-mutants`) cho kernel và phần tiền / thuế, chạy định kỳ — workflow `mutants.yml` và báo cáo đã có; còn: lần chạy CI đầu tiên để đặt `baseline` trong `docs/quality/mutation.json` (đang `null`, chưa có hồi quy nào làm đỏ được), rồi chỉnh số shard; chạy thêm trên Postgres và test tích hợp (hiện chỉ `--lib` trên SQLite)
- [x] Bất biến tiền, thuế, quyền thành property test chạy hằng đêm (rà ở `docs/quality/invariants.md`; phần còn thiếu ghi cuối file)
- [ ] Đo tải trong CI hằng tuần trên cả hai database, chặn khi p95 của đường nóng tệ đi; vài benchmark vi mô cho đường
      nóng của kernel
      — `load.yml`, `scripts/load-compare.py`, benchmark `divan` đã có; còn: lần chạy CI đầu để đặt số gốc trong
      `docs/quality/load-baseline.json` (đang `null`, chưa chặn được hồi quy nào)
- [ ] Chỉ số đo mỗi bản phát hành, ngưỡng cho 1.0: tải về tới request đầu tiên dưới 5 phút; người mới xong module đầu
      tiên dưới 30 phút; nâng cấp trong CI 100% thành công kể cả nhiều khách; agent thật làm xong việc mẫu trên 90%; p95
      đường nóng không tệ đi giữa hai bản
      — bảng, ngưỡng và cách đo ở `docs/RELEASING.md` ("Chỉ số của mỗi bản phát hành"), workflow `release-metrics.yml` đã có;
      còn: số đo người mới / agent làm module đầu (`docs/quality/first-module-timing.json`), file kết quả eval của G4,
      và lần chạy đầu của `load.yml` (số gốc p95)

### G10 — Giảm tải cho agent phát triển

- [ ] Rút `CLAUDE.md` còn khoảng một phần ba (giữ luật và chỉ dẫn, quy trình chuyển sang skill); chuyển mục đã xong của
      lộ trình sang `docs/roadmap/done.md`

### G11 — Ứng dụng tham chiếu

- [ ] Dựng một cửa hàng nhỏ ở repo riêng như người ngoài (module viết ngoài repo, giao diện đơn giản), ghi mọi chỗ vướng
      thành việc của core — **chờ chủ dự án tạo repo**

### G12 — Rà từ bên ngoài bằng phiên AI đóng vai (chủ dự án quyết ngày 26/11/2026, D-20261126-ai-role-reviews)

- [ ] Rà pháp lý (phiên đóng vai luật sư công nghệ và chuyên gia thuế, hoá đơn): dữ liệu cá nhân (Luật 91/2025/QH15,
      Nghị định 356/2025/NĐ-CP), thương mại điện tử, hoá đơn điện tử, thuế, thanh toán, an ninh mạng; mỗi điểm nêu căn cứ
      (văn bản, điều, ngày hiệu lực) hay ghi "cần đối chiếu văn bản gốc / người có chuyên môn pháp lý"; vòng 1 trên `main`
      hiện tại, vòng cuối trên bản RC
- [ ] Kiểm thử bảo mật kiểu bên ngoài (phiên đóng vai đội pentest độc lập, chỉ biết tài liệu công khai, chỉ tấn công máy
      chủ chạy trong chính phiên đó): vòng 1 sau khi phần code lớn của G1–G4 đã merge (lớp kernel mới, OAuth, nâng cấp),
      vòng cuối trên bản RC; lỗi cao trở lên báo kín, công bố sau khi
      sửa; sau 1.0 cân nhắc chương trình thưởng báo lỗi bảo mật

Việc từ rà pháp lý vòng 1 do AI đóng vai (`docs/legal/review-2026-11-r1/`, không phải ý kiến pháp lý):

- [x] LG-1 (chặn 1.0): quy tắc `moit.checklist.*`, `cases.deadlines` phiên bản mới theo NĐ 248/2026 (thay NĐ 52/2013, 85/2021
      từ 01/7/2026), chuyển tiếp tới 30/6/2027; sửa `COMPLIANCE.md` / `.vi.md` và mẫu trang
- [ ] LG-2 (chặn 1.0): ngày lập hoá đơn và `tax_point` của đơn theo múi giờ người bán, không theo UTC; test mốc
      31/12/2026 – 01/01/2027 giờ Việt Nam; xong trước 31/12/2026; qua kiểm độc lập
- [ ] LG-3: tài liệu nói rõ cổng KYC người bán chưa chặn đăng bán; cài `ListingGate` gọi `KycGate` là điều kiện của module
      sàn; đối chiếu trường KYC với NĐ 248 Điều 18 (từ 01/01/2027)
- [x] LG-4: quy tắc `privacy.subject_requests` v2 theo NĐ 356/2025 Điều 5 (hạn xác nhận và hạn thực hiện, ngày làm việc)
- [ ] LG-5: dữ liệu cá nhân của trẻ em — tự khai tuổi khi đăng ký (cấu hình được), sổ đồng ý ghi người giám hộ
- [ ] LG-6: module khai bên xử lý và nước nhận dữ liệu (AI, mail, SMS, lưu trữ) vào sổ xử lý; `doctor` cảnh báo bên nhận
      ngoài Việt Nam; `docs/AI.md` thêm mục dữ liệu cá nhân
- [ ] LG-7: gói thuế `vn` v2 theo NĐ 68/2026 (ngưỡng hộ kinh doanh, tỉ lệ khấu trừ); `orders` hỏi
      `TaxEngine::invoice_requirements` thay vì quyết theo kênh `pos`
- [ ] LG-8: tài liệu tách phần khấu trừ thay đã có và chưa có; phát `TaxWithheld`, chứng từ khấu trừ, dữ liệu tờ khai là điều
      kiện của module sàn
- [ ] LG-9: huỷ hoá đơn đã gửi SePay — lỗi rõ hướng sang điều chỉnh / thay thế; xác định nghĩa vụ 04/SS sau NĐ 70/2025
- [ ] LG-10: phát hiện và báo chênh lệch thuế giữa đơn và hoá đơn khi qua mốc đổi thuế suất
- [x] LG-11: hạn mức và thủ tục khuyến mại theo NĐ 239/2026 và thông tư của Bộ Công Thương (thay căn cứ NĐ 81/2018 Điều 6, 7)
- [ ] LG-12: `orders` ẩn đánh giá qua `ModerationPolicy` (ánh xạ mã lý do) hoặc sửa `COMPLIANCE.md` cho khớp code
- [x] LG-13: căn cứ Luật An ninh mạng 116/2025 và nghị định hướng dẫn thay NĐ 53/2022 trong tài liệu lưu dữ liệu trong nước
- [ ] LG-1b: khi có văn bản gốc NĐ 248/2026 — thêm mục kiểm mới (tiêu chí xếp hạng, quy tắc điểm thưởng, báo cáo
      định kỳ), xác nhận cơ quan tiếp nhận / online.gov.vn và hạn chuyển tiếp (D-20261126-legal-rules-r1-update)
- [ ] LG-4b: tính hạn thực hiện yêu cầu của chủ thể dữ liệu theo `fulfil_within_days` (ngày làm việc, bên xử lý thứ ba)

### G13 — Phát hành 1.0 (chủ dự án duyệt ngày 26/11/2026, D-20261126-release-1-0-readiness)

- [ ] Đánh phiên bản mọi hợp đồng, không chỉ REST: tên và payload event giữa module, payload webhook gửi đi, tên và schema
      tool MCP, schema `x-*` của OpenAPI; quy tắc thêm / đổi / bỏ, cách khai bản cũ sắp bỏ, test giữ hợp đồng (như api-diff)
- [ ] Chạy thử trước 1.0: phát hành `1.0.0-rc.N`; phiên AI đóng vai người dùng (người mới / code bằng AI, người viết
      module, người nối AI agent qua MCP, người vận hành, chủ cửa hàng dùng API) dùng thử theo tài liệu công khai, vài vòng
      (dùng thử → sửa → vòng sau, D-20261126-ai-role-reviews); vẫn mở kênh phản hồi cho người ngoài thật; tiêu chí chốt
      (không còn lỗi hợp đồng mở); chỉ sửa lỗi trong thời gian này
- [ ] Chạy thử vòng 1 bằng agent đóng vai người dùng (`docs/trials/rc-trial-r1/`, agent đóng vai, không phải người thật) —
      các mục dưới đây đều có mã UT-x trong báo cáo; xong hết thì chạy vòng 2 trên `1.0.0-rc.1`
  - [ ] UT-1 (chặn): giao hàng không ghi dòng `sale` vào kho — `on_hand` không giảm, reservation vẫn `confirmed`, đơn vẫn
        `unfulfilled` sau `delivered`; có test chạy trên cả hai database; chạm tiền / kho nên qua kiểm độc lập
  - [ ] UT-2 (chặn): phân quyền khi quản lý API key qua MCP chưa đúng phạm vi (đã báo kín, công bố sau khi sửa); có test;
        chạm bảo mật nên qua kiểm độc lập
  - [ ] UT-3: đường chạy bằng binary không Docker cho người mới: README trỏ tới DEPLOY.md mục B, hai secret ở bước
        `.env`, `migrate up` trước `serve`, `AUTOXIDE_HTTP_ADDRESS`, mục "binary + PostgreSQL từ đầu", một ví dụ GET có
        dữ liệu; `config` cảnh báo biến `AUTOXIDE_*` lạ; lỗi đọc `.env` nêu tên biến
  - [ ] UT-4: thử luồng bán hàng khi chưa có tài khoản nhà cung cấp thật: cách chính thức hoàn tất thanh toán giả trong
        sandbox, mục "thử không cần tài khoản thật" (chế độ multi, `tenant create`, header `Autoxide-Tenant`), dựng
        cửa hàng trong sandbox; `shop.md` và `PROMOTIONS.md` thêm bước còn thiếu (phí ship thủ công, hồ sơ thuế, dãy hoá
        đơn, tạo mã, tỉnh thông báo, luật giá tham chiếu, Order-Token của khách vãng lai)
  - [ ] UT-5: tool MCP đủ để agent làm việc nhiều bước: mô tả cho mọi tham số (gate kiểm), `catalog.list_price_lists`,
        kiểm `tax_category`, giá trị lọc sai (`readiness`, `module`, `subject`) báo lỗi, `version` nhận cả hai dạng, thông
        báo `precondition_failed` riêng cho MCP, ẩn tool luôn lỗi với API key, cách idempotency của MCP ghi trong `mcp.md`,
        lỗi `Idempotency-Key` trùng thành lỗi tool, gom lỗi của `invoices.create_draft`, tên key trong audit
  - [ ] UT-6: `payment_id` của checkout / `payment-link` dùng được ở API ledger; header `Idempotent-Replayed` cho checkout
        và huỷ đơn khách; `payment-link` gọi lại sau lần lỗi; cách gỡ khoản `needs_review`
  - [ ] UT-7: thời gian build lần đầu của module ngoài repo (đo ~30 phút debug trên 4 lõi): hướng dẫn nêu con số, đường dùng
        bản dựng sẵn hay cache, một cách gọi `check-module`; đo lại cho G9 trên máy rảnh
  - [ ] UT-8: thông báo và log cho người vận hành: `--steps 0`, log backup / restore chỉ tóm tắt, `migrate up` tóm tắt,
        log drain không để mức ERROR, `tenant_id` lặp, lỗi kết nối database nêu nguyên nhân gốc, WARN SMTP chỉ ở lệnh dùng
        tới, `autoxide backup create` sửa thành `backup <thư mục>`, chữ "required" của `AUTOXIDE_AUTH_TOKEN_SECRET`
  - [ ] UT-9: tài liệu rate limit khớp hành vi (120/phút/key, thông báo nói key, key đã thu hồi ra 401); bỏ ID quyết định
        nội bộ ("D-225") khỏi dữ liệu công khai của `promotions`; sửa `sandbox.md` (số hoá đơn) và README của SDK (import,
        header tenant)
- [x] Chính sách hỗ trợ dài hạn: bản 1.x nhận vá bảo mật bao lâu, bản lớn trước còn được hỗ trợ bao lâu khi bản lớn mới ra,
      quy trình vá ngược; ghi vào `docs/COMPATIBILITY.md` và `SECURITY.md` (EN + VI) — quy trình vá ngược và bảng ở `COMPATIBILITY.md` / `SECURITY.md`
- [x] Trang tài liệu có phiên bản: website tài liệu chính thức (hướng dẫn, tham chiếu API sinh từ OpenAPI, MCP, docs.rs),
      có tìm kiếm, mỗi bản phát hành một bộ tài liệu, EN + VI, tự dựng khi phát hành — mdBook, `scripts/build-docs-site.py`, workflow `docs-site.yml` (D-20261126-docs-site-mdbook); **chủ dự án bật GitHub Pages (nguồn: nhánh `gh-pages`)**
- [ ] Chạy nhiều máy chủ cùng lúc: test trong CI 2–3 máy chủ song song không giao trùng việc nền / event, không mất lock
      migration; tài liệu sức chứa (SQLite chịu tới đâu, khi nào chuyển Postgres, số khách mỗi máy chủ, cấu hình đề xuất)
- [ ] Mô hình mối đe doạ của core (`docs/THREAT_MODEL.md`, EN + VI) làm đầu vào cho đợt kiểm thử bên ngoài; mã hoá ở mức
      trường cho dữ liệu cá nhân nhạy cảm (khoá quản lý được, xoay khoá); nhật ký kiểm toán khó sửa lén (chuỗi băm hay tương
      đương)
- [ ] Mục tiêu khôi phục sau sự cố: RPO / RTO ghi rõ cho từng cách triển khai; diễn tập khôi phục trong CI cả trường hợp
      nhiều khách có database riêng và kho file
- [x] Rà giấy phép thư viện: `cargo deny` chặn giấy phép không hợp MIT/Apache; danh sách giấy phép của bên thứ ba đi kèm
      mỗi bản phát hành (binary, image, SDK) — `cargo deny` (đã có) + `THIRD-PARTY-LICENSES.html` bằng cargo-about (D-20261126-third-party-licenses)

### G14 — Vận hành và cam kết nền (chủ dự án duyệt ngày 26/11/2026, D-20261126-operations-baseline)

- [ ] Cam kết nền tảng: chính sách bản Postgres (các bản lớn còn được hãng hỗ trợ), bản SQLite tối thiểu, hệ điều hành và
      kiến trúc CPU cho binary và image; CI chạy ma trận các bản Postgres được hỗ trợ (ít nhất bản cũ nhất và mới nhất); ghi
      vào `docs/COMPATIBILITY.md` (EN + VI); `doctor` cảnh báo khi chạy trên bản ngoài danh sách
- [x] Bộ đồ nghề vận hành: sổ tay xử lý sự cố `docs/RUNBOOK.md` (EN + VI) cho các tình huống thường gặp (database đầy hay
      chậm, hàng đợi việc nền ứ, xoay khoá ký và khoá mã hoá khi lộ, thu hồi hàng loạt API key / phiên, khôi phục một khách
      từ sao lưu, khách báo mất dữ liệu); mẫu bảng theo dõi và luật cảnh báo dựng sẵn từ số đo OTLP / Prometheus của core;
      quy trình ứng phó lỗ hổng bảo mật (nhận báo, đánh giá, vá, công bố, xin mã CVE) ghi ở `SECURITY.md` — `docs/RUNBOOK.md`, thêm 3 luật cảnh báo vào `deploy/prometheus/autoxide-alerts.yml`, mục "How we handle a report" ở `SECURITY.md`
- [ ] Thử hỏng có chủ đích trong CI trên cả hai database: mất kết nối database giữa transaction, đầy ổ đĩa, Postgres
      chuyển máy dự phòng / khởi động lại, tiến trình bị giết giữa việc nền, migration và sao lưu; hệ thống tự hồi phục,
      không mất hay nhân đôi event / job, không hỏng dữ liệu; kết quả ghi vào tài liệu sức chứa
- [ ] Mở rộng danh sách hợp đồng được cam kết (bổ sung G13): tên lệnh, tham số và mã thoát của CLI `autoxide`; định dạng
      file sao lưu và file xuất dữ liệu (bản 1.x đọc được file do mọi bản 1.y trước đó tạo, có số phiên bản định dạng trong
      file); test giữ hợp đồng như api-diff
- [ ] Đường chuyển từ 0.x lên 1.0 cho người viết module: hướng dẫn từng bước (EN + VI) và công cụ tự sửa code cũ sang hợp
      đồng mới (dùng chính nó để chuyển module trong repo); chạy thử trên `examples/external-module`

## Giai đoạn A — Hợp đồng và nền lõi

Xong cả giai đoạn (33–37, 40): xem [`roadmap/done.md`](roadmap/done.md).

## Giai đoạn F — Core cho cộng đồng (làm trước, chủ dự án quyết ngày 21/11/2026, D-20261121-community-first)

## 70. Core cho cộng đồng: chạy phần mình cần, mở rộng không fork, cam kết ổn định

Thứ cộng đồng cần là lý do để tin và đường để đóng góp. Không dựng module mới trước bản 1.0, trừ bộ nối nhà cung cấp
thật (P7).

- [x] P1 — Bật/tắt module bằng cấu hình: module tắt không route, không có trong OpenAPI phục vụ lúc chạy, không tool
      MCP, không job, không nhận event, không chạy migration trên database mới; database đã có bảng của module tắt thì
      giữ nguyên dữ liệu; bật thiếu phụ thuộc thì báo rõ phải bật thêm gì; `doctor` liệt kê module bật / tắt;
      `docs/openapi.json` trong repo vẫn sinh với mọi module; test tập tối thiểu trên cả hai database
- [x] P2 — Module viết ngoài repo, không fork: crate `autoxide` dùng được như thư viện (binary của người ngoài = module
      chính thức + module của họ, đủ lệnh CLI); `examples/external-module/` là workspace riêng, CI build và test trên
      cả hai database; hướng dẫn `docs/guides/writing-a-module.md` + `.vi.md`; phụ thuộc bằng git tag dùng được ngay;
      `cargo publish --dry-run` qua cho kernel và testkit (đăng thật ở P5) — D-20261121-external-modules:
      `autoxide::Autoxide`, `cargo xtask check-module` (luật của gate trên crate ngoài repo), `cargo xtask
      publish-check` (đóng gói + build thử mọi crate sẽ đăng; `publish = false` giữ tới khi chủ dự án bảo đăng), job
      `semver` ở PR phát hành. Còn lại cho P4 / P5: rustdoc của kernel và testkit đang viết tiếng Việt (docs.rs cần
      tiếng Anh); một hàm gộp chuỗi migration bảng phụ trợ (outbox, job) cho module mới
- [x] P3 — Hai hạng module: chính thức (giữ ổn định) và thử nghiệm (ghi rõ "có thể đổi", không bật khi không đặt cấu
      hình); danh sách và phụ thuộc ở `docs/MODULES.md` + `.vi.md`; người nâng cấp có dữ liệu của module đang tắt được
      cảnh báo rõ ở log khởi động và `doctor`, kèm đúng dòng cấu hình cần thêm
- [x] P4 — Bản 1.0 có cam kết: chính sách tương thích cho API, mã lỗi, cấu hình, migration và phần kernel mở cho module
      ngoài; `cargo-semver-checks` trong bước phát hành; dòng hỗ trợ dài hạn (vá bảo mật bao lâu) ghi ở
      `docs/RELEASING.md` và `SECURITY.md` — D-20261123-compatibility-policy: `docs/COMPATIBILITY.md` + `.vi.md`;
      `cargo xtask semver` kiểm cả `autoxide` và module core / official, `--changed`; job CI `semver` ở PR chạm phần
      công khai và PR phát hành (cảnh báo trước 1.0, chặn từ 1.0 — chủ dự án thêm check bắt buộc lúc phát hành 1.0);
      LTS: minor trước vá bảo mật 3 tháng, major trước 6 tháng
- [ ] P5 — Đăng gói: SDK TypeScript lên npm, kernel và testkit lên crates.io, tự động lúc phát hành — **ghi nợ, làm
      sau** (D-20261121-vn-providers); cần tài khoản npm và crates.io của chủ dự án
- [x] P6 — Rà soát bảo mật toàn bộ trước 1.0 bằng một phiên AI độc lập (chưa thuê bên ngoài, D-20261121-vn-providers);
      làm sau khi P1–P4 đã vào `main`; `SECURITY.md` ghi đúng đây là rà soát nội bộ có AI hỗ trợ, không phải kiểm toán
      của bên thứ ba — báo cáo `docs/security/review-2026-11.md`, D-20261123-security-review-2026-11; 3 mức cao, 8 mức
      trung bình và các mức thấp rẻ đã sửa (PR #444, #450, #452, #454, #456, #461, #464, #473). Nợ bảo mật sau rà
      soát 2026-11 (mức thấp, lý do để sau ghi trong báo cáo):
  - [x] L-7: đổi mật khẩu (không phải khôi phục) giữ API key, passkey, liên kết đăng nhập ngoài — thêm lựa chọn "thu
        hồi mọi thông tin đăng nhập" hay liệt kê chúng trong câu trả lời — xong (#508, D-20261125-password-change-revoke):
        `revoke_other_credentials`
  - [x] L-8: luật tự động hoá theo event nhận event của mọi tổ chức trong khách — cần event mang tổ chức thống nhất
        — xong (#512, D-20261125-event-org): `Delivered::org_id()`, luật nút con chỉ nhận event của nhánh mình
  - [x] L-9: CSP của trang tài liệu API chỉ cho đúng gói Scalar đã ghim (cần thử lại trên trình duyệt, D-320) — xong
        (#505, D-20261125-docs-csp-pinned): đã thử trên Chromium
  - [x] L-10: giới hạn số kết nối mở (hiện để reverse proxy làm) — xong (#505, D-20261125-max-connections):
        `AUTOXIDE_HTTP_MAX_CONNECTIONS`, quá thì chờ; kết nối im lặng đóng sau 30 giây
  - [x] I-2: giới hạn kích thước câu trả lời của nhà cung cấp do người vận hành cấu hình (AI, cổng kết nối) — xong
        (#505, D-20261125-provider-response-cap): `AUTOXIDE_PROVIDER_MAX_RESPONSE_BYTES`
  - [x] I-5: ghim image `gitleaks` của CI bằng digest — xong (#505, D-20261125-gitleaks-digest)
  - [ ] Phần chưa rà: SCIM, xác minh số điện thoại, định dạng file xuất / nhập / chuyển khách, đối chiếu đủ mọi tool
        MCP với REST, điểm thưởng / giới thiệu / chia thanh toán POS, `content` / `media` / `newsletter` / `analytics`
        ngoài các điểm đã thử
- [ ] P7 — Bộ nối nhà cung cấp thật ở Việt Nam (D-20261121-vn-providers), mỗi bộ nối một module nhỏ sau trait của core,
      viết theo tài liệu công khai trước, hạng thử nghiệm cho tới khi chạy thử bằng tài khoản thật:
  - [x] Nền chung: tài khoản nhà cung cấp theo từng khách, khách tự cấu hình qua API (mã hoá, không trả lại secret),
        webhook tìm đúng khách rồi kiểm chữ ký; hướng dẫn viết bộ nối khác
  - [x] Thanh toán: SePay
  - [x] Thanh toán: MoMo
  - [x] Vận chuyển: Giao Hàng Nhanh
  - [x] SMS: eSMS
  - [x] Hoá đơn điện tử: SePay eInvoice
- [x] P8 — Luồng bán hàng mẫu chạy trọn (D-20261121-shop-first-order): một bài test đi từ sản phẩm → giỏ → đơn →
      thanh toán → vận chuyển → hoá đơn điện tử → tin nhắn, qua máy chủ giả của các nhà cung cấp ở P7, trên cả hai
      database; hướng dẫn "mở một cửa hàng" `docs/guides/shop.md` + `.vi.md` làm theo được từ quickstart; chỗ vướng tìm
      ra thì sửa hoặc ghi thành mục — `crates/app/src/e2e/shop.rs`; sửa kèm (D-20261122-shop-flow-fixes): báo phí mang
      người nhận và kích thước kiện, cổng thanh toán mặc định theo khách, `provider.not_configured` là "kênh chưa bật"
  - [x] Chỗ vướng còn lại (tìm ra ở P8): người mua tự mở lại lượt thu khi lần đầu hỏng (vd thiếu `return_url` của
        MoMo) hay link hết hạn — vãng lai bằng token xem đơn (`POST /public/orders/{id}/payment-link`, header
        `Order-Token`), đã đăng nhập bằng `POST /me/orders/{id}/payment-link`; cùng logic với
        `POST /{id}/payment-link` của nhân viên, `return_url` mới tuỳ chọn, chống trùng theo `Idempotency-Key`, tối đa
        10 lần một giờ mỗi đơn. Đặt hàng khi đối tác từ chối vẫn trả 201 với `kind = none`, thêm
        `payment_actions[].error` (`code` ổn định + `message` theo ngôn ngữ người gọi)
- [ ] P9 — **Ghi nợ, hoãn ngày 22/11/2026, chủ dự án mở lại ngày 25/11/2026** (D-20261125-p9-reopened; làm trước 1.0,
      trừ mục chạy thử bằng tài khoản thật):
  - [x] Chế độ "nghỉ": module tắt vẫn tham gia dữ liệu cá nhân và vòng đời khách (xuất, xoá, dời khách) —
        D-20261125-dormant-modules: hook `Module::dormant`, xoá / xuất / dời / tách / gộp khách gom cả bảng của module
        tắt; `files` tắt vẫn mở kho từ cấu hình để xoá nội dung file, `conversations` tắt xoá ghi âm qua kho và bộ nối
        thoại đang bật
    - [ ] Module tắt: dọn dữ liệu theo thời hạn lưu (job của module chưa chạy khi tắt)
    - [ ] Module tắt: gieo dữ liệu khởi đầu cho khách tạo trong lúc module tắt
    - [ ] `copy` / `backup create` / `backup restore` gồm cả bảng của module tắt (hiện từ chối khi còn dữ liệu)
    - [ ] `conversations` đang bật mà bộ nối nhà cung cấp thoại tắt: `erase_recording` bỏ qua xoá ở nhà cung cấp nhưng
          vẫn xoá `provider_recording_id` (bản ghi nằm lại không dấu vết) — làm như đường "nghỉ": báo lỗi, thử lại
    - [ ] `forms` tắt: job dọn file CSV xuất theo `expires_at` không chạy (cần xác minh; thuộc mục dọn theo thời hạn lưu)
    - [ ] Kế hoạch tách / gộp (dry run) đếm cả dòng của module tắt chỉ có bảng ở nguồn (dời đã đếm)
    - [ ] Test chiều ngược cho tách / gộp, test lệch phiên bản migration của module tắt giữa hai database
    - [ ] Test dry run dời khách không tạo bảng ở đích
  - [x] Mẫu câu OTP khách tự sửa cho khớp template đã đăng ký với nhà mạng (đang viết cứng ở `compliance`, `orders`,
        `bookings`) — D-20261125-sms-templates: cấu hình theo khách nhóm `sms_template`, kiểm `{code}` đúng một lần
  - [x] 139 endpoint GET trả `Data<Vec<T>>` không phân trang: endpoint mới trả `Page<T>`, khai deprecation cho cái cũ —
        chủ dự án quyết sửa tại chỗ cả 140 (đo lại 25/11), D-20261125-list-pages; làm theo cụm module, mỗi cụm một PR;
        xong ở D-20261126-list-pages-done:
    - [x] Kernel (`NaturalOrderPageQuery`, `SortChoicePageQuery::into_request`, `page_of`) + `support`
    - [x] Các module còn lại (theo cụm) — cụm 1 (12 module, 25 endpoint), cụm 2 (plans, tax, organizations,
          analytics, kernel `/me/*`), cụm 3 (auth, catalog, compliance, orders, promotions, kernel; cuối cùng
          `kernel/records/{kind}/{id}/usages` qua `ReferenceSource::referrers_page`)
    - [x] `customers/me/wishlist` và `notifications/templates`: thêm cột id UUIDv7 + migration điền dữ liệu rồi phân trang
    - [x] Luật gate chặn danh sách không phân trang (`api-unpaged-list`, `api-page-params`, `api-param-description`,
          `mcp-page-count`), hướng dẫn nâng cấp EN + VI (`docs/guides/list-pages-upgrade.md`), danh sách ngoại lệ — ba
          tra cứu theo khoá client gửi (`catalog/variants?code=`, `comments/public/counts`,
          `inventory/public/availability`) được cho phép (chủ dự án giao phiên điều phối quyết, 26/11/2026,
          D-20261126-list-pages-done mục 2)
  - [ ] `./gate.sh` chưa chạy tsc trên SDK và ví dụ trong `docs/guides` (CI có: `cargo xtask sdk --extract-examples` + `tsc
        --noEmit`); `sdk --check` chỉ so SDK với `docs/openapi.json` nên kiểu SDK hỏng vẫn lọt tới CI (#517). Chạy khi SDK
        hay ví dụ đổi và có Node; còn hai kiểu `& Record<string, unknown>` có sẵn trên `main` (`realm`, một kiểu tiền)
  - [x] Rustdoc tiếng Anh cho phần công khai của kernel, testkit, `autoxide` và `api` của module (docs.rs); gate
        `doc-language` giữ phần viết mới là tiếng Anh (D-20261123-english-rustdoc)
  - [ ] Chạy thử năm bộ nối của P7 bằng tài khoản thử thật; đối chiếu các điểm "(?)" của MoMo với trang tài liệu gốc
- [x] P10 — Trải nghiệm người viết module (dev và AI agent viết module không phải chép code của core):
  - [x] Bản giả dùng chung của trait module: `autoxide_ledger::api::testing::FakeLedger` (feature `testing`) thay sáu
        bản chép tay ở test của các module thương mại; quy ước `api::testing` cho bản giả khác
        (D-20261122-module-test-fakes); theo cùng quy ước: `autoxide_notifications::api::testing::FakeNotifier` thay
        bảy bản, `autoxide_compliance::api::testing::FakeConsentLedger` thay năm bản; `autoxide_testkit::FakeMailer` thay
        bảy bản, `autoxide_address::api::testing::FakeAdministrativeUnits` thay năm bản; `autoxide_media::api::testing::FakeMedia`
        thay bảy bản; `autoxide_catalog::api::testing::FakeCatalog`
        thay năm bản
  - [x] Hàm gộp migration bảng phụ trợ của kernel (outbox, job) cho module mới: `helper_tables`
        (D-20261122-module-helpers), dùng trong `examples/external-module/` và hướng dẫn
        `docs/guides/writing-a-module.md` + `.vi.md`; `notes` giữ danh sách cũ (đổi là đổi tên migration đã áp dụng)
  - [x] Bản mẫu khởi tạo repo module ngoài bằng một lệnh: `cargo xtask new-module --external` sinh workspace riêng chạy
        được ngay (module, migration, route có OpenAPI, test `TestApp::builder` trên cả hai database, `check.sh`, CI
        mẫu, README EN + VI, `AGENTS.md`); `./gate.sh --only example` sinh khung rồi lint và test nó
        (D-20261122-external-template)
- [x] P11 — Dùng được mọi thứ đã dựng: mọi endpoint trong `docs/openapi.json` và mọi tool trong `docs/mcp-tools.json`
      được test end-to-end gọi thành công ít nhất một lần (chạy trên cả hai database); mọi hướng dẫn trong
      `docs/guides/` làm theo được đúng như viết; chỗ hỏng tìm ra thì sửa tận gốc kèm test (D-20261123-api-coverage)
  - [x] Công cụ đo: testkit ghi endpoint / tool gọi thành công khi đặt `TEST_API_COVERAGE`; `cargo xtask coverage-api
        --run` báo phần chưa phủ theo module
  - [x] Hướng dẫn: mọi lệnh `curl` trong `docs/guides/` (cả khối không chạy thật) gọi endpoint có trong hợp đồng, đúng
        method, thân JSON chỉ dùng trường có thật — `e2e/guides_endpoints.rs`
  - [x] Luồng e2e theo nhóm (`crates/app/src/e2e/usable/`): nền, auth, kernel, gói, thông báo, webhook, file; thương
        mại; nội dung và AI; chăm sóc khách, đặt lịch, tự động hoá, phân tích; tuân thủ; bản nháp AI; chứng từ của đơn
        vãng lai (tra đơn bằng mã SMS qua eSMS giả). Đo 23/11: 1319/1319 endpoint, 312/312 tool (100%). Chỗ chưa có
        bộ nối thật thì phủ bằng bộ giả của testkit qua đúng trait module dùng: gọi điện (`FakeVoiceProvider`), phiên
        KYC với nhà cung cấp (`FakeIdentityVerifier`). Phễu analytics: chưa module nào khai phễu nên bản cài thật có
        danh sách phễu trống; tool `analytics.funnel` được phủ bằng phễu của test module `analytics`
- [x] P12 — Thử phá có chủ đích: fuzz / property test cho chỗ nhận dữ liệu từ ngoài, tiền và trạng thái
      (D-20261123-property-fuzzing): `proptest` trên stable trong gate (256 ca), workflow `property.yml` chạy hằng đêm
      20 000 ca, không chặn PR; lỗi tìm ra sửa tận gốc kèm ca tái hiện
  - [x] Webhook và chữ ký: SePay (sửa: timestamp `i64::MIN` làm tràn số → panic), MoMo, GHN, Standard Webhooks;
        token của form chống spam; bộ lọc SSRF (mọi cách viết IPv4 nội bộ)
  - [x] Tiền, thuế, khuyến mại; state machine đơn hàng (sửa: `tax` dồn phần dư làm tròn vào tầng không chịu thuế,
        số lượng 28 chữ số làm `Decimal` tràn → panic; `promotions` cộng tiền dòng bão hoà tràn `i64` → panic)
  - [x] Token (access token, API key, bearer, `id_token` của OIDC), bộ lọc SCIM, ALTCHA, header (Accept-Language,
        If-None-Match, Range) — sửa: claim thời gian cực biên của OIDC tràn `i64` → panic. Idempotency key không có
        hàm thuần riêng (middleware chỉ kiểm độ dài và `HeaderValue`), đã có test tích hợp
  - [x] Link an toàn của khối trang và thân bài, địa chỉ nhúng: so với bộ phân tích URL chuẩn WHATWG (cách trình
        duyệt hiểu) — không tìm ra lỗi
- [x] P13 — Chuỗi cung ứng và phát hành (D-20261123-supply-chain): ký không khoá (Sigstore qua OIDC của GitHub Actions)
      image và mọi file đính kèm — binary Linux amd64 / arm64 (chép từ image), gói SDK, SBOM — kèm chứng thực nguồn gốc
      build (SLSA) cho từng thứ, `SHA256SUMS` ký bằng cosign; hướng dẫn kiểm `docs/VERIFYING.md` + `.vi.md`; workflow
      tối thiểu quyền, mọi checkout `persist-credentials: false`, job CI `workflows` (zizmor + actionlint); Dependabot
      cho cargo, npm, github-actions, docker có nhóm và `cooldown`; giai đoạn bản thử `X.0.0-rc.N` (pre-release, image
      không `latest` / `X.Y`, bản vá dòng cũ không giành `latest`) và tiêu chí từ RC lên 1.0 ở `docs/RELEASING.md`;
      `SECURITY.md` + `.vi.md` thêm chính sách công bố phối hợp
- [x] P14 — Pentest hộp đen có uỷ quyền trên bản chạy thật (release, SQLite và PostgreSQL, nhiều khách, mọi module):
      IDOR và vượt khách, leo thang quyền, xác thực, injection, XSS, SSRF, race, DoS rẻ, lộ thông tin; báo cáo phần
      "Black-box test" của `docs/security/review-2026-11.md`. Không lỗ vượt khách hay mất tiền; đã sửa: API key tạo
      token SCIM / nhà cung cấp đăng nhập (#483), mã SMS đặt lịch không giới hạn theo địa chỉ (#487), người mua đọc
      được nhãn nội bộ (#489). Nợ mức thấp:
  - [x] P-4: tên miền khách khai mà không xác minh chặn khách khác 7 ngày — cho nhiều khách cùng chờ, ai xác minh
        trước được (đổi unique index), hoặc endpoint cho nhân sự nền tảng thu hồi — xong (#512, D-20261125-domain-claims):
        nhiều khách cùng chờ, unique index chỉ trên dòng `verified` (migration `m0040_domain_shared_claims`)
  - [x] P-5: `return_url` của thanh toán nhận mọi địa chỉ http(s) — chỉ nhận tên miền đã xác minh của khách (cần
        quyết danh sách địa chỉ hợp lệ) — xong (#508, D-20261125-return-url-allowlist): tên miền đã xác minh (https)
        hoặc origin khai ở `ledger.payments.return_origins`
- [x] P15 — Job `semver` của CI chạy xong trong giới hạn trước `1.0.0-rc.1`: ở PR phát hành `v0.19.0` (#495) job so
      mọi crate hợp đồng với tag trước và bị huỷ sau 90 phút (`timeout-minutes`), nên `CHANGELOG.md` của 0.19.0 thiếu
      mục `### Breaking (Rust)`. Từ rc.2 job này chặn phát hành (`docs/RELEASING.md`), nên phải đo thời gian từng
      crate rồi sửa tận gốc (chia crate ra nhiều máy như gate, dùng lại build baseline, hay cache rustdoc JSON) thay vì
      chỉ nới giới hạn; xong thì chạy lại cho `v0.18.0...v0.19.0` để biết 0.19.0 có phá vỡ gì. Xong: thời gian nằm ở
      bước dựng rustdoc JSON (công cụ dựng mỗi crate trong target dir riêng); nay một target dir chung cho mọi crate và
      chia 4 máy (`--shard k/4`), job gộp giữ tên check cũ; `v0.18.0` → `v0.19.0` không có phá vỡ phía Rust
      (D-20261125-semver-job-time)
- [ ] P16 — Nợ còn mở sau khi trả nợ bảo mật mức thấp của rà soát 2026-11 (#505, #508, #512;
      D-20261125-security-debt-followups):
  - [ ] Event mang `org_id` ở mọi module có dữ liệu theo đơn vị (hệ quả của L-8, danh sách trong
        D-20261125-event-org): tới lúc đó luật tự động hoá của nút con không chạy với các event đó
  - [ ] L-11: `return_url` của xác minh danh tính (`compliance`, `POST /api/v1/compliance/kyc`) chưa qua danh sách
        địa chỉ hợp lệ như thanh toán (P-5) — phát hiện khi sửa P-5, để cho lượt rà soát luồng xác minh danh tính
  - [ ] Test chắc chắn đi qua nhánh "trùng unique index" khi xác minh tên miền (`check_on` → `lost_race`): hiện chỉ
        test đồng thời đi qua nhánh đó khi hai lượt tình cờ xen kẽ
  - [ ] Cache `domain_site` (theo `Host`, giới hạn 10 000 mục, D-20261125-tenant-cors) bị dội `Host` ngẫu nhiên
        thì bị xoá sạch liên tục, tên miền thật phải đọc lại database mỗi lần — cần cách bỏ mục tốt hơn (LRU hay
        ảnh chụp như CORS)
  - [ ] `http/ws.rs` không kiểm `Origin` (mức thấp: xác thực bằng header `Authorization`, trình duyệt không gửi
        được khi mở WebSocket) — kiểm cùng nguồn `OriginGrant` như MCP
  - [ ] Test cho lượt đọc ảnh chụp tên miền verified bị treo quá `READ_TIMEOUT` (3 giây,
        `tenancy::verified_domains`, D-20261125-tenant-cors): cần database giả treo được
  - [x] Chủ dự án quyết: `PUT /api/v1/auth/users/{id}/roles` (`auth.roles.manage`) cho tự gán mọi role kể cả `admin`
        — chặn (PT-9, D-20261126-no-self-grant): gán role và sửa role theo "chỉ cấp cái mình có", kể cả cho chính mình
  - [ ] Các chỗ bất nhất còn lại của luật cấp quyền (chưa đổi hành vi): (i) `require_covers` (xoá / khoá / đổi
        email / nhận về tài khoản) so theo tên permission, bỏ qua phạm vi, còn `require_grantable` so cả phạm vi;
        (iii) cùng một luật ánh xạ: tạo vượt quyền trả 422, xoá vượt quyền trả 403; (iv) hạ quyền người khác bằng cách
        thu hẹp permission của role họ giữ không qua luật "gỡ được" như gỡ role (sửa role chỉ chặn thêm / nới / bỏ cấm)
  - [ ] `automations`: `run_as_limit` chỉ giới hạn theo TÊN permission (`Principal::only` giữ phạm vi hiện tại của chủ
        luật) — chụp cả phạm vi lúc tạo (D-20261126-automation-owner-revoked)
  - [ ] `automations`: event `auth.access_revoked` vào dead-letter thì luật không tạm dừng và không có đường dự phòng
        (runner chỉ dừng lần chạy khi luật đã `paused`)
  - [ ] Gỡ role có lệnh cấm đòi người gỡ `allows(p)` theo TÊN permission, bỏ qua phạm vi; khách có guardrail: role cấm
        một permission ngoài rào thì có thể không ai trong khách gỡ được role đó (cần kiểm chứng)
  - [ ] Nhóm SCIM chưa có luật chứa "thành viên chờ": ai giữ token SCIM cũng đưa người vào được; admin đặt luật
        thì mọi thành viên nhận role (response tạo luật liệt kê ở `granted_user_ids`, hướng dẫn dặn xem trước —
        D-20261125-scim-grant-cover). Chặt hơn: ghi người đưa vào (`added_by`) ở thành viên nhóm và chỉ cấp cho
        thành viên mà người đưa vào cấp được role, còn lại chờ duyệt — cần migration, để sau
  - [ ] Hạn mức số nhân sự: hai lần đổi role thật cho cùng một người chạy đồng thời có thể làm lệch chỗ (dùng chung
        mã giữ chỗ `auth.member:<id>`; có từ trước) — khoá dòng người dùng quanh giữ chỗ → đổi → xác nhận, hay job
        đối soát số nhân sự với `auth_user_role` (D-20261125-scim-sync-report)
  - [ ] SCIM: khi đồng bộ đưa một tài khoản lên role cao hơn người đã tạo liên kết SCIM của nó (token quyền thấp tạo
        tài khoản, token của admin cấp role cao), thu hồi mọi cách vào cũ như khi đổi email — cần ghi người tạo liên kết
        ở `auth_scim_users` (migration) (D-20261125-scim-sync-report mục 7)
  - [x] `automations`: luật chạy bằng quyền hiện tại của `run_as` — luật tạm dừng khi tài khoản bị cắt cửa vào
        (event `auth.access_revoked`) và không có hơn quyền lúc tạo (D-20261126-automation-owner-revoked)

- [ ] P17 — Sửa lỗi đợt test bảo mật + hiệu năng độc lập 2026-11 (29.584 case, cả hai database; PT-1..3 ở #515,
      PT-9 ở P16, PT-19 phiên khác):
  - [x] PT-4: giới hạn theo địa chỉ đếm theo một IPv4 hay một khối /64 IPv6 qua `throttle::address_key`, luật gate `address-key`
        (D-20261125-address-keys)
  - [x] PT-5: số tiền / số lượng của client không làm tràn `i64` (POS, kho, điểm thưởng, COD, chia kiện, nhận hàng
        trả, cân nặng, bậc phí, GHN)
  - [x] PT-6: tiền khác tiền tệ của bản ghi không bị ghi như tiền của nó (thu và đếm két POS, tín dụng cửa hàng, dòng
        bảng kê COD, khoản thanh toán của đơn)
  - [x] PT-7: mã đăng nhập SMS nhập sai đếm theo số + client (IPv4 / khối /64) người xin mã, trần chung 5× (D-20261125-otp-wrong-by-block)
  - [x] PT-8: quyết định quyền theo đơn vị đòi đơn vị có trong cây của khách; `OrgTree::contains` / `ensure_exists`
        (D-20261125-org-must-exist)
  - [x] PT-10: push subscription chỉ nhận máy chủ công khai; hợp đồng `WebPushSender` đòi bộ gửi chỉ nối IP công khai
  - [ ] Nợ của rà soát PT-5/6/8 (mức thấp, cần dữ liệu cực đoan do quản trị đặt hay chưa có đường khai thác):
        `orders/invoicing.rs` giảm giá × số lượng, `promotions/compliance.rs` giá × bp với giá rất lớn — cần trần giá
        chung của catalog; điều khoản trả trước (`orders/advance.rs`) không so tiền tệ với đơn; phiếu nhập kho theo đơn
        mua không so tiền tệ với đơn mua; tiền tệ của ca POS lấy từ `opening_cash` thay vì tiền của cửa hàng;
        sandbox nhận `role_id` không có mà vẫn 201 (kernel không biết role — nay chỉ không uỷ quyền, cần trait vai trò
        để báo 422 lúc tạo)
  - [ ] Bản ghi đã gắn `org_id` không có trong cây từ trước PT-8: vẫn hiện trong danh sách của người có quyền toàn
        khách nhưng mở / sửa / xoá theo ID bị 404 / 403 — cần lệnh báo cáo và dọn (gắn về nút gốc hay nút đúng)
  - [ ] `orders.payment_currency_mismatch` bỏ qua kiểm khi `PosPayment.currencies` rỗng (chỉ REST điền) — thêm
        đường gọi mới (tool MCP bán ở quầy) dễ quên; nên bắt buộc tiền tệ trong kiểu dữ liệu của service
  - [ ] Bậc phí vận chuyển, cân nặng gửi GHN bão hoà về `i64::MAX` thay vì báo 422 — cần trần cân nặng / kích thước
        ở đầu vào (như catalog 1e7 mm)
  - [ ] Khoản thanh toán khác tiền tệ của đơn (`orders/src/payments.rs`, sau đối soát chấp nhận khoản nhận) nay vào
        dead-letter — chưa có test, chưa có hướng dẫn xử lý cho người vận hành
  - [ ] Luật `address-key` của `cargo xtask check` chỉ bắt `format!("…ip:{…")` trên một dòng; khoá dựng cách khác
        (`ip.to_string()`, nhiều dòng) lọt — cân nhắc kiểm theo kiểu (`IpAddr` vào `Throttle::hit`)
  - [x] PT-7: theo NIST SP 800-63B-4 — mã sống tối đa 10 phút; lần sai liên tiếp đếm bền (client lạ khoá từ 90, mọi client từ
        100; quản trị viên mở khoá qua REST / MCP / CLI);
        trần chung là setting riêng thay cho 5×; số mã mỗi giờ theo cặp (số, client); client đã đăng nhập bằng số không
        bị trần "mọi client" khoá
        (D-20261126-otp-nist)
  - [ ] Mã đăng nhập SMS: token thiết bị có ký, cấp khi đăng nhập đúng, để nhận client quen thay cho địa chỉ (CGNAT di
        động đổi địa chỉ) — cần đổi hợp đồng `PhoneCodes` của `auth` (D-20261126-otp-nist, rủi ro còn lại)
  - [x] PT-13, PT-14: lớp kiểm permission chung của kernel đọc `x-permissions` (403 trước validation / ETag / tra bản
        ghi), 18 `x-permissions` thiếu đã bổ sung, hai test e2e giữ (D-20261126-permission-gate)
  - [x] PT-16, PT-17: slug tự sinh tìm theo nhóm ứng viên `slug IN (…)` dùng unique index (content, catalog); bộ
        phát event và worker xét lệnh dừng giữa hai lượt cả khi còn việc (tắt nhanh sau đợt tải)
  - [x] PT-15, poll outbox: tìm công khai có trần số lượt cùng lúc, theo địa chỉ và thời gian; lượt rảnh của bộ phát
        event và worker còn một câu `EXISTS` mỗi database (D-20261126-idle-polling)
  - [ ] PT-15 triệt để: chỉ mục tìm chữ chạy trên cả hai database (FTS5 / `tsvector` sau một trait) thay cho
        `LIKE '%q%'` quét mọi bài đã đăng — các chốt hiện có chỉ giữ máy chủ không nghẽn, mỗi lượt tìm vẫn quét
  - [ ] PT-11, PT-12, PT-18 và chỗ vướng cho việc test (bộ mồi dữ liệu thương mại, tool MCP trỏ operationId REST,
        đếm câu SQL theo request) (PR 4, sau #516)

## Giai đoạn B — Lớp cho giao diện và phân quyền

## 51. Hợp đồng cho khung quản trị và mặt tiền (thiết kế giao diện 2, D-200)

Đặc tả: [`docs/steps/51-ui-contracts.md`](steps/51-ui-contracts.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).
Thiết kế: [`docs/research/ui-design.md`](research/ui-design.md).

Giao diện (repo riêng, D-097) dựng menu, hộp việc, tìm kiếm, trang tổng quan từ dữ liệu của core để
cùng một giao diện phục vụ mọi mô hình kinh doanh. Module khai phần của mình (mặc định rỗng, D-038);
kernel gom và lọc theo khách, gói, permission ở nút đang chọn. Kernel không có danh sách phân hệ viết cứng.
Bước 41–49 khai phần của module mình ngay trong bước đó nếu phần tương ứng của bước 51 đã xong.

- [x] Nhúng bản ghi của module khác vào nội dung (P6): node `embed` (`kind`, `id`) của `content`, kiểm qua
      sổ đối tượng (D-123) khi lưu, đọc công khai trả tiêu đề, tóm tắt, đường dẫn, ảnh của đối tượng còn
      công khai (`Resource.image` và `resolve_many` đã có, D-238). `content` không biết khái niệm sản phẩm
- [x] Tham chiếu ngược "đang dùng ở đâu" (D-267): module khai tham chiếu tới đối tượng khác qua sổ đối tượng
      (bài, trang, menu, sản phẩm trỏ tới tệp media…); `GET /media/{id}/usages` và tương tự cho loại khác — xong (18/11,
      D-376): `GET /kernel/records/{kind}/{id}/usages`, nguồn của `content` và `menus`; module thương mại khai khi làm
- [x] Sổ điều hướng (P1): `Module::navigation` (nhóm, mục, permission, tính năng, nguồn số đếm, thứ tự); nhóm
      trùng phải khai giống nhau; menu ẩn khi module chưa bật hay thiếu permission, **khoá (vẫn hiện) khi
      thiếu tính năng của gói**; nhãn theo i18n; ETag; tool MCP
- [x] Hộp việc (P2, P10): `WorkSource` tính từ dữ liệu, không bảng việc, không "Đã xử lý" (D-212); sắp theo
      gấp, hạn; số đếm cho menu (cache có khách trong khoá); nguồn chậm không hỏng hộp; khác hộp thư của
      `notifications`; khoá ổn định, "vì sao bạn thấy", hoãn / bỏ hoãn, giao; `SetupStep` đánh dấu bước chặn bán
- [x] Tìm (P3, P9): `FinderSource` gom theo loại, không dấu vẫn tìm, chỉ nguồn người gọi có quyền, áp phạm vi
      nút; toán tử `is:`, `status:`, so sánh, phủ định; phạm vi `node | subtree | accessible`; `resolve` đổi
      mã (số đơn, mã vận đơn, ID) thành bản ghi người gọi được xem
- [x] Số liệu tóm tắt (P4): `SummaryMetric` (tiền, số đếm, tỉ lệ) theo phạm vi nút, số nặng từ bảng dẫn xuất;
      `definition` (i18n key, mặc định `None`) trả đã dịch, gate kiểm mọi số liệu khai và có bản dịch (D-207)
- [x] Hoạt động (P5, P13): câu đọc được theo i18n key trên `EventInfo`, chỉ bản ghi người gọi đọc được ở nút
      của nó; dòng hoạt động và ghi chú nội bộ trên mọi bản ghi theo loại của sổ đối tượng (D-123), nhắc @
      có thông báo; một chỗ dùng chung thay cho mỗi module tự làm ghi chú — xong (17/11): `/kernel/records/{kind}/{id}/
      notes|activity`; nhắc @ phát event `kernel.record_note_added`, `notifications` gửi tin trong ứng dụng (18/11, D-377)
- [x] Schema bộ lọc mỗi danh sách công bố trong OpenAPI (máy, agent dựng chip lọc) và view đã lưu `/me/views`
      (P7): tham số, sắp xếp, cột, nhóm, mật độ; phạm vi cá nhân / nút / khách; bộ lọc nội dung `review`,
      `scheduled`, `ai_generated`, `missing_locale`, `missing_alt`
- [x] Job và hàng loạt (P8, P15, P16; D-213): job API chung (danh sách, tiến độ, file, chạy lại có audit);
      module khai hành động hàng loạt, nhận `ids[]` hoặc `filter` + `expected_count`, `Idempotency-Key`, kết
      quả từng bản ghi, tiến độ qua SSE; xuất danh sách theo bộ lọc thành CSV qua job; job xong thì báo người yêu cầu
      (`kernel.job_finished` → tin trong ứng dụng, D-377)
- [x] Giải thích quyền (P11): `POST /me/can` nhiều cặp (permission, nút), lý do `missing_permission` /
      `module_not_enabled` / `feature_not_in_plan` / `quota_exceeded` — giao diện hiện vì sao bị khoá, ai cấp
      được, nâng gói mở gì — **một phần** (17/11): `POST /me/can` với `unknown_permission`, `module_not_enabled`
      (`required_module`), `missing_permission`, cùng hàm kiểm với request thật. Còn: `feature_not_in_plan` /
      `quota_exceeded`, `upgrade_to` — xong (18/11, D-376) với `feature` / `limit` trong câu hỏi. `grantors` (ai cấp được)
      làm cùng mục "giải thích và mô phỏng" của bước 54
- [x] Agent là người dùng thật (P12): API key `agent`; `ActorKind` chung cho audit, nhật ký, event (D-224);
      lệnh rủi ro (hoàn tiền, chi trả, khoá sổ, xoá dữ liệu) agent gọi trả `202 approval_required`, yêu cầu
      duyệt có hạn, đủ duyệt / từ chối / event / tool MCP, như nhau REST và MCP; việc chờ vào hộp việc — xong
      (17/11, D-365): `x-approval: "agent"`, `/kernel/approvals`, gửi lại kèm `Autoxide-Approval`; MCP từ chối agent
      và chỉ sang REST; (18/11) yêu cầu chờ vào hộp việc của người quyết được (nguồn `kernel.approvals`), tool MCP
      `kernel.list_approvals` / `kernel.decide_approval`
- [x] Cờ theo người dùng cho trợ giúp trong giao diện (P14, D-207): `/api/v1/me/flags` (`kernel_user_flags`),
      chỉ cờ của mình, hạn mức `AUTOXIDE_USER_FLAGS_MAX`, có trong bản xuất dữ liệu, xoá khi xoá tài khoản;
      `tenant_id` thêm ở bước 33 cùng mọi bảng
- [x] Danh sách lý do theo khách (P18, D-223); cài đặt nối ra ngoài có `last_used_at`, `last_error` và thử
- [x] Trường tự định nghĩa trên bản ghi (P19, D-235): module khai đích, khách thêm trường (kiểu, nhãn, bắt buộc,
      mức hiện `internal` / `public` / `buyer_input`), giá trị nằm trên bản ghi, lọc qua bảng index của module ghi
      cùng transaction, dùng chung cho xuất / nhập / tự động hoá; hạn mức theo gói; test trên cả hai database
      — xong phần lõi (17/11): `kernel_custom_fields`, `/kernel/custom-fields`, `CustomFields::validate` / `write_index` /
      `condition`, `content.entry` (giá trị, lọc `cf.<khoá>`); (18/11, D-373) cột `cf.<khoá>` trong xuất, mức `public` chép vào
      bản công khai của bài, hạn mức `kernel.custom_fields` theo gói, tool `kernel.list_custom_fields`. Cột `cf.` khi nhập
      thuộc bước 56; trường `personal_data` áp khi có loại bản ghi gắn với một người (khách hàng, đơn — bước 41–49)
- [x] Nhãn gắn bản ghi (P20, D-235): nhãn nội bộ dùng chung mọi loại bản ghi, lọc `tags_any` / `tags_all`, gắn
      hàng loạt, gộp nhãn, event cho tự động hoá — xong (17/11); gắn / gỡ hàng loạt (18/11): `POST /kernel/tags/{id}/apply`
      (tối đa 500 bản ghi, chọn theo bộ lọc thì lấy id bằng `dry_run` của thao tác hàng loạt)
- [x] Đổi câu chữ hiển thị theo khách (P21, D-235): chỉ câu module cho đổi và không gian `ui.*`, giữ chỗ điền, không
      đổi mã lỗi và câu pháp lý, đọc qua cache không query
- [ ] Nhập dữ liệu (P17, D-220): không làm ở kernel — `imports` (bước 56) thêm nguồn `autoxide` cùng cột với
      xuất chung và đích `content`, `media`, `newsletter` (người đăng ký chỉ khi có bằng chứng đồng ý)
- [x] Khai menu, nguồn việc, nguồn tìm, số liệu cho mọi module đang có; MODULE_GUIDE (EN + VI) mục "khai phần
      của module cho giao diện"; test rò rỉ giữa hai nhánh anh em cho số đếm và kết quả tìm

## 68. Menu của website (`menus`, chủ dự án quyết ngày 14/11/2026, D-238)

Đặc tả: [`docs/steps/68-menus.md`](steps/68-menus.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).
Đề bài: [`docs/prompts/menus-module.md`](prompts/menus-module.md).

Menu của website (thanh danh mục, bảng xổ lớn, chân trang) là module riêng: mục menu trỏ tới địa chỉ hay tới bản ghi
của mọi module qua sổ đối tượng. Không nhầm với sổ điều hướng (P1 của bước 51), là menu của trang quản trị. Phần
chính làm trước bước 33 theo lệnh của chủ dự án; phần theo khách, theo website làm cùng bước 33, 34.

- [x] Kernel mở rộng sổ đối tượng theo D-038: `Resource.image`, `ResourceRef`, `resolve_many` (tra theo lô, theo
      ngôn ngữ), `has_children`, `children`; `content` đăng ký `content.term` (có cây) và tra `content.entry` theo lô
- [x] Module `menus`: menu theo `key` và ngôn ngữ; mục trỏ `url` / đối tượng / nhóm tiêu đề; tự lấy nhóm con có giới
      hạn; biểu tượng, làm nổi, nhãn có hạn, ô gợi ý; 4 tầng, 200 mục; lỗi gom một lần với mã riêng
- [x] API quản trị (`menus.read`, `menus.manage`, `If-Match` bắt buộc khi thay cây) và tool MCP dùng chung service;
      API công khai trả cây đã giải, bỏ đối tượng không còn công khai, thiếu ngôn ngữ thì dùng ngôn ngữ mặc định,
      `ETag` + cache trong bộ nhớ có thời hạn (rút tới mốc nhãn bật / tắt)
- [x] Chuyển từ menu của `content`: bốn endpoint cũ deprecated (sunset 2027-05-14), lệnh
      `menus.import_content_menus` chạy lại không nhân đôi, `menus` không đọc bảng của `content`
- [ ] Gỡ menu cũ của `content` khi qua 2027-05-14: bốn endpoint, trait `LegacyMenus`, bảng `content_menus` và
      `content_menu_items` (migration mới), mục menu trong `docs/CONTENT.md`

## 54. Phân quyền chuẩn hàng đầu (chủ dự án giao ngày 13/11/2026, D-205) — làm trước 1.0 (D-20261126-steps-52-54-before-1-0)

Đặc tả: [`docs/steps/54-authorization.md`](steps/54-authorization.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 34. Mô hình: RBAC theo cây tổ chức có độ sâu (bước 34) + chia sẻ bản ghi một bước + deny
và rào + điều kiện theo ngữ cảnh — luật là dữ liệu có kiểu, không ngôn ngữ chính sách tự do. Đối chiếu với
Google Zanzibar, AWS IAM, GitHub, Microsoft Entra, Salesforce, Shopify, Stripe và chuẩn OpenID AuthZEN.

- [x] Một hàm quyết định duy nhất, thu hồi tức thì (quyền không nằm trong token); test so khớp "lọc danh sách =
      kiểm từng bản ghi" sinh ngẫu nhiên trên cả hai database (D-20261118-step54-one-decision)
- [ ] Cache quyền theo phiên bản quyền của khách (P11): chỉ làm khi benchmark bước 39 cho thấy câu nạp quyền
      đáng kể (D-20261118-step54-one-decision)
- [x] Deny thắng allow trên role (D-20261118-step54-role-denies); rào quyền tối đa cho khách con, kể cả admin của
      khách con (D-20261119-step54-guardrails); API key chi tiết: hạn bắt buộc, dải IP, lần dùng cuối, xoay vòng
      (D-20261119-step54-api-key-limits), quản trị duyệt key (D-20261119-step54-key-approval)
- [ ] API key chọn bản ghi cụ thể (qua `kernel_shares` với `subject_kind = api_key`): làm cùng mục chia sẻ bản ghi
- [ ] Xác thực lại cho việc nhạy cảm (passkey / MFA gần đây, RFC 9470); điều kiện theo IP và khung giờ
      trên gán role — đã có: xác nhận lại cho việc nhạy cảm (D-20261119-step54-step-up)
- [ ] Chia sẻ bản ghi có hạn cho người, nhóm, nút, key (module khai quan hệ `viewer` / `editor`…), không
      trao quyền mình không có
- [ ] Quyền theo trường dữ liệu: trường cá nhân / nhạy cảm / tài chính phân loại trong OpenAPI, bị che khi
      thiếu `<module>.read_personal`, đọc dữ liệu nhạy cảm có audit (Luật 91/2025) — áp cho `auth`,
      `comments`, `forms`, `newsletter`
- [ ] Nâng quyền đúng lúc (role đủ điều kiện, kích hoạt có lý do, hạn, duyệt); tài khoản khẩn cấp; lệnh hai
      người và cặp quyền xung đột do module khai, ngưỡng là dữ liệu
- [ ] Giải thích và mô phỏng (vì sao có / không có quyền, thử role nháp), "ai có quyền trên bản ghi này",
      quyền không dùng 90 ngày, rà soát quyền định kỳ vào hộp việc
- [ ] Endpoint theo chuẩn OpenID AuthZEN Authorization API 1.0; agent hành động thay người dùng có vết
      `on_behalf_of`; số đo quyết định quyền; hướng dẫn `docs/ACCESS.md` (EN + VI)
- [ ] Khách duyệt từng lần khi nhân sự nền tảng xin vào dữ liệu (D-232): `per_request` mặc định, tối đa 4 giờ,
      duyệt qua yêu cầu duyệt, `legal_request` như D-211
- [ ] "Xem như" người dùng chỉ đọc thay cho đăng nhập thay (D-232): người mua phải cấp phép, không xem người
      quyền cao hơn, mọi lệnh ghi bị chặn kể cả qua MCP, không trả secret, audit hai id

## 52. Trải nghiệm quản trị nâng cao (D-201) — làm trước 1.0 (D-20261126-steps-52-54-before-1-0)

Đặc tả: [`docs/steps/52-admin-experience.md`](steps/52-admin-experience.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 51. Những điều nên có theo nghiên cứu các trang quản trị hàng đầu (Stripe, Shopify,
Linear, GitHub, Odoo); mỗi mục chạy và có test trên cả hai database.

- [x] Bàn kỹ thuật: nhật ký request theo khách (API key, đường dẫn mẫu, status, `code`, `request_id` /
      `trace_id`, có hạn lưu, không lưu thân request); lần giao webhook kèm gửi lại; event theo bản ghi đọc
      từ audit; chạy lại job trong dead-letter queue; `kernel.devtools.read` / `kernel.devtools.manage`
      - [x] `GET /webhooks/deliveries?status=`, `POST /webhooks/deliveries/{id}/redeliver` (+ tool MCP, event audit);
            `GET /kernel/devtools/events?object=` đọc từ audit; chạy lại job hỏng đã có từ P15 (`POST /kernel/jobs/{id}/retry`,
            event `kernel.job_retried` vào audit). Còn: tool MCP cho event theo bản ghi.
- [x] Nhật ký lệnh của agent (D-202): lệnh MCP ghi `mcp:<tên tool>`, không ghi tham số; lọc theo
      `actor_kind` (đủ `ActorKind`, D-224) và theo key; dòng của lệnh `202 approval_required` giữ `approval_id`
      - [x] Bảng `kernel_request_log`, ghi bất đồng bộ, `GET /kernel/devtools/requests`, tool `kernel.request_log`,
            lệnh MCP `mcp:<tool>`, `approval_id`, hạn lưu `AUTOXIDE_REQUEST_LOG_DAYS` (D-370)
- [x] Hoàn tác an toàn: lưu trữ / ẩn có `restore`; lệnh ghi lớn có `dry_run` trả trước những gì sẽ đổi
      - `POST /content/entries/{id}/unarchive`; media đã có `restore`, bình luận và người đăng ký bản tin có thao tác ngược
        (`approve` / `unblock`); `dry_run` ở thao tác hàng loạt (P8), nhập menu cũ. Nhập CSV (bước 56) và đổi giá hàng loạt
        (thương mại) thêm `dry_run` khi làm tới.
- [ ] AI trong quản trị (D-109): lời thường thành bộ lọc đã kiểm theo schema (`ai/filters/parse`); trợ lý chạy
      tool MCP với quyền người gọi, mọi lệnh ghi là bản nháp hay yêu cầu duyệt, trả kèm tool đã gọi
      - [x] `POST /kernel/filters/parse`: lời thường thành bộ lọc đã kiểm theo schema (D-372). Còn: trợ lý.
- [ ] Thông báo: lý do nhận, trạng thái xong / lưu, bộ lọc, cấu hình kênh theo loại event; ghim màn hình hay
      dùng (`/me/pins`)
      - [x] `/api/v1/me/pins`: ghim màn hình hay bản ghi, tối đa 20, xếp lại, dữ liệu cá nhân
- [ ] Sức khoẻ người bán và sức khoẻ tuân thủ: chỉ số so với ngưỡng (ngưỡng là dữ liệu có ngày hiệu lực), vi
      phạm, hạn phải xử lý; huy hiệu người bán tin cậy tính từ số liệu thật, công bố tiêu chí
- [ ] Quản trị trên điện thoại (D-232): gợi ý `mobile` trên sổ điều hướng (tối đa 5 mục thanh đáy); tóm tắt
      cuối ngày tự bật theo giờ của người dùng (số liệu, việc gấp, cảnh báo bất thường)
      - [x] Gợi ý `mobile` trên `NavItem`, `GET /kernel/navigation?device=mobile`, tool MCP nhận `device` (D-369)

## Giai đoạn C — Vận hành

## 39. Vận hành và hiệu năng (chủ dự án giao ngày 13/11/2026; số cũ 33, D-189)

Đặc tả: [`docs/steps/39-operations-performance.md`](steps/39-operations-performance.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Khi đã có người dùng thật: biết sớm khi hệ thống kẹt, không mất dữ liệu, xử lý sự cố không phải
đoán, và chịu được dữ liệu cỡ thật. Bắt đầu SAU bước 38: backup, số đo, cảnh báo
theo từng khách hàng và từng database (nền đa khách của bước 33); sau 38 để rà index phủ cả bảng của 33–38. Làm nhóm A trước: số đo tồn đọng dùng để quan sát lúc chạy tải ở nhóm B. Mọi số đo ghi kèm máy,
ngày, cỡ dữ liệu; mọi thay đổi chạy và có test trên cả hai database; không đổi tính đúng lấy tốc độ.

Nhóm A — vận hành:

- [x] Backup tự động theo lịch: job định kỳ (qua `Schedule` của kernel) tạo bản backup như
      `autoxide backup` (`crates/app/src/commands/backup.rs`) rồi ghi vào `Storage` (ổ đĩa hay
      S3), giữ N bản gần nhất, lịch và số bản cấu hình qua `Setting` (sinh lại `.env.example`);
      bản hỏng thì ghi log lỗi và đếm bằng metric. `autoxide doctor` cảnh báo khi bản mới nhất cũ
      hơn ngưỡng. Không giữ transaction ghi trong lúc backup (dùng `Database::begin_snapshot`)
- [x] Diễn tập restore: test đầu-cuối backup → restore vào database trống → chương trình khởi
      động, dữ liệu và file khớp, trên cả hai database (cả chép chéo SQLite ↔ Postgres nếu
      `autoxide copy` hỗ trợ); `docs/DEPLOY.md` (EN + VI) có quy trình restore từng bước
- [x] Số đo tồn đọng (gauge, cập nhật định kỳ, tên có tiền tố như luật code): job đang chờ và tuổi
      của job chờ lâu nhất theo module, job trong dead-letter queue, event chưa giao và tuổi của
      event cũ nhất, kết nối database đang dùng / còn rảnh. Hiện chỉ có bộ đếm cộng dồn
      (`jobs_failed_total`, `events_failed_total`…), không thấy được hàng đợi kẹt
- [x] Dashboard và cảnh báo có sẵn: file dashboard Grafana và luật cảnh báo Prometheus trong repo
      (tỉ lệ 5xx, độ trễ p95, job chờ quá lâu, dead-letter tăng, event tồn, hết kết nối database,
      backup quá cũ), mỗi cảnh báo kèm câu "làm gì"; hướng dẫn dùng trong `docs/DEPLOY.md` (EN + VI)
- [x] Xoay khoá không gián đoạn: `AUTOXIDE_AUTH_TOKEN_SECRET` (và các khoá ký khác) nhận thêm khoá
      cũ trong một khoảng chuyển tiếp — ký bằng khoá mới, kiểm bằng cả hai — để đổi khoá (định kỳ
      hay khi bị lộ) không đăng xuất mọi người; test; quy trình trong `docs/DEPLOY.md`
- [x] Sổ tay xử lý sự cố trong `docs/DEPLOY.md` (EN + VI): hàng đợi kẹt, job trong dead-letter
      (`autoxide jobs retry`), database đầy, lộ khoá, mất admin cuối cùng (`autoxide user …`),
      nâng cấp khi nhiều máy chủ chạy hai phiên bản code cùng lúc (migration theo kiểu mở rộng
      trước, thu hẹp sau). Mỗi tình huống: dấu hiệu, lệnh kiểm, cách sửa

Nhóm B — hiệu năng ở quy mô thật (bài đo hiện có chạy trên database gần như trống, chỉ đi qua
`notes` và `auth`):

- [x] Bộ dữ liệu lớn để đo: một lệnh (vd trong `xtask` hay `scripts/`) sinh nhanh dữ liệu cỡ thật
      trên cả hai database — vd 100 nghìn bài nhiều ngôn ngữ có danh mục, thẻ, tác giả; 50 nghìn
      ảnh kèm bản nhỏ; 1 triệu bình luận; vài nghìn người dùng — chỉ ghi qua service hay đúng định
      dạng bảng của module, không dùng cho dữ liệu thật
- [x] Đo tải đường đọc công khai trên bộ dữ liệu đó, mở rộng `scripts/loadtest.sh`: một bài
      (`/public/resolve`), danh sách bài và theo danh mục (`/public/posts`, `/public/terms/…`),
      `/public/sitemap`, `/public/feed`, `/public/search`, ảnh và bản nhỏ, bình luận của bài
      (`/public/threads`, `/public/comments`). Đặt ngưỡng p95 mỗi đường, ghi vào `docs/BENCHMARKS.md` — đo 19/11/2026: lỗi thống kê SQLite đã sửa (D-20261118-step39-sqlite-optimize); đường còn đỏ (lọc
      theo danh mục, tìm theo từ của `content`; danh sách đầy đủ trên SQLite) ghi ở BENCHMARKS, báo phiên giữ `content`
- [x] Rà index bằng `EXPLAIN` (SQLite `EXPLAIN QUERY PLAN`, Postgres `EXPLAIN ANALYZE`) cho mọi
      truy vấn danh sách, lọc, đếm của các module trên bộ dữ liệu lớn; index thiếu thêm bằng
      migration mới (không sửa migration cũ); ghi bảng truy vấn → kế hoạch trước / sau — xong cho bốn module có dữ liệu cỡ thật (`auth`, `content`, `media`,
      `comments`): không có index thiếu thật sự, không thêm migration; module khác chưa có bộ dữ liệu mẫu,
      D-20261120-step39-index-review
- [x] Nén response (gzip, brotli): đo cỡ và thời gian trên JSON thật (danh sách, `openapi.json`
      hơn 1 MB), rồi quyết nén trong app (`tower-http`) hay để reverse proxy làm; không nén lại
      file đã nén (ảnh). Ghi quyết định; sửa `docs/DEPLOY.md` (EN + VI) theo — nén trong app, mức 1, D-20261119-step39-compression
- [x] Cache cho CDN: `Cache-Control`, `ETag`, `Vary` đúng cho mọi endpoint công khai; file media có
      địa chỉ mang phiên bản thì cache dài hạn (`immutable`); nội dung riêng tư hay xem trước không
      bao giờ được cache chung; hướng dẫn đặt CDN trước Autoxide trong `docs/DEPLOY.md` (EN + VI) — lớp kernel `http/cache.rs`, D-20261119-step39-cdn-cache
- [x] Tìm theo ý nghĩa khi dữ liệu lớn: đo cách duyệt hết vector hiện tại trên bộ dữ liệu lớn so
      với ngưỡng 50 ms của D-110 (`docs/BENCHMARKS.md`, mục "Tìm theo ý nghĩa"); vượt ngưỡng thì
      chọn cách giảm (vector nhỏ hơn, chia nhiều nhân, hay index sau một trait) chạy được trên cả
      hai database, ghi quyết định; không vượt thì ghi số đo và giữ nguyên — vượt (137 ms); `dot` SIMD + chia nhân còn
      20 ms, D-20261119-step39-semantic-scan
- [x] Quan sát theo khách (D-190): log và trace mang trường `tenant_id`; **không** dùng `tenant_id`
      làm nhãn số đo (trái luật code về nhãn); lượng dùng theo khách lấy từ số đo dùng của bước 35
- [x] Ảnh AVIF (D-232): đo so với WebP trên bộ ảnh mẫu; chỉ thêm (job, tắt mặc định) khi nhẹ hơn ≥ 20% ở chất
      lượng tương đương, không thì ghi "không làm" kèm số đo — đạt (−34% trung vị), làm: job `media.make_avif`,
      D-20261119-step39-avif
- [x] Rút số câu SQL của `GET /auth/me`, `auth.whoami`, `GET /auth/users` (D-302: 3 → 6 câu cho một `GET /auth/me`,
      chậm 35–46%; bước 34 thêm một câu kiểm role bắt buộc MFA, D-317): lấy `has_password`, `last_sign_in_at` từ chính
      dòng người dùng đã đọc, gộp cờ MFA, role bắt buộc MFA và email đang chờ đổi vào một câu; đo lại bằng `scripts/loadtest.sh`, mục tiêu trong ±20% của mốc trước bước 33 — xong trước bước 39 theo lời chủ dự án: 7 → 3 câu, +172% (D-319)
- [x] Login throttling trên Postgres khi bị dò dồn dập một email (D-302: 8500 → 232 request/s cho lượt bị chặn): bộ
      đếm cập nhật một dòng bằng một câu thay vì nhiều câu `UPDATE` nối tiếp; trả 429 không cần khoá dòng khi đã bị khoá — xong trước bước 39 theo lời chủ dự án: lượt bị chặn vốn nhanh, số cũ là bài đo lẫn; đếm một câu (D-321)

## Giai đoạn D — Module nền dùng chung

Module trung lập với nghiệp vụ, không phải core: `support`, `analytics`, `automations`, `apps`; SCIM nằm trong `auth`.

## 53. Hỗ trợ khách hàng (`support`) (chủ dự án giao ngày 13/11/2026, D-204)

Đặc tả: [`docs/steps/53-support.md`](steps/53-support.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 37; song song được với 41–49. Phiếu hỗ trợ; chat thời gian thực là bước 61
(D-221). Khiếu nại có hạn luật định vẫn ở `compliance` (bước 37), tranh chấp ở `marketplace` (bước 46):
`support` chuyển phiếu sang đó, không làm lại.

- [x] Phiếu và tin nhắn: tạo từ website (có tài khoản; vãng lai qua OTP email / số điện thoại; từ token
      xem đơn của bước 45), nhân viên ghi hộ; trạng thái qua một máy trạng thái; tin nhắn chỉ thêm, ghi
      chú nội bộ có nhắc tên, đính kèm qua `media`; người mua vãng lai xem bằng token phiếu, gộp phiếu vào
      tài khoản sau khi đăng ký
- [x] Phạm vi theo nút tổ chức (bước 34): nhân viên thấy phiếu của nút mình và nhánh dưới; người bán trên
      sàn chỉ thấy phiếu gian hàng mình; nhóm nhân viên, quy tắc tự phân công là dữ liệu, chia vòng
- [x] Hạn trả lời (SLA) theo ưu tiên, tính theo giờ làm việc và ngày nghỉ của nút, dừng khi chờ người
      mua; job đánh dấu quá hạn; nguồn việc cho hộp việc (bước 51); báo cáo thời gian phản hồi, tỉ lệ quá
      hạn, mức hài lòng
- [x] Gắn phiếu với đơn, sản phẩm qua sổ đối tượng (D-123); câu trả lời mẫu có biến; chuyển phiếu thành
      khiếu nại người tiêu dùng của `compliance` (gọi lại không tạo hai vụ việc) hay lên sàn
- [x] Tool MCP (cùng permission như REST); gợi ý trả lời bằng AI chỉ là bản nháp (D-109); email trả lời
      vào lại phiếu qua trait `InboundMail` (tắt mặc định); hướng dẫn `docs/SUPPORT.md` (EN + VI); test
      trên cả hai database
- [x] Trả lời tự động câu hỏi đơn giản (D-230): chỉ gửi câu trả lời mẫu đã duyệt hay mẫu câu dựng từ dữ liệu
      chính xác (`FactSource` của module sau), mô hình chỉ phân loại; luật quyết trong Rust, không bao giờ tự động
      cho khiếu nại / hoàn tiền / dữ liệu cá nhân; tắt mặc định, nhãn và "gặp nhân viên"; câu phức tạp → nháp

## 57. Phân tích truy cập và hành vi (`analytics`) (chủ dự án giao ngày 14/11/2026, D-220)

Đặc tả: [`docs/steps/57-analytics.md`](steps/57-analytics.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 37. Số liệu truy cập ngay trong Autoxide, không gửi dữ liệu người đọc cho bên thứ ba.

- [x] Thu sự kiện từ website (endpoint công khai, gom lô, chống bot) và từ event của module; loại sự kiện do
      module khai; phiên ẩn danh không cookie với khoá xoay mỗi ngày, không lưu IP hay user agent đầy đủ
- [x] Gộp theo giờ / ngày, báo cáo chỉ đọc bảng gộp: tổng quan, trang, nguồn, chiến dịch UTM, đối tượng xem
      nhiều, phễu do module khai; số liệu cho trang tổng quan (bước 51) và "thịnh hành" cho `catalog`
- [x] Theo dõi theo người chỉ khi đồng ý (sổ đồng ý bước 37), rút đồng ý thì xoá phần gắn người; hạn lưu là
      cấu hình; tool MCP chỉ đọc; phạm vi theo website của nút; test trên cả hai database

## 58. Tự động hoá quy trình (`automations`) (chủ dự án giao ngày 14/11/2026, D-220)

Đặc tả: [`docs/steps/58-automations.md`](steps/58-automations.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 51 (lệnh chờ duyệt, job API). Luật "khi … thì …" như Shopify Flow; mỗi hành động là một tool
MCP đã có, chạy với quyền của người tạo luật.

- [x] Luật: kích hoạt bằng event, lịch hay gọi tay; điều kiện là cây toán tử cố định (không biểu thức tự do);
      hành động là tool MCP có tham số điền từ event; lưu luật kiểm event, tool, quyền, tham số một lần
- [x] Chạy bằng job ngoài transaction, nhận lại event không chạy hai lần, mỗi bước có `Idempotency-Key`; hành
      động rủi ro chờ duyệt; chống vòng lặp và giới hạn số lần chạy; nhật ký từng lần chạy, chạy lại
- [x] Chạy thử khô, danh mục kích hoạt và hành động người gọi dùng được, mẫu luật do module khai; agent tạo
      luật thì chờ duyệt; người tạo mất quyền thì luật tự tắt; test trên cả hai database
- [x] Cảnh báo bất thường (D-229): kích hoạt theo số liệu tóm tắt so với ngưỡng hay mức nền 4 tuần (doanh thu
      tụt, sắp hết hàng theo tốc độ bán, tỉ lệ bom hàng tăng), thống kê tất định, một lần báo mỗi kỳ

## 67. Trang dựng từ khối, AI tạo trang, bộ giao diện website (chủ dự án giao ngày 14/11/2026, D-229)

Đặc tả: [`docs/steps/67-page-builder.md`](steps/67-page-builder.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 51 và 37. Kéo thả là giao diện ở repo riêng; core cho dữ liệu trang và API. AI chỉ ghép khối có
sẵn, không sinh code; kết quả luôn là bản nháp.

- [x] Sổ khối của kernel: `Module::blocks()` (tên, nhóm, schema props, gợi ý cho AI, tính năng của gói),
      `BlockData` trả dữ liệu theo lô; `GET /kernel/blocks` chỉ khối của module đang bật
- [x] Trang có thân `layout` (cây khối JSON) cạnh thân `rich`: kiểm từ chối mặc định, theo cỡ màn hình, theo ngôn
      ngữ; sửa từng phần bằng JSON Patch + `If-Match`; nháp, lịch sử, xem trước, hẹn giờ dùng lại `content`
      (D-20261122-page-layout)
- [x] Khoá sửa trang (`POST/DELETE /entries/{id}/edit-lock`, giữ 2 phút, người khác thấy ai đang sửa; `If-Match` vẫn là
      chốt cuối), test đồng thời trên cả hai database (tách khỏi mục trên — D-20261122-page-layout)
- [x] Khối nền (bố cục, chữ, media, FAQ, thanh thông báo, đếm ngược, popup thu email có đồng ý), `content.embed`,
      `content.search_box`, `content.placement` (vị trí banner trong trang và đầu trang, D-20261120-step55-close);
      chân trang pháp lý `compliance.legal_footer` tự gắn dưới trang (D-20261122-base-blocks)
- [x] Khối của `forms`, `newsletter`, `comments`, `menus` (`menus.menu`, D-238); `media` không khai khối riêng —
      `content.image` / `gallery` / `video` dùng ID `media` (D-20261122-base-blocks)
- [x] Đọc công khai trả bố cục kèm dữ liệu khối, một lần gọi mỗi loại khối, khối lỗi không hỏng trang, ETag;
      đo p99 trang 50 khối trên cả hai database (D-20261122-layout-public)
- [x] Bộ giao diện của website (token màu, chữ, khoảng cách, kế thừa theo nút, kiểm tương phản) — D-20261122-theme-tokens
- [x] Khu dùng chung nhiều trang (D-20261122-shared-sections)
- [x] Thư viện trang mẫu theo ngành (D-20261122-page-templates)
- [x] Chủ đề theo mùa, ngày lễ, khung giờ (D-234, D-20261122-theme-variants): bản chủ đề ghi đè một phần theme gốc theo lịch (một lần, hằng
      ngày, hằng tuần, theo múi giờ của website), ưu tiên khi trùng, thư viện chủ đề mẫu, chép sang năm sau, xem
      trước ở một thời điểm; hiệu ứng trang trí chỉ trong danh sách cho phép, không chạy ở trang thanh toán
- [x] Đầu trang và chân trang của cả website (D-234, D-20261122-public-site): bố cục khối có nháp / đăng, theo ngôn ngữ, kế thừa theo nút,
      bản chủ đề kèm được bản riêng; `GET /public/site` một lời gọi cho khung website, cache tới đúng mốc đổi chủ
      đề, không query khi cache nóng; đo p99 trên cả hai database
- [x] Loại nội dung tự định nghĩa (D-235), phần chính: khách tạo loại (đội ngũ, dự án, chi nhánh…) với trường riêng
      của bước 51 (đích `content.type.<key>`), mục `kind = custom` dùng lại nháp / lịch sử / bản dịch / SEO, đường dẫn
      theo tiền tố, API công khai của loại (D-20261122-content-types)
- [x] Loại nội dung tự định nghĩa, phần sau: trang chi tiết dựng sẵn (`detail_layout`, khối `content.entry_field`),
      khối `content.type_list`, lọc `cf.<key>` ở danh sách công khai, AI gợi ý bộ trường (bản nháp), hạn mức theo gói
      (D-20261123-content-types-later)
- [x] AI tạo trang và sửa tiếp bằng trò chuyện (job, kết quả qua bộ kiểm, thử lại có giới hạn, bản nháp
      `ai_assisted`, lịch sử trò chuyện để hoàn tác) — D-20261122-ai-pages
- [x] Bộ nháp chiến dịch từ một chương trình khuyến mại (trang đích, banner, chiến dịch `newsletter`, mỗi thứ là bản
      nháp ở module của nó); hạn mức `content.ai_page_generations` theo gói — D-20261123-campaign-drafts
- [x] Kiểm tra sức khoẻ website vào hộp việc, chạy theo yêu cầu: thiếu mô tả SEO / alt, tiêu đề trùng, link gãy,
      bản ghi nhúng đã ẩn, đếm ngược đã qua (D-20261122-site-health)
- [x] Kiểm tra sức khoẻ website, phần sau: job hằng ngày mọi website, đếm đường dẫn 404 (bảng
      `content_not_found_paths`, không lưu IP) → gợi ý chuyển hướng; tool MCP như REST cho khối, bố cục, giao diện,
      loại nội dung, AI trang, sức khoẻ — D-20261123-site-health-daily
- [x] Hướng dẫn EN + VI cho người viết module khai khối (`docs/guides/page-blocks.md`, mục 9b của MODULE_GUIDE)
- [ ] Thử A/B hai phiên bản trang (D-231, sau bước 57), **sau 1.0** — D-20261122-step67-after-1-0): chia cố định theo người xem (F5 không đổi), bot thấy bản
      gốc, chỉ thử bố cục và nội dung (không thử giá), số liệu và độ tin cậy, người chọn bản thắng
- [ ] AI tạo ảnh (D-231), **sau 1.0** — D-20261122-step67-after-1-0): `ImageModel` mặc định tắt, chỉ banner / ảnh nền / minh hoạ, cờ `ai_generated` và nhãn,
      `catalog` chặn làm ảnh sản phẩm
- [ ] Khối mã tự viết (D-231), **sau 1.0** — D-20261122-step67-after-1-0): mặc định tắt; mức cách ly (iframe `sandbox`, CSP theo host) chủ cửa hàng bật; mức
      không cách ly cần cả máy chủ và chủ cửa hàng bật, không trên trang thanh toán / tài khoản; audit, sổ script

## 66. Đồng bộ nhân viên qua SCIM 2.0 (chủ dự án giao ngày 14/11/2026, D-221)

Đặc tả: [`docs/steps/66-scim.md`](steps/66-scim.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 34 và 40 (đăng nhập OIDC).

- [x] Endpoint SCIM 2.0 trong `auth` (Users, Groups, bộ lọc, PATCH theo RFC 7644) với token SCIM riêng mỗi khách
- [x] Nhân viên `active=false` thì khoá và thu hồi phiên, API key ngay; nhóm ánh xạ sang role ở nút; test trên
      cả hai database

## 65. Chợ ứng dụng qua OAuth 2.0 (`apps`) (chủ dự án giao ngày 14/11/2026, D-221) — **tạm dừng tới sau 1.0** (D-20261121-community-first); **bước đầu tiên được mở sau 1.0** (D-20261125-miniapp-platform)

Đặc tả: [`docs/steps/65-apps.md`](steps/65-apps.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 54 và 33. Ứng dụng bên ngoài cắm vào như Shopify App Store. Đây là tầng "mini app" của nền
(D-20261125-miniapp-platform): nhà phát triển bên ngoài dựng mini app trên core như trên Zalo Mini App hay MoMo Mini
App; mọi mục phải chạy được cả khi một công ty tự vận hành nền của họ lẫn khi một dịch vụ phục vụ nhiều khách.

- [ ] Máy chủ ủy quyền OAuth 2.0 trong `auth`: mã ủy quyền + PKCE bắt buộc, refresh token xoay vòng, phạm vi là
      tên permission, thu hồi và kiểm tra token, siêu dữ liệu máy chủ (RFC 7009, 7636, 7662, 8414)
- [ ] Đăng ký ứng dụng, nền tảng duyệt ứng dụng công khai, khách cài / đồng ý / gỡ (token mất hiệu lực ngay),
      webhook theo ứng dụng, audit `actor_kind = app`; test trên cả hai database
- [ ] Bán ứng dụng trả phí (D-232): thuê bao tháng, dùng thử; nền tảng là bên bán (hoá đơn thuê bao của 35), chia
      doanh thu cho nhà phát triển qua `PayoutGateway` (36); nhà phát triển nước ngoài chờ đối chiếu thuế
- [ ] Bản mô tả mini app máy đọc được (D-20261125-miniapp-platform): tên, phiên bản, quyền xin, địa chỉ giao diện,
      webhook, phiên bản API tối thiểu; cài một mini app từ bản mô tả trên mọi bản cài Autoxide cùng phiên bản lớn;
      server công bố mình hỗ trợ gì để mini app tự dò
- [ ] Giao diện mini app trong sản phẩm của người vận hành (D-20261125-miniapp-platform): hợp đồng cho giao diện chủ
      (danh sách mini app đã cài, chỗ mở, token ngắn hạn theo phiên người dùng trong đúng quyền đã đồng ý) và SDK
      TypeScript cho giao diện mini app (người dùng hiện tại, gọi API, yêu cầu thanh toán, thông báo); cách ly giữa
      mini app và trang chủ
- [ ] Hướng dẫn cho nhà phát triển mini app (EN + VI) kèm một mini app mẫu ở repo riêng, CI cài và chạy nó trên cả
      hai database

## Giai đoạn E — Module thương mại (chủ dự án giao ngày 13/11/2026, D-191)

Module nghiệp vụ, không phải core: khái niệm sản phẩm, đơn hàng, kho không vào `kernel`. Thiết kế:
`docs/research/business-foundation.md`, mục 8. Luật chung cho bước 41–49:

- Mỗi module một crate, sở hữu bảng của mình; mọi bảng có `tenant_id`, dữ liệu theo cửa hàng / kho /
  người bán có `org_id`; quyền theo độ sâu của bước 34. Tiền là `Money`; thuế qua engine thuế, mọi
  chứng từ lưu ảnh chụp giá, thuế, thông tin sản phẩm lúc giao dịch.
- Nhất quán giữa module bằng **giữ chỗ → xác nhận → trả lại** (tồn kho, mã giảm giá, hạn mức): giữ
  chỗ có hạn và mã tham chiếu (vd `order_id`), gọi lại không trùng, job trả chỗ hết hạn; không có
  transaction xuyên module. Việc phụ đi qua event (outbox).
- Thứ tự đăng ký module: `catalog` → `inventory`, `promotions`, `shipping` → `orders` →
  `marketplace`. `shipping` không phụ thuộc `orders`: nhận đơn qua event, báo trạng thái lại qua event.
- Bộ nối với đối tác cụ thể (cổng thanh toán, hãng vận chuyển, nhà cung cấp hoá đơn, SMS, eKYC) là
  module riêng sau trait của core; core chỉ có trait và bộ giả cho test. Viết bộ nối thật cần tài
  khoản thử của đối tác — secret thật, nên dừng hỏi chủ dự án.
- Quy định hay đổi (hạn mức khuyến mại, danh mục hàng có điều kiện, đơn vị hành chính) là dữ liệu có
  ngày hiệu lực (D-186). Mỗi bước có test đồng thời trên cả hai database cho mọi chỗ giữ chỗ.

## 41. Sản phẩm (`catalog`)

Đặc tả: [`docs/steps/41-catalog.md`](steps/41-catalog.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 35 và 36. Khai hạn mức (số sản phẩm) và tính năng (nhiều biến thể, bảng giá) theo
bước 35.

- [x] Chuẩn bị chung cho bước 42–49 (commit đầu tiên): `Subscription::optional` (nhận event của module có thể
      chưa cài), bộ CSV của `forms` thành `autoxide_kernel::csv`, slug của `content` thành
      `autoxide_kernel::slug` — để ba bước song song 42–44 không cùng sửa kernel
- [x] Sản phẩm và biến thể: tuỳ chọn (màu, cỡ…), SKU và mã vạch duy nhất trong khách, trạng thái (nháp, đang
      bán, ngừng bán, lưu trữ), ảnh qua `Media`, nội dung nhiều ngôn ngữ, cân nặng và kích thước (cho vận
      chuyển), nhóm thuế của engine thuế, `warranty_months`; slug và đường dẫn công khai
- [x] Danh mục dạng cây, thuộc tính theo danh mục; **thuộc tính bắt buộc là dữ liệu** (xuất xứ, hạn dùng, nhãn
      hàng hoá theo NĐ 43/2017, NĐ 111/2021 — cần đối chiếu); hàng cấm / có điều kiện là dữ liệu kèm giấy tờ
      phải có; `readiness` để lọc và đếm sản phẩm chưa mở bán được (D-223…D-227)
- [x] Bảng giá: theo cửa hàng / nhóm khách (B2B), kế thừa theo cây (chuỗi đặt, cửa hàng ghi đè — bước 34), ngày
      hiệu lực, **lịch sử giá** (khuyến mại cần "giá ngay trước khuyến mại"); giá niêm yết bằng Đồng Việt Nam,
      gồm thuế khi bán cho người tiêu dùng (Luật Giá 16/2023 — cần đối chiếu)
- [x] Bảng giá theo các nền tảng lớn (D-222): một thứ tự chọn bảng (nút gần nhất → công ty B2B → nhóm khách →
      kênh → thị trường → ưu tiên); bảng giá suy ra ghi thành dòng giá thật; làm tròn giá đẹp; sàn giá (giá tối
      thiểu, biên lợi nhuận trên giá vốn); đổi giá hàng loạt có xem trước, duyệt, hẹn giờ, giá tạm thời
- [x] Đọc công khai cho website: danh sách, lọc theo danh mục / thuộc tính / khoảng giá, tìm (hợp đồng tìm kiếm
      của kernel), ETag và cache như `content`, sitemap; giá sau giảm tự động và lọc `on_sale`; sản phẩm của
      người bán trên website của chuỗi (`seller`, `sold_by`, facet, `ProductCard.seller`)
- [x] Tìm và lọc như các site hàng đầu (D-201): gợi ý tìm gom sản phẩm, danh mục, bài viết, cửa hàng qua một
      trait; khớp đúng dấu xếp trước không dấu; từ đồng nghĩa theo khách; sắp bán chạy và đánh giá; số đếm theo
      từng giá trị lọc (có giới hạn, có index, cả hai database)
- [x] Thẻ sản phẩm và JSON-LD (D-201): tóm tắt biến thể, điểm đánh giá, đơn giá theo đơn vị cơ sở; GTIN,
      thương hiệu, URL từng biến thể, giá gạch là giá ngay trước khuyến mại
- [x] Nhập / xuất CSV bằng job (kiểm từng dòng, lỗi theo dòng, chống CSV injection, trạng thái của lần nhập /
      xuất); gợi ý mô tả và SEO bằng AI chỉ thành bản nháp, đánh dấu `ai_assisted` (bước 37)
- [x] API hợp đồng (`catalog::api`): đọc sản phẩm / biến thể / giá tại một thời điểm để chụp vào chứng từ, sàn
      giá cho `promotions`; điểm cắm `set_listing_gate` cho `marketplace`; event khi giá hay trạng thái đổi;
      tool MCP như REST
- [x] Loại sản phẩm: hàng vật lý, hàng số (link ký có hạn qua `files`, giới hạn lượt tải), dịch vụ (không
      giao hàng), combo / bộ sản phẩm trừ kho từng thành phần; hàng đặt trước (`sale_mode`, ngày dự kiến có
      hàng, giới hạn suất — D-204)
- [x] Gợi ý sản phẩm: sản phẩm tương tự theo ý nghĩa (embedding sẵn có của module `ai`); "thường mua cùng"
- [x] Hỏi đáp và ảnh song ngữ (D-202): `catalog.product` trong sổ đối tượng, nhãn `seller` (permission quản lý
      sản phẩm ở nút sở hữu), event vòng đời (lưu trữ / xoá → đóng và ẩn hỏi đáp); `catalog_product_media`
      ghi đè `alt` theo ngôn ngữ như `media_alt_translations`
- [x] Danh mục trong menu của website (D-238): `catalog.category` trong sổ đối tượng (`resolve_many` theo ngôn ngữ,
      `children` theo thứ tự hiển thị, chỉ danh mục đang hiện), `catalog.product` có `image`; khối "Bán chạy" của
      bảng xổ là danh sách sản phẩm công khai lọc theo danh mục, sắp `best_selling`
- [x] Tạo sản phẩm nháp từ ảnh bằng AI (tên, mô tả, danh mục trong cây của khách, thuộc tính, SEO — D-229)
- [x] Khối `catalog.product_grid`, `product_card`, `collection` cho trang dựng từ khối (bước 67,
      D-20261122-commerce-blocks)
- [x] Website bán hàng (web-build-api-gaps, 16/11/2026): thông tin hiển thị theo biến thể trong `by-slug`, theo
      ngôn ngữ (vd số người dùng, trong hộp có gì, cách dùng / chăm sóc, dùng được máy rửa bát) — thuộc tính khai
      theo loại sản phẩm, không viết cứng ngành nào; catalog công khai lọc theo người bán và đường dẫn trang sản
      phẩm cho hàng người bán (cùng 46)

## 42. Kho (`inventory`)

Đặc tả: [`docs/steps/42-inventory.md`](steps/42-inventory.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 41. Không bao giờ bán quá tồn khi nhiều request cùng lúc, trên cả hai database.

- [x] Kho là nút tổ chức loại `warehouse` hay cửa hàng (bước 34); tồn theo biến thể × kho; **sổ biến động kho
      chỉ thêm** (nhập, xuất, điều chỉnh, chuyển); tồn hiện tại suy từ sổ và giữ bằng khoá dòng của kernel
- [x] Giữ chỗ tồn: giữ có hạn theo `order_id` → xác nhận khi đặt hàng → trả lại khi huỷ hay hết hạn (job); chọn
      kho theo quy tắc (gần người mua, đủ hàng); test đồng thời "100 người mua 1 cái cuối"
- [x] Chuyển kho (đang đi đường), kiểm kê (chênh lệch thành bút toán điều chỉnh có lý do; lý do là danh sách
      theo khách, D-223), cảnh báo tồn thấp (event), cho phép bán khi hết hàng hay không theo sản phẩm
- [x] Giá vốn bình quân gia quyền theo kho cho báo cáo lãi gộp; lô và hạn dùng (bán trước hết hạn trước, ngưỡng
      sắp hết hạn theo nút), số serial — bật theo sản phẩm
- [x] Nhận event của `orders` (xác nhận, huỷ, trả hàng nhập lại kho) — chịu được nhận lại event
- [x] Nhập hàng (D-194): nhà cung cấp, đơn đặt hàng nhà cung cấp, nhận hàng một phần, trả hàng cho nhà cung
      cấp; công nợ phải trả vào sổ cái, trạng thái ghi sổ của phiếu nhập (ghi lại được), công nợ từng nhà
      cung cấp qua `ledger`
- [x] Hàng đặt trước (D-204): biến thể `preorder` (ngày dự kiến có hàng, giới hạn suất — bước 41 khai); giữ suất
      không vượt giới hạn khi nhiều người đặt cùng lúc; hàng về thì chia cho đơn đặt trước theo thứ tự đặt
- [x] Tình trạng hàng công khai theo cửa hàng (D-200): mức `in_stock` / `low` (kèm số khi dưới ngưỡng cấu hình)
      / `out` / `preorder`, không lộ số tồn lớn; `catalog` lọc "chỉ hàng có sẵn ở cửa hàng X" qua `api`; ETag,
      cache ngắn
- [x] Báo khi có hàng lại theo cửa hàng (D-201): người mua đăng ký (xác nhận hai bước, đồng ý riêng cho mục đích
      này), nghe event tồn kho rồi gửi qua `notifications`; mỗi người một lần mỗi đợt có hàng

## 43. Khuyến mại (`promotions`)

Đặc tả: [`docs/steps/43-promotions.md`](steps/43-promotions.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 41. Luật khuyến mại đổi thường xuyên: hạn mức là dữ liệu có ngày hiệu lực, không viết
cứng (Thông tư 39/2025/TT-BCT trần 50% giá ngay trước khuyến mại, có ngoại lệ chương trình tập trung
quốc gia; NĐ 239/2026 sửa NĐ 81/2018 — cần đối chiếu văn bản hiện hành).

- [x] Chương trình là luật, đến người mua bằng ba cách (D-222): giảm tự động; coupon — mã ai có cũng dùng
      được (mã chung hay lô mã riêng); voucher — quyền dùng một lần trong ví của một tài khoản; mã gắn với
      một người luôn là voucher; khung thời gian với ngày kết thúc bắt buộc (D-225)
- [x] Loại khuyến mại: giảm theo sản phẩm, danh mục, giá trị đơn, nhóm khách; mua X tặng Y (mua 2 tính 1,
      món rẻ nhất); miễn phí vận chuyển; flash sale; bậc thang, combo đồng giá, mua kèm deal sốc, chọn quà
      (D-222)
- [x] Điều kiện: phương thức thanh toán, hãng giao, tỉnh, phân khúc, hạng, app / web / quầy, thương hiệu,
      người bán, khách mới, loại trừ hàng đang giảm (D-222); đơn đầu tiên (`first_order_only`); khung giờ
- [x] API cho `orders`: tính giảm giá cho một giỏ, trả từng dòng giảm kèm lý do (người mua thấy rõ điều
      kiện — Luật 19/2023); tính trong bộ nhớ, đo p99 < 5 ms cho 50 dòng × 500 chương trình (D-222)
- [x] Cộng dồn theo ba lớp sản phẩm / đơn / vận chuyển: độc quyền trong lớp hay toàn giỏ, `combines_with`,
      dừng sau chương trình, chế độ ưu tiên hay lợi nhất cho người mua (tất định); 1 voucher mỗi lớp cho
      mỗi bên trả tiền (D-222)
- [x] Ai trả tiền giảm giá (sàn / người bán / nhãn hàng / đối tác thanh toán, chia theo tỉ lệ) chia về từng
      dòng; người bán đăng ký tham gia chương trình của sàn; ngân sách tiền, lượt theo ngày (D-222)
- [x] Giữ chỗ lượt dùng (tổng, mỗi người, theo ngày, ngân sách), suất flash sale và quà như tồn kho (giữ →
      xác nhận → trả lại), test đồng thời; chống lạm dụng theo nhiều khoá người mua (tài khoản, số điện
      thoại, địa chỉ — chỉ bản băm), giới hạn theo kỳ (D-222)
- [x] Ví voucher của người mua (D-222): lưu, nhập mã nhận, được tặng qua `grant_voucher` (gọi lại không
      cấp thêm), cấp tay có lý do; có trong xuất dữ liệu cá nhân, xoá tài khoản thì thu hồi voucher chưa dùng
- [x] Quy tắc trả hàng và huỷ cho `orders` (D-222): hoàn theo tỉ lệ, quà khi trả món điều kiện, voucher về
      ví khi huỷ đơn
- [x] Kiểm tuân thủ: chặn chương trình vượt hạn mức đang hiệu lực (cộng mọi khoản giảm trên một đơn vị,
      D-222) và sàn giá; giới hạn số ngày giảm giá là dữ liệu; kiểm trước khi bật, không ghi (D-225)
- [x] Thông báo khuyến mại theo từng nơi nhận (D-225): xuất dữ liệu thông báo gửi Sở Công Thương nơi tổ
      chức, ghi nhận đã nộp từng nơi; lưu vết chương trình đã chạy
- [x] Mã cho người mua thấy: mã / voucher dùng được cho giỏ và mã chưa đủ điều kiện kèm lý do ("thêm X để
      dùng"), bộ chọn thay ô nhập mã (D-201); trang sản phẩm hiện "còn X với mã …", giá niêm yết vẫn là giá
      không mã (D-200); số lượt còn lại chỉ hiện khi là số thật
- [x] Công khai và nhắc: event giá sau giảm tự động cho danh sách công khai; flash sale công khai với suất
      còn là số thật; nhắc khi flash sale bắt đầu (đồng ý riêng cho một lần nhắc, D-225)
- [x] Chạy thử chương trình trên giỏ mẫu, kiểm xung đột, lịch, số liệu chương trình (D-222)
- [x] Khối `promotions.flash_sale`, `promotions.voucher_claim` cho trang dựng từ khối (D-229, bước 67);
      `promotions.promotion` trong sổ đối tượng (chỉ công khai khi đang chạy) để menu của website trỏ tới (D-238)
- [x] Website bán hàng (16/11/2026): `GET /public/promotions` trả số thông báo khuyến mại với Sở Công Thương và
      mức giảm tối đa theo luật đang áp (dữ liệu có ngày hiệu lực, luật số hai)

## 44. Vận chuyển (`shipping`)

Đặc tả: [`docs/steps/44-shipping.md`](steps/44-shipping.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 41. Không phụ thuộc `orders`: nhận đơn qua event, báo trạng thái qua event.

- [x] Sổ địa chỉ của người mua dùng `Address` của module nền `address` (bước 34, D-193); địa chỉ nước ngoài
      dạng chung
- [x] Phí vận chuyển khách tự đặt: vùng theo tỉnh, bậc cân nặng / kích thước, miễn phí từ một giá trị đơn,
      nhận tại cửa hàng (chuỗi); API báo giá cho `orders`; thuế GTGT trên phí giao (Luật 48/2024 — cần đối
      chiếu)
- [x] Trait `ShippingCarrier` (báo giá, tạo vận đơn, huỷ, in nhãn, theo dõi) và bộ giả cho test; trạng thái
      kiểm tài khoản hãng; webhook trạng thái của hãng: kiểm chữ ký, không trùng, ánh xạ về trạng thái chung
- [x] Vận đơn: một đơn nhiều kiện, giao một phần, giao thất bại, hoàn hàng; event cho `orders`
- [x] Thu hộ khi giao (COD): tiền do hãng thu và chuyển trả theo hợp đồng với hãng; sổ cái ghi phải thu từ hãng,
      đối soát với bảng kê COD; lệch thì báo, khớp dòng lệch có lý do và audit; `shipping.cod_settled` mang
      `seller_org_id` và nơi nhận tiền (D-216)
- [x] Ngày giao dự kiến là ngày cụ thể (D-201): khoảng ngày tuyệt đối kèm giờ chốt trong ngày, theo múi giờ của
      khách; lịch giao, ngày làm việc, ngày nghỉ, giờ chốt giao trong ngày là dữ liệu có ngày hiệu lực; nhận
      tại cửa hàng trả giờ sẵn sàng (chuẩn bị + giờ mở cửa của bước 34) và thời hạn giữ hàng chờ nhận
- [x] Ước tính phí giao công khai (D-200): theo tỉnh và một sản phẩm / số lượng, chưa cần giỏ, cho trang sản
      phẩm; cùng bảng phí với báo giá của `orders`, ghi rõ "ước tính", ngưỡng miễn phí
- [x] Câu trả lời tự động về hành trình vận đơn cho `support` (`FactSource`, chỉ cho chủ đơn — D-230)
- [x] Website bán hàng (16/11/2026): phí và ngày giao dự kiến theo tỉnh khi chưa có giỏ (trang sản phẩm), kèm giờ
      chốt đơn trong ngày của nơi bán

## 45. Giỏ hàng, đặt hàng, đơn hàng (`orders`)

Đặc tả: [`docs/steps/45-orders.md`](steps/45-orders.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 42, 43, 44. Gọi `catalog`, `inventory`, `promotions`, `shipping` qua `api` của chúng;
tiền qua `PaymentGateway` và sổ cái của bước 36.

- [x] Tạo tài khoản từ số điện thoại đã xác thực qua tra đơn vãng lai (D-202, chuyển từ bước 34 — D-316):
      `POST /api/v1/auth/phone-signups` khi `orders` có token xem đơn, đăng nhập bằng passkey hay OTP, không bắt
      buộc mật khẩu (đặc tả 34 mục "Tạo tài khoản từ số đã xác thực") — D-20261122-late-apis (`StartContext::later`,
      `autoxide_kernel::sms::VerifiedPhoneTokens`)
- [x] Giỏ hàng cho người mua và vãng lai (gộp khi đăng nhập); giá, giảm, thuế, phí giao luôn tính lại ở máy
      chủ, không tin số từ client; `free_shipping_remaining` (D-201); coupon và voucher tách nhau, điền dữ liệu
      điều kiện cho `promotions`, chụp khoản giảm kèm ai trả tiền vào đơn (D-222); danh sách giỏ bỏ dở
- [x] Đặt hàng: báo giá cuối → giữ tồn, lượt khuyến mại → số đơn tuần tự (bước 36) → thanh toán theo chế độ
      dòng tiền (tự thu / thu hộ / chia tại cổng / COD; `payment_actions[]`, D-216); `Idempotency-Key` bắt
      buộc; hạn thanh toán đọc từ `ledger` (D-218), hết hạn thì trả chỗ; trạng thái thanh toán công khai
- [x] Vòng đời đơn: chờ thanh toán → xác nhận → chuẩn bị → đang giao → đã giao → hoàn tất; huỷ theo quy tắc
      (`buyer_cancel_until` tới `preparing`), chờ cửa hàng nhận đơn `acceptance_hours` (D-209); mọi chuyển
      qua service, ghi event và bút toán sổ cái; gửi lại link thanh toán còn hạn
- [x] Nhận tại cửa hàng và hàng đặt trước: mã nhận 6 số, người nhận hộ, giữ hàng chờ nhận có hạn rồi huỷ /
      hoàn theo chính sách; đặt trước (D-204): tách lần giao, đồng ý điều kiện và thấy ngày giao trước khi
      đặt, dời ngày thì báo và cho huỷ miễn phí, ngừng bán đặt trước thì huỷ và hoàn tiền tự động
- [x] Đổi trả và hoàn tiền: trả trong hạn chính sách, lý do theo `allowed_return_reasons` (D-209, D-223), duyệt,
      nhận về (nhập kho), hoàn qua cổng, bút toán đảo, hoá đơn điều chỉnh / thay thế (NĐ 123/2020 sửa bởi
      NĐ 70/2025 — cần đối chiếu); hoàn theo tỉ lệ khoản giảm, xử lý quà, voucher về ví (D-222)
- [x] Trả hàng phía người mua (D-201, D-202): cách gửi trả (lấy tận nơi / bưu cục / cửa hàng), thời gian hoàn
      theo phương thức thanh toán; ảnh kèm (lý do bắt buộc ảnh là chính sách theo nút); `returns/preview`
      trả tiền hoàn, cách gửi và phí, thời gian hoàn, hạn — không ghi
- [x] Hoá đơn điện tử qua `EInvoiceProvider`, kể cả từ máy tính tiền (D-20261120-step45-einvoice-pos); `invoice_timing = on_delivered` mặc
      định (D-215); khoản giảm, hàng khuyến mại trên hoá đơn (D-222, cần đối chiếu NĐ 123/2020 sửa bởi NĐ
      70/2025); mail xác nhận, cập nhật trạng thái qua job theo i18n; chứng từ của người mua
- [x] Mua không cần tài khoản (D-201, D-202, D-20261120-step45-guest-orders): tra đơn bằng số đơn + OTP; token xem đơn để huỷ, đổi địa chỉ
      (OTP lần hai), mở hỗ trợ ở `support` (D-204), qua cùng service; giới hạn tra / gửi lại OTP / hạn token
      là cấu hình; che số điện thoại, địa chỉ tới cấp phường; gắn đơn vãng lai vào tài khoản đã xác thực số
- [x] Bán tại quầy (POS) qua API cho chuỗi (D-20261120-step45-einvoice-pos; danh mục đầy đủ và tìm khách ở quầy chờ
      bước 47): đơn tạo tại cửa hàng, tiền mặt / thẻ / QR, trừ kho của cửa hàng
      đó; đóng ca có chênh lệch: lý do, ghi sổ cái, cần duyệt khi vượt ngưỡng (D-227)
- [x] Đánh giá sau mua (D-20261120-step45-reviews-risk-reports): chỉ người đã mua, kiểm duyệt không chặn / ẩn trái quy định (Luật 19/2023), lý do ẩn
      theo danh sách (D-223); dùng `comments`, module `reviews` hay đặt ở `orders` — ghi quyết định; lọc có
      ảnh / số sao / sắp xếp, ảnh người mua; đếm đánh giá mới theo mốc đã xem; nhãn `verified_buyer` (D-202)
- [x] Nút "Đánh giá này hữu ích" (D-203): mỗi người một phiếu, rút được, đếm cùng transaction, `sort=helpful`
      bằng con trỏ, không tự / người bán bỏ phiếu, không có "không hữu ích"; tóm tắt đánh giá bằng AI chỉ là
      bản nháp nhân viên duyệt, gắn nhãn (D-109, D-201)
- [x] Báo cáo bán hàng theo cửa hàng / kênh / sản phẩm / thời gian, theo quyền độ sâu; event cho kho báo cáo
      gộp (bước 38); xuất đơn qua xuất chung của bước 51 (D-213)
- [x] Chống "bom hàng" COD (D-194): tỉ lệ nhận hàng thành công của người mua chỉ trong dữ liệu của khách (có
      căn cứ xử lý theo Luật BVDLCN), quy tắc buộc trả trước khi rủi ro cao, danh sách chặn có lý do và thời
      hạn; không chia sẻ danh sách giữa các khách
- [x] Trả góp qua đối tác (trait của bước 36); bảo hành theo số serial hay theo đơn: phiếu bảo hành, tiếp
      nhận, trạng thái xử lý, hạn bảo hành
- [x] Khối `orders.add_to_cart`, `buy_now`, `reviews` cho trang dựng từ khối (D-229) và `orders.cart_link` cho đầu
      trang (D-234); câu trả lời tự động về trạng thái đơn cho `support` (`FactSource`, chỉ cho chủ đơn — D-230)
      — `FactSource`: D-20261120-step45-guest-orders; khối: D-20261122-commerce-blocks
- [x] Ô người mua nhập lúc đặt hàng (D-235): `orders.order` là đích trường tự định nghĩa (P19 của bước 51) có mức
      `buyer_input`; checkout kiểm cùng bộ gom lỗi; mail xác nhận đọc được các trường này
- [x] Trả trước một phần cho hàng đặt trước (D-232): tỉ lệ hay số tiền, phần còn lại thu khi sẵn sàng giao,
      người bán huỷ hoàn đủ, người mua huỷ theo điều kiện công bố; không mặc định là "đặt cọc" (BLDS Điều 328)
      — thu phần còn lại bằng COD và hoá đơn lúc thu trước chưa làm (D-20261120-step45-preorder-advance)
- [x] Website bán hàng (16/11/2026): báo giá giỏ trả thêm số điểm tối đa dùng được, cách trả dùng được cho giỏ
      (COD theo từng người bán), ngày giao dự kiến theo từng nơi bán; đơn có sẵn `return_until`,
      `buyer_cancel_until`; đánh giá: số đếm có ảnh và theo biến thể, "chủ đề người mua hay nhắc" kèm bản tóm tắt AI
      đã được người duyệt (D-109: AI chỉ là gợi ý)
      — đã có cách trả, ngày giao dự kiến theo nơi bán, `return_until` / `buyer_cancel_until`, đếm có ảnh và theo
      biến thể, chủ đề kèm tóm tắt đã duyệt (D-20261120-step45-warranty-installment-fields); số điểm tối đa: `tender_limits` của
      giỏ qua `Tender::limit` (D-20261122-cart-tender-limits)

## 46. Sàn (`marketplace`) — **tạm dừng tới sau 1.0** (D-20261121-community-first)

Đặc tả: [`docs/steps/46-marketplace.md`](steps/46-marketplace.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 45 và 37. Sàn là một khách; người bán là nút loại `seller` (bước 34).

- [ ] Người bán đăng ký: hồ sơ, KYC qua cổng chặn của bước 37 (chưa xác thực thì không đăng bán),
      chấp nhận quy chế sàn và hợp đồng với người bán có phiên bản (bước 37), giấy tờ cho ngành hàng
      có điều kiện; gian hàng của người bán; thông tin người bán hiện trên trang sản phẩm; `marketplace.seller`
      trong sổ đối tượng để menu của website trỏ tới gian hàng (D-238)
- [ ] Duyệt sản phẩm của người bán: hàng cấm, hàng có điều kiện thiếu giấy tờ, báo vi phạm (bước 37)
- [ ] Phí sàn theo ngành hàng là dữ liệu có ngày hiệu lực; một lần đặt hàng tách thành đơn theo
      từng người bán; người bán tự giao hay sàn giao
- [ ] Chi trả cho người bán: tạm giữ tới hết hạn đổi trả, trừ phí sàn và thuế khấu trừ thay (NĐ
      117/2025), lệnh chi qua `PayoutGateway`; bảng đối soát cho người bán; sổ cái khớp từng đồng
- [ ] Tranh chấp người mua – người bán, sàn phân xử, có hạn xử lý; điểm và chế tài người bán; báo cáo
      cho cơ quan có thẩm quyền khi được yêu cầu (có audit)
- [ ] Cung cấp thông tin người bán và doanh thu cho cơ quan thuế theo kỳ (NĐ 117/2025 — cần đối
      chiếu mẫu và hạn); kho của sàn (sàn nhận hàng của người bán, đóng gói, giao — tồn theo người bán
      trong kho của sàn)
- [ ] Đối chiếu thiết kế 2 (D-214, D-216, D-218): bút toán theo tên tài khoản của 36, tạm giữ qua
      `ledger_holds`; COD không về tài khoản sàn; danh sách người bán công khai, đánh giá theo người bán, theo
      dõi gian hàng (đồng ý riêng cho tin hàng mới); huy hiệu chỉ từ dữ liệu thật
- [ ] Quyết toán theo ai trả tiền giảm giá (D-222): voucher sàn thì người bán nhận đủ, voucher shop trừ doanh
      thu người bán; gốc tính phí sàn là dữ liệu có ngày hiệu lực; người bán đồng ý trước khi cùng chịu chi phí
- [ ] Website bán hàng (16/11/2026): hồ sơ người bán công khai có số sản phẩm theo nhóm, tỉ lệ giao đúng hạn,
      trạng thái tạm ngừng; phí sàn công khai (không cần `marketplace.sell`); đăng ký bán trước khi có tài khoản;
      endpoint công khai báo vi phạm cho sản phẩm, đánh giá, livestream (lý do theo danh sách cố định; cơ chế tiếp
      nhận phản ánh của sàn theo luật thương mại điện tử — cần đối chiếu văn bản gốc)

## 47. Khách hàng và giữ chân khách (`customers`)

Đặc tả: [`docs/steps/47-customers.md`](steps/47-customers.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 45. Người mua là tài khoản theo từng khách (bước 34); module này giữ hồ sơ bán hàng.

- [x] Hồ sơ khách: lịch sử mua (từ event của `orders`), tổng chi; nhãn, ghi chú, trường tự định nghĩa dùng chỗ
      chung của bước 51 (D-237); che số điện thoại / email, xem đủ cần permission và ghi audit (D-226) —
      D-20261120-step47-customers
- [x] Nhóm và phân khúc theo quy tắc, dùng cho bảng giá (bước 41) và khuyến mại (bước 43); phân khúc và
      hạng thành viên làm điều kiện khuyến mại (D-222) — nhóm cắm vào `orders` qua `BuyerGroups`; hạng có giá trị
      khi mục điểm thưởng / hạng xong
- [x] Điểm thưởng và hạng thành viên: sổ điểm chỉ thêm, điểm có hạn dùng, quy tắc tích / đổi là dữ liệu;
      điểm chưa dùng là nghĩa vụ ghi trong sổ cái; cơ sở tính điểm thống nhất (D-226) — D-20261120-step47-loyalty
      (còn: trả điểm khi hoàn phần đơn trả bằng điểm, đối trừ `buyer.prepayments` với phải thu của đơn)
- [x] Thẻ quà tặng và số dư cửa hàng: chỉ dùng mua hàng của chính khách đó, không rút ra tiền, không chuyển
      ra ngoài — **tắt mặc định**, chỉ bật sau khi đối chiếu NĐ 52/2024 về ví điện tử / thẻ trả trước
      (D-194); số dư là nghĩa vụ trong sổ cái — D-20261120-step47-stored-value (còn: bán thẻ qua `orders`,
      hoàn đơn trả bằng thẻ về thẻ, người mua chọn nhận hoàn tiền dạng số dư, giới hạn theo cửa hàng)
- [x] Danh sách yêu thích (gộp danh sách của trình duyệt vào tài khoản khi đăng nhập), sản phẩm vừa xem;
      nhắc giỏ hàng bỏ dở qua `notifications` — chỉ khi người mua đã đồng ý nhận tin quảng cáo (bước 37);
      giới thiệu bạn bè có thưởng — D-20261120-step47-engagement
- [x] Tặng voucher (D-222): sinh nhật, đổi điểm lấy voucher, sau khi mua, giới thiệu bạn — qua
      `grant_voucher`, gọi lại không cấp thêm — D-20261120-step47-loyalty, D-20261120-step47-engagement
- [x] Báo khi sản phẩm yêu thích có hàng lại hay giảm giá (D-233): người mua tự bật, đồng ý riêng, chỉ giảm thật
      ≥ ngưỡng so với giá mốc, tối đa 1 lần / 7 ngày mỗi sản phẩm, gom lô theo khung giờ — D-20261120-step47-wishlist-alerts
- [x] Website bán hàng (16/11/2026): đăng nhập / tạo tài khoản công khai cho người mua bằng OTP qua SMS hay
      passkey (gửi SMS qua đối tác, không giữ secret trong code) — D-20261120-phone-accounts

## 48. Bán đa kênh (`channels`) — **tạm dừng tới sau 1.0** (D-20261121-community-first)

Đặc tả: [`docs/steps/48-channels.md`](steps/48-channels.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 45. Người bán ở Việt Nam bán cùng lúc trên website, Shopee, Lazada, TikTok Shop,
Facebook / Zalo; một kho, một danh sách sản phẩm, đơn từ mọi nơi về một chỗ.

- [ ] Trait `SalesChannel` (đăng sản phẩm, cập nhật giá và tồn, lấy đơn, cập nhật trạng thái giao)
      và bộ giả cho test; bộ nối từng sàn là module riêng, làm khi có tài khoản thử (D-191)
- [ ] Liên kết sản phẩm / biến thể với mã trên từng kênh; giá riêng theo kênh (giá cố định cho một tin
      đăng, hay bảng giá suy ra theo kênh của `catalog` — D-222)
- [ ] Đồng bộ tồn có phần để dành cho từng kênh, không bán quá khi nhiều kênh cùng bán; số đẩy lên kênh ở
      chế độ `reserved` (phần giữ riêng còn lại + phần chia từ tồn chung)
- [ ] Đơn từ kênh khác vào `orders` với kênh nguồn, trừ kho chung, không trùng khi kênh gửi lại; trạng
      thái đơn kênh sau khi xử lý (`resolved` kèm lý do — D-223, `cancelled_by_channel`)
- [ ] Báo cáo theo kênh; đối soát tiền sàn trả với sổ cái
- [ ] Nguồn cấp sản phẩm cho Google Merchant Center và danh mục của Meta (D-204): tệp XML / CSV dựng bằng
      job theo cửa hàng, ngôn ngữ, tiền tệ; link công khai có token; báo lỗi từng sản phẩm (thiếu GTIN,
      ảnh); không lộ số tồn hay giá vốn

## 49. Tiếp thị liên kết và bán qua livestream (`affiliate`) — **tạm dừng tới sau 1.0** (D-20261121-community-first)

Đặc tả: [`docs/steps/49-affiliate.md`](steps/49-affiliate.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 45 và 37. Luật TMĐT 2025 buộc xác thực danh tính người livestream và người tiếp thị
liên kết (bước 37).

- [ ] Người tiếp thị đăng ký có KYC (cổng chặn bước 37), link và mã riêng, thời hạn ghi nhận đơn, quy
      tắc hoa hồng theo sản phẩm / danh mục là dữ liệu có ngày hiệu lực
- [ ] Hoa hồng ghi sổ cái khi đơn hoàn tất, trừ lại khi trả hàng; chi trả qua `PayoutGateway` có khấu
      trừ thuế thu nhập cá nhân theo hồ sơ thuế (tỉ lệ và ngưỡng là dữ liệu — các nguồn đang khác nhau
      giữa 2 triệu và 5 triệu đồng / lần, NĐ 253/2026 — cần đối chiếu); chứng từ khấu trừ cho người nhận
- [ ] Phiên livestream: người phát đã xác thực, sản phẩm gắn trong phiên, đơn ghi nhận theo phiên,
      lưu vết phiên theo yêu cầu của luật
- [ ] Đối chiếu đợt 2: `paid_on` khi đánh dấu đã chi
- [ ] Website bán hàng (16/11/2026): danh sách phiên livestream công khai (lịch phát), không chỉ một phiên theo id

## 55. Giao diện cửa hàng qua API: banner, bộ sưu tập, thông tin cửa hàng (D-219)

Đặc tả: [`docs/steps/55-storefront.md`](steps/55-storefront.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 41. Website và app của khách tự vẽ giao diện (D-026: core chỉ có API); bước này cho
họ dữ liệu để vẽ trang chủ, trang danh mục, chân trang mà không phải sửa code giao diện mỗi lần đổi.

- [x] Vị trí hiển thị và banner (`content`): vị trí theo khoá (vd `home_hero`), banner có ảnh máy tính
      và điện thoại qua `Media`, tiêu đề, nút, link (địa chỉ, bài, danh mục, sản phẩm, bộ sưu tập,
      chương trình khuyến mại), ngôn ngữ, khung giờ hiển thị, thứ tự, áp cho cửa hàng / nhánh (bước 34)
- [x] Đọc công khai `GET /api/v1/content/public/placements/{key}`: banner đang hiệu lực lúc gọi, ảnh có
      `srcset` và WebP, cache tới mốc bắt đầu / kết thúc gần nhất; banner trỏ tới chương trình khuyến
      mại tự ẩn khi chương trình kết thúc (event tuỳ chọn của `promotions`) — quảng cáo không được hứa
      ưu đãi đã hết (Luật 19/2023, Luật Quảng cáo — cần đối chiếu)
- [x] Bộ sưu tập sản phẩm (`catalog`): chọn tay (có thứ tự) hay theo quy tắc (danh mục, thuộc tính,
      khoảng giá, hàng mới, đang khuyến mại), sắp theo mới nhất / giá / bán chạy (số bán nhận từ event
      tuỳ chọn của `orders`); trang công khai có slug, SEO, ảnh; dùng được trong menu và banner
      — số bán dùng nguồn 90 ngày sẵn có từ `orders.order_completed` (D-20261119-step55-collections,
      D-20261120-step55-close)
- [x] Thông tin cửa hàng công khai (`organizations`): thương hiệu (tên, logo, biểu tượng trang, liên hệ, mạng xã
      hội) là cài đặt kế thừa theo cây (chuỗi đặt, cửa hàng ghi đè); địa chỉ, giờ mở cửa, ngày nghỉ lấy từ hồ sơ
      địa điểm của bước 34, không lưu hai chỗ (D-237); chân trang lấy thêm thông tin chủ sở hữu và logo Bộ Công
      Thương từ bước 37 — chân trang pháp lý lấy từ endpoint công khai của `compliance` (D-20261119-step55-store-info)
- [x] Liên kết tới đối tượng của module đăng ký sau (sản phẩm, bộ sưu tập, chương trình) qua điểm cắm
      `ContentLinks` trong `content::api`: module đó gắn bộ dựng đường dẫn và bộ kiểm "còn hiển thị
      không"; thiếu module thì link loại đó bị từ chối khi lưu — làm bằng sổ đối tượng của kernel thay
      `ContentLinks` (D-20261119-step55-banners)
- [x] Tool MCP cho banner và bộ sưu tập (soạn, lên lịch, sắp xếp) như REST; AI chỉ gợi ý chữ, đánh dấu
      `ai_assisted` (bước 37); hướng dẫn cho người làm giao diện (EN + VI) — `docs/guides/storefront.md`
      (D-20261120-step55-close)
- [x] Kiểu vị trí banner (D-236): cách hiện (một banner, trượt, lưới, xếp dọc, dải chữ, popup), cỡ và tỉ lệ ảnh
      kiểm khi lưu, thiết bị, loại trang dùng được; bộ vị trí chuẩn có sẵn cho khách mới (dải trên, banner chính,
      giữa trang chủ, đầu danh mục, cột trái / phải, giữa nội dung, trên chân trang, popup); banner chọn trang hiện
      (loại trang, danh mục / sản phẩm / bài cụ thể); banner dạng chữ màu theo theme; đọc nhiều vị trí một lần;
      khối `content.placement` cho trang dựng từ khối và đầu trang
      — khối `content.placement` chuyển sang bước 67 (cần sổ khối, D-20261120-step55-close); link và trang được
      chọn dùng sổ đối tượng thay `ContentLinks` (D-20261119-step55-banners)
- [ ] Banner trong gian hàng người bán trên sàn (D-233): người bán tự quản trong nút của mình, sàn duyệt theo
      chính sách, link chỉ tới hàng của chính người bán, không phải quảng cáo trong sàn
      — **chờ bước 46** (`marketplace` chưa có); không làm trước (D-20261120-step55-close)

## 56. Chuyển dữ liệu từ nền tảng khác (`imports`, D-219) — **tạm dừng tới sau 1.0** (D-20261121-community-first)

Đặc tả: [`docs/steps/56-imports.md`](steps/56-imports.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 47. Người bán đang ở Shopify, WooCommerce, Haravan, Sapo, KiotViet muốn chuyển sang mà
không nhập lại tay và không mất thứ hạng tìm kiếm của đường dẫn cũ.

- [ ] Module `imports`: tải file xuất (CSV, Excel) của nền tảng cũ; **hồ sơ ánh xạ cột là dữ liệu có
      phiên bản** cho từng nền tảng (định dạng xuất của họ đổi thì sửa dữ liệu, không sửa code), người
      dùng chỉnh ánh xạ được; nhận diện bảng mã và dấu phân cách
- [ ] Chạy thử trước: xem trước số dòng, lỗi theo từng dòng và cột, không ghi gì; chạy thật bằng job
      theo lô, dừng / chạy tiếp được, gọi `api` của `catalog`, `inventory`, `customers`, `orders` (đơn
      cũ nhập làm lịch sử, không trừ kho, không gửi thông báo) — không ghi thẳng vào bảng module khác
- [ ] Chạy lại không trùng: bảng ánh xạ ID nguồn → ID mới theo nền tảng và loại; ảnh tải từ link của
      nền tảng cũ qua bộ chặn SSRF của kernel (`autoxide_kernel::net`), giới hạn cỡ, vào `media`
- [ ] Giữ đường dẫn cũ: tạo chuyển hướng 301 từ đường dẫn sản phẩm / danh mục / bài cũ sang đường dẫn mới
      (chuyển hướng của `content`), báo các đường dẫn không ánh xạ được
- [ ] Dữ liệu cá nhân khi nhập khách hàng (Luật BVDLCN 91/2025): người nhập xác nhận căn cứ xử lý; đồng ý
      nhận quảng cáo **không** tự mang sang nếu không có bằng chứng (bước 37); mật khẩu cũ không nhập —
      khách đặt lại qua link; file tải lên xoá sau khi xong
- [ ] Báo cáo sau khi nhập (đã tạo, bỏ qua, lỗi, cần làm tay); hướng dẫn chuyển từ từng nền tảng (EN + VI)

## 59. Bán sỉ B2B (`wholesale`) (chủ dự án giao ngày 14/11/2026, D-221) — **tạm dừng tới sau 1.0** (D-20261121-community-first)

Đặc tả: [`docs/steps/59-wholesale.md`](steps/59-wholesale.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 45 và 47. Bán cho doanh nghiệp trên cùng hệ thống bán lẻ.

- [ ] Công ty mua có nhiều người mua và người duyệt; bảng giá sỉ, bậc giá theo số lượng, số lượng tối thiểu —
      không lộ giá sỉ ra công khai
- [ ] Báo giá có hạn, chấp nhận thành đơn với giá đã chốt; đặt nhanh theo SKU / CSV
- [ ] Trả chậm theo điều khoản, hạn mức tín dụng không vượt khi đặt đồng thời, công nợ trong sổ cái, tuổi nợ,
      nhắc nợ, chặn đơn trả chậm khi quá hạn; hoá đơn ghi mã số thuế công ty; test trên cả hai database
- [ ] Hợp đồng khung (D-232): cam kết số lượng / giá trị, đơn gọi hàng trừ cam kết nguyên tử; mời thầu nhiều
      người bán trên sàn, giá niêm phong tới hạn, trao thầu thành báo giá / hợp đồng; không dùng cho mua sắm công
- [ ] Ký hợp đồng điện tử trong `compliance` (D-232): `SignatureProvider` + bộ giả, ký `simple` (OTP + đồng ý) hay
      `digital` (chữ ký số qua nhà cung cấp được cấp phép), mã băm chống sửa tệp; dùng cho 59, 46, 49
- [ ] Trao đổi chứng từ B2B tự động (EDI, D-233): UBL 2.1 cho đơn, xác nhận đơn, báo giao; vào qua API key
      `service`, ra qua `webhooks`; chống trùng theo ID chứng từ, chặn XXE; EDIFACT / X12 là bộ chuyển đổi riêng

## 60. Đặt lịch dịch vụ (`bookings`) (chủ dự án giao ngày 14/11/2026, D-221)

Đặc tả: [`docs/steps/60-bookings.md`](steps/60-bookings.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 34 và 36; không cần phần bán hàng. Spa, salon, phòng khám, lớp học, sân thể thao.

- [x] Dịch vụ (thời lượng, đệm, sức chứa, cọc), nhân viên và phòng / thiết bị, lịch làm việc lặp và ngày nghỉ
      theo múi giờ của nút; tính giờ trống công khai
- [x] Đặt chỗ không bao giờ chồng giờ khi nhiều người đặt cùng lúc (ô giờ có khoá duy nhất, chạy trên cả hai
      database); giữ tạm khi trả cọc qua đối tác; huỷ, đổi giờ, vắng mặt theo chính sách
- [x] Nhắc lịch qua `notifications`, khách vãng lai đặt bằng OTP, `/me/bookings`, tool MCP; test trên cả hai
      database

## 61. Chat chăm sóc khách (`conversations`) (chủ dự án giao ngày 14/11/2026, D-221)

Đặc tả: [`docs/steps/61-conversations.md`](steps/61-conversations.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 40 (SSE) và 53 (`support`).

- [x] Cuộc trò chuyện giữa người mua và cửa hàng / người bán, gắn với sản phẩm hay đơn; tin nhắn chỉ thêm, gửi
      lại không trùng, thu hồi trong thời hạn ngắn; hộp thư chung, phân công, đã đọc — xong (19/11, D-20261119-step61-conversations)
- [x] Tin mới thời gian thực qua SSE, người không online nhận thông báo gom lô; chuyển thành phiếu hỗ trợ — xong
      (19/11, D-20261119-step61-conversations, D-20261119-step61-stream-audience)
- [x] Chống lạm dụng (chặn, báo vi phạm, xác thực tài khoản khi khách bật), hạn lưu là cấu hình; câu phức tạp:
      AI soạn nháp cho nhân viên; câu đơn giản đã có câu trả lời chính xác: trả lời tự động qua `AutoResponder`
      của `support` (D-230); test trên cả hai database — xong (19/11, D-20261119-step61-conversations)
- [x] Gọi điện trong chat (D-232): `VoiceProvider` + bộ giả, yêu cầu gọi lại vào hộp việc, nhật ký cuộc gọi,
      ghi âm tắt mặc định và phải thông báo trước; bộ nối thật cần tài khoản — dừng hỏi chủ dự án — xong
      (19/11, D-20261119-step61-voice; bộ nối thật chưa làm: chờ chủ dự án có tài khoản nhà cung cấp)

## 62. Sản phẩm bán theo kỳ (`recurring`) (chủ dự án giao ngày 14/11/2026, D-221) — **tạm dừng tới sau 1.0** (D-20261121-community-first)

Đặc tả: [`docs/steps/62-recurring.md`](steps/62-recurring.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 45 và 36. Người mua trả cho cửa hàng theo kỳ — khác `plans` (khách trả cho nền tảng, bước 35).

- [ ] Gói bán theo kỳ trên sản phẩm (chu kỳ, giá, dùng thử, số kỳ tối thiểu); mỗi kỳ tự tạo đơn và thu tiền qua
      đối tác, không tạo trùng khi job chạy lại; thanh toán lỗi thì thử lại rồi tạm dừng
- [ ] Người mua tự tạm dừng, bỏ qua kỳ, đổi sản phẩm, đổi phương thức, huỷ dễ như đăng ký; báo trước khi trừ tiền
      và khi đổi giá; không lưu số thẻ; số liệu MRR và tỉ lệ rời bỏ; test trên cả hai database

## 63. Đa tiền tệ và bán xuyên biên giới (chủ dự án giao ngày 14/11/2026, D-221) — **tạm dừng tới sau 1.0** (D-20261121-community-first)

Đặc tả: [`docs/steps/63-multi-currency.md`](steps/63-multi-currency.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 36, 41, 44, 45. Mở rộng module có sẵn, không module mới.

- [ ] Dùng tỉ giá có ngày hiệu lực của sổ cái (bước 36, D-237), thêm nguồn tỉ giá nếu cần; thị trường (quốc gia,
      tiền tệ, ngôn ngữ) theo website; giá theo thị trường, đơn chốt tỉ giá lúc đặt
- [ ] Sổ cái ghi số gốc và số quy đổi, chênh lệch tỉ giá; thuế GTGT 0% cho xuất khẩu khi đủ hồ sơ; người mua
      trong nước chỉ VND (Pháp lệnh Ngoại hối — cần đối chiếu); mã HS và xuất xứ cho giao hàng quốc tế; test trên
      cả hai database
- [ ] Người bán nước ngoài trên sàn (D-233): tắt mặc định, điều kiện (giấy tờ, đại diện, thuế) là gói luật có
      ngày hiệu lực, KYC hồ sơ nước ngoài, công bố quốc gia và nơi gửi hàng, chi trả ngoại tệ; hàng nhập khẩu
      cho người mua ngoài phạm vi

## 64. Quảng cáo trong sàn (`ads`) (chủ dự án giao ngày 14/11/2026, D-221) — **tạm dừng tới sau 1.0** (D-20261121-community-first)

Đặc tả: [`docs/steps/64-ads.md`](steps/64-ads.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 46 và 57 (`analytics`).

- [ ] Chiến dịch của người bán (vị trí, từ khoá / danh mục, CPC / CPM, giá thầu, ngân sách, lịch); chèn vị trí
      quảng cáo vào danh sách công khai qua điểm cắm của `catalog`, luôn có nhãn "Quảng cáo"
- [ ] Đếm hiển thị và bấm qua `analytics` (chống bấm giả), trừ ngân sách không vượt khi đồng thời; phí là doanh
      thu dịch vụ của sàn thu qua quyết toán hay hoá đơn, không giữ ví trả trước; báo cáo hiệu quả cho người bán;
      test trên cả hai database

## Dọn cho agent (việc ngoài bước, chủ dự án duyệt ngày 18/11/2026)

Làm Autoxide dễ đọc hiểu cho AI agent, cả khi phát triển repo lẫn khi dùng API. Không phải một bước của lộ trình;
không chặn bước nào.

- [x] Mục lục quyết định tự sinh (`cargo xtask decisions` → `docs/decisions/INDEX.md`, theo chủ đề, đánh dấu mục đã bị
      thay); gate báo khi lệch (D-20261118-decision-ids)
- [x] Quyết định mới mỗi mục một file, ID theo ngày và tên (`D-<YYYYMMDD>-<tên>`) để các phiên song song không chọn
      trùng; gate chặn ID trùng và tham chiếu tới ID không có (D-20261118-decision-ids)
- [x] OpenAPI theo module (`GET /api/v1/openapi/<module>.json`), endpoint MCP theo module (`/mcp/<module>`), `/llms.txt`
      trỏ tới cả hai; test trên cả hai database (D-20261118-module-contracts)
- [x] Gate cảnh báo file `.rs` trên 800 dòng, chặn trên 1200 trừ danh sách miễn không được dài thêm; tách sáu file dài
      nhất (D-20261118-file-length)

Ghi nhận, làm sau nếu đáng: `./gate.sh --quick` chậm khi sửa kernel — mọi crate phụ thuộc kernel nên `test-affected`
build và chạy test của cả workspace (ở đợt này, một lượt gate nhanh sau khi sửa `crates/kernel/src/http` lâu hơn nhiều
lần lượt chỉ sửa `xtask`, phần lớn là test e2e của binary). Hướng có thể xét: chỉ chạy test của crate phụ thuộc vào đúng phần
công khai bị đổi, hay tách phần ít đổi của kernel thành crate riêng. Chưa làm.

## Việc tồn đọng sau v0.15.0 (đánh giá ngày 20/11/2026)

Thứ tự ưu tiên chủ dự án đồng ý; mỗi mục khi làm thì ghi quyết định riêng.

- [x] Đăng nhập / tạo tài khoản bằng số điện thoại + OTP (tài khoản không bắt buộc email — thay đổi phá vỡ của
      `auth`, cần quyết định) — làm cùng bước 47 (chuyển từ bước 45) — D-20261120-phone-accounts (không phá vỡ:
      `email = ""`)
- [ ] Nối nhà cung cấp thật đầu tiên cho luồng bán chính: thanh toán, SMS, vận chuyển, hoá đơn điện tử (mỗi loại một
      đối tác; chủ dự án chọn đối tác và cấp tài khoản thử) — hiện mọi nhà cung cấp chỉ có bộ giả — **chuyển thành P7
      của bước 70**
- [ ] Ghép lô chạy thêm test Postgres cho crate bị sửa (lỗi đồng thời của #246 và luồng sự kiện chỉ lộ trên Postgres
      ở CI) — **xong phần này**: PR lô chờ CI đủ cả hai database rồi mới merge (D-20261121-ci-batch-release-faster); còn
      lại: rà mọi chỗ "đọc → so → ghi event" ở các module tìm chỗ thiếu khoá
- [ ] `/kernel/find`, `/kernel/work` bỏ nguồn chậm khi máy bận: đo lại hạn mỗi nguồn và để client hiện rõ phần thiếu
- [ ] Tách kernel (nút thắt build: sửa kernel phải build lại gần hết, xem D-20261120-build-faster)
- [ ] Phát hành thường hơn (1–2 tuần) cho CHANGELOG mỗi bản gọn (phát hành nay tự chạy khi merge PR phát hành,
      D-20261121-auto-release); cài `cargo-semver-checks` cho bước phát hành — **chuyển thành P4 của bước 70** (xong)
- [ ] Đăng SDK TypeScript lên npm (chờ tài khoản của chủ dự án) — **chuyển thành P5 của bước 70**
- [ ] Quét gitleaks định kỳ MỌI nhánh bằng một job riêng không chặn PR (sau D-20261125-gitleaks-scope, nhánh chưa có PR không được quét)
- [ ] Nợ nhỏ: tách các file `.rs` trên 800 dòng (đang làm); ~~`chrono` không còn bảo trì~~ — đã thay bằng `jiff`
      (D-20261121-time-library)

## Cấu hình theo từng khách (chủ dự án giao ngày 25/11/2026)

SaaS nhiều khách: thứ chỉ đặt được bằng biến môi trường chung (sửa `.env`, khởi động lại) phải đặt được theo từng khách;
biến chung vẫn là mặc định. Mỗi PR một cụm.

- [x] Link trong thư theo khách: mỗi link là một tenant setting nhóm `link` (`<module>.link.<trang>`, mẫu URL đầy đủ),
      host ràng vào tên miền đã xác minh / `kernel.public_url`, kiểm lúc đặt và lúc dùng; áp cho auth (5 link),
      support, compliance, inventory, notifications, comments, newsletter; luật gate `site-links`
      (D-20261125-tenant-link-templates)
- [x] Địa chỉ người gửi thư quảng cáo theo khách: cấu hình khách → hồ sơ pháp lý (`compliance`) → biến chung
      (D-20261125-tenant-postal-address)
- [x] `content.embed_hosts`, `analytics.allowed_hosts` theo khách (analytics vốn đã nhận tên miền đã xác minh)
- [ ] `content.embed_hosts` chặn hậu tố công cộng (`com.vn`, `github.io`): repo chưa có danh sách hậu tố công cộng
      (Public Suffix List); hiện chỉ cảnh báo trong mô tả setting
- [x] Link bài mới của newsletter mở trên tên miền của khách (`kernel.public_url`), như comments
- [x] P16: ràng `kernel.public_url` và link mẫu vào tên miền ĐANG xác minh của chính khách (hay host nền tảng) ở chế độ
      nhiều khách; giá trị cũ không thoả bị bỏ qua và báo trong `GET /kernel/settings` (D-20261125-tenant-site-verified)
- [x] Email bị dội: sổ dội và danh sách chặn cho mọi thư trong module `mail` (dội cứng SMTP 5.1.x / 5.2.x lúc gửi), tra
      theo địa chỉ, chặn sau `AUTOXIDE_MAIL_HARD_BOUNCE_LIMIT` lần, gỡ chặn, số liệu theo khách, thời hạn giữ
      (D-20261125-mail-bounces)
- [ ] Thư dội báo về sau (DSN) và webhook báo dội của nhà cung cấp mail cho mọi thư — hiện chỉ đọc lỗi lúc gửi SMTP
- [ ] Gộp danh sách chặn riêng của `newsletter` vào sổ dội của `mail` (một nguồn) — newsletter còn nhận dội mềm /
      khiếu nại qua `POST /newsletter/bounces`, đổi nguồn cần chuyển dữ liệu
- [ ] Xoá sổ dội theo tài khoản bị xoá: `auth.user_deleted` chỉ mang ID, không mang email — cần thêm email vào event
      (module `auth`); hiện xoá theo địa chỉ (`DELETE /mail/bounces/{id}`) và theo thời hạn giữ
- [x] Hạn mức token AI mỗi ngày theo gói (`ai.daily_tokens`; gói không nêu thì biến chung)
- [x] Giới hạn request mỗi phút theo khách (tầng thứ ba, `kernel.api_rate_per_minute` của gói hay biến chung; REST và
      MCP; D-20261126-tenant-rate-limit)
- [x] Ngôn ngữ của `catalog` và `menus` theo khách (`catalog.locales`, `menus.locales`, `menus.default_locale`); múi
      giờ / tiền tệ đã có theo khách qua `organizations.timezone` / `organizations.currency` của nút gốc (mặc định
      `UTC` khai ở `organizations`; `promotions` / `shipping` dùng giờ Việt Nam theo quyết định sẵn có; "VND" viết
      cứng ở `bookings` chỉ nằm trong test) — không thêm setting mới
- [x] `mail.delivery_failed` mang `enhanced` / `hard`; newsletter chỉ tự chặn người đăng ký khi `hard`
- [x] Tự đăng ký tài khoản nhân sự theo khách (`auth.staff_registration` ở nút gốc; REST và đăng nhập OIDC lần đầu;
      chưa đặt thì `AUTOXIDE_AUTH_SELF_REGISTRATION`) — D-20261126-tenant-login-policy
- [x] Passkey theo khách: RP ID là tên miền ĐANG xác minh của khách (`auth.passkey_rp_id`, loại cài đặt `own_domain`
      của kernel, kiểm lúc đặt và lúc dùng; tên miền chuyển khách thì passkey trên đó hết dùng ở khách cũ) —
      D-20261126-tenant-passkey-rp
- [ ] Passkey đăng ký trên RP của máy chủ dùng song song với RP của khách (lưu RP vào từng passkey): hiện đổi RP làm
      passkey cũ hết dùng được
- [x] Chính sách đăng nhập theo khách, chỉ siết hơn máy chủ: `auth.password_min_length`, `auth.session_days`,
      `auth.totp_issuer` — D-20261126-tenant-login-policy
- [x] Hạn mức `kernel.api_requests`: đếm sau giới hạn theo khách, chỉ chặn người gọi của khách; nhớ xác thực thất bại
      trong request — D-20261126-api-quota-signed-in
- [x] Tên miền của khách (`TenantSetting::own_domain`): từ chối địa chỉ IP, cả dạng nhãn cuối hex `0x…` mà trình duyệt
      đọc thành IPv4
- [ ] MCP `resources/read` và `prompts/get` chưa tính hạn mức `kernel.api_requests` và giới hạn request theo khách
      (chỉ `tools/call` tính)

Chỗ trong core làm cấu hình theo khách khó (rút ra khi làm đợt này; mỗi mục là việc nên làm, chưa có quyết định):

- [ ] Hai cơ chế cài đặt theo khách song song — cài đặt khách của kernel (`kernel.settings`) và cài đặt theo cây của
      `organizations` — chưa có quy tắc chọn: ghi quy tắc (vd cấu hình kỹ thuật ở kernel, chính sách nghiệp vụ theo
      cây) hay gộp về một chỗ
- [ ] Kernel chưa kiểm chéo giữa hai cài đặt lúc lưu (vd ngôn ngữ mặc định phải thuộc danh sách ngôn ngữ): hiện bỏ
      qua lúc dùng thay vì từ chối lúc lưu
- [ ] Cài đặt theo cây của `organizations` không có cache: mỗi lần đọc (đăng nhập, làm mới phiên…) là một truy vấn
- [ ] Khách của request trước khi xác thực lấy từ gợi ý chưa xác thực (`Autoxide-Tenant`, `Host`) qua cùng một hàm với
      khách đã xác thực: dễ dùng nhầm cho hạn mức hay chính sách (lỗi đã sửa ở D-20261126-tenant-rate-limit) — tách hai
      hàm rõ tên
- [ ] Passkey không lưu RP ID của nó: không đổi RP mềm được (cùng mục passkey ở trên)

## Chờ chủ dự án

## 50. Công khai repo (chờ chủ dự án, D-175, D-197, D-199)

Chủ dự án giữ repo riêng tư (D-199). **Agent không làm bước này khi chủ dự án chưa nói repo đã
công khai**; khi đó các bước khác vẫn đi tiếp bình thường. Workflow đã viết sẵn (D-197), công
khai là tự chạy.

- [ ] Chủ dự án công khai repo (Settings → Change visibility); ngay trước đó agent quét lại
      gitleaks toàn lịch sử và tìm tên máy chủ, IP, đường dẫn riêng, thông tin cá nhân. Agent không
      tự đổi chế độ công khai
- [ ] Chủ dự án bật trong Settings → Code security: secret scanning, push protection, Dependabot
      alerts, private vulnerability reporting (`SECURITY.md` dựa vào nó); gitleaks trong `./gate.sh`
      giữ nguyên
- [ ] `CI` và `Security audit` chạy xanh trên GitHub
- [ ] Bản phát hành kế tiếp qua workflow `Release`: image amd64 + arm64 đăng lên `ghcr.io`, chủ
      repo đổi gói image sang Public một lần; `gh attestation verify` xác nhận chứng thực nguồn gốc
      và SBOM; `docs/RELEASING.md` bỏ phần "Khi repo còn riêng tư"
- [x] Thử cài như người lạ bằng image đã đăng (cả SQLite và Postgres), sửa README /
      `docs/DEPLOY.md` (EN + VI cùng commit) theo chỗ vấp

## 69. Nền cho ứng dụng đa nền tảng: di động, ngoại tuyến, thời gian thực hai chiều (chủ dự án giao ngày 19/11/2026, D-20261119-step69-multiplatform)

Đặc tả: [`docs/steps/69-multiplatform.md`](steps/69-multiplatform.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Core chỉ có API nên app điện thoại, app máy tính, máy bán hàng tại quầy đã dùng được phần lớn; bước này lấp phần còn
thiếu. Các bước nền đã xong: bắt đầu được ngay, song song với 41–49.

- [x] ~~SDK Swift, Kotlin, Dart~~ — **đã bỏ** theo quyết định của chủ dự án (D-20261121-drop-mobile-sdk): chỉ giữ SDK
      TypeScript (web, React Native, Node); app native / Flutter tự sinh client từ `docs/openapi.json`
- [x] Đồng bộ khi mất mạng: `Module::sync_resources()`, luồng thay đổi theo con trỏ có dấu xoá
      (`<module>_changes` ghi cùng transaction), ghi có điều kiện `If-Match` → `412` kèm bản hiện tại, lô ghi
      `POST /kernel/sync/batch` qua đúng service như REST, con trỏ quá cũ trả `reset`; test đồng thời cả hai database
- [x] WebSocket hai chiều `GET /kernel/events/ws` dùng chung `StreamHub` và bộ lọc quyền của SSE, nối lại không mất
      event, chỉ nhận lệnh điều khiển (đăng ký, đang gõ, online) — ghi dữ liệu vẫn qua REST
- [x] Mua trong ứng dụng: trait `StorePurchases` (App Store Server API, Google Play Developer API, thông báo máy chủ
      có kiểm chữ ký) + bộ giả, `plans` nối vào thuê bao và quyền dùng, gọi lại không cấp trùng; tiền do Apple /
      Google giữ (D-187); căn cứ thuế ghi trong quyết định; bộ nối thật cần tài khoản — dừng hỏi chủ dự án
- [x] Hướng dẫn `docs/guides/mobile.md` + `.vi.md`: đăng nhập và làm mới token, PKCE cho app, thông báo đẩy, tải ảnh
      thẳng lên S3, nhận event, đồng bộ ngoại tuyến, mua trong ứng dụng, dùng SDK; ví dụ chạy như test

## Sau core (chưa mở)

GraphQL · tách service — xem bảng "Chưa làm, đã cân nhắc" trong `STACK.md`.

Đã đưa lên lộ trình (D-221): bán sỉ B2B (bước 59), đặt lịch dịch vụ (60), chat chăm sóc khách (61), sản
phẩm bán theo kỳ (62), đa tiền tệ (63), quảng cáo trong sàn (64), chợ ứng dụng OAuth (65), SCIM (66).

Đã đưa vào các bước (D-232): khách thử nghiệm và nhóm nút cho báo cáo (38), AVIF (39), trả trước một phần (45),
quản trị trên điện thoại (52), duyệt từng lần truy cập hỗ trợ và "xem như" chỉ đọc (54), hợp đồng khung, mời thầu và
ký điện tử (59), gọi điện trong chat (61), bán ứng dụng trả phí (65).

Đã viết đặc tả (D-233): hợp đồng tool MCP và `cargo semver-checks` (31), báo yêu thích (47), banner gian hàng
người bán (55), EDI (59), người bán nước ngoài (63). Sau đó: chủ đề theo lịch, đầu / chân trang (67, D-234); trường
tự định nghĩa, nhãn, đổi câu chữ (51), loại nội dung (67), mã theo dõi và đồng ý cookie (37) (D-235); kiểu vị trí
banner (55, D-236); rà lộ trình lần hai (D-237); module `menus` cho menu của website (68, D-238).

Đã đưa lên lộ trình (D-20261119-step69-multiplatform): nền cho ứng dụng đa nền tảng — SDK di động, đồng bộ
ngoại tuyến, WebSocket, mua trong ứng dụng, hướng dẫn cho người làm app (69).

Để sau: ý tưởng mới ghi một dòng ở đây, chưa viết đặc tả, cho tới khi xong giai đoạn A.

- Rà 139 danh sách `Data<Vec<T>>` trong `xtask/api-shape-allow.txt` (D-20261121-api-shape): danh sách nào không có
  giới hạn thật (vd nhãn, view, ghi chú, biến thể, hoàn tiền của đơn) thì làm endpoint mới trả `Page<T>` và khai
  endpoint cũ qua `Module::deprecations` (D-059).

Đã cân nhắc và không làm: API dữ liệu tự
sinh từ bảng, plugin WASM nạp lúc chạy, trang quản trị dựng sẵn (`docs/research/frameworks.md`,
D-005, D-026). Thư viện số thập phân và module sổ cái: đã quyết ở D-187, D-188.
