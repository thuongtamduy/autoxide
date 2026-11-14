# Lộ trình

Mỗi bước xong khi `./gate.sh` xanh trên **cả SQLite và Postgres**; agent đánh dấu `[x]` rồi commit. Chi tiết
thư viện: [`STACK.md`](../STACK.md). Cách làm từng bước: [`docs/steps/`](steps/README.md).

Trạng thái: **bước 0–30 xong** — đã chuyển sang [`docs/roadmap/done.md`](roadmap/done.md) (D-228); bước 50 chờ
chủ dự án công khai repo (D-199); bản phát hành mới nhất `v0.12.0`. CI trên GitHub chỉ chạy khi repo công khai
(D-197, D-199): `./gate.sh` cục bộ là gate duy nhất.

**Số bước là mã cố định, không phải thứ tự** (D-228): bước mang số lớn vẫn có thể làm trước. Các bước dưới đây
xếp theo **thứ tự làm**; bước kế tiếp là bước đầu tiên (từ trên xuống) còn mục `[ ]` mà các bước nó phụ
thuộc đã xong.

## Thứ tự làm (D-228)

| Giai đoạn | Bước, theo thứ tự | Vì sao ở đây |
|---|---|---|
| A. Hợp đồng và nền lõi | 31, 32 → 33 → 34 → 35, 36, 37 (song song) → 40 | giữ API không vỡ trước khi sửa mọi bảng; đa khách, tổ chức, tiền, tuân thủ là nền của mọi thứ sau |
| B. Lớp cho giao diện và phân quyền | 51 → 54 → 52 | module thương mại khai luôn phần của mình (51 nên xong trước 41); 54 cần P12 của 51 |
| C. Vận hành | 38 → 39 | báo cáo xuyên database, backup, hiệu năng trước khi có dữ liệu thương mại thật |
| D. Module nền dùng chung | 53, 57, 58, 66, 65 | hỗ trợ khách (45 và 61 dùng), phân tích (64 dùng), tự động hoá, SCIM, chợ ứng dụng |
| E. Thương mại | 41 → 42, 43, 44 → 45 → 46, 47, 48, 49, 55 → 56, 59, 60, 61, 62, 63, 64 | module nghiệp vụ cắm vào core đã xong |
| Chờ chủ dự án | 50 | chỉ làm khi chủ dự án nói repo đã công khai |

Bước lớn nhất là 33 (sửa mọi bảng của mọi module): một người làm, không chia song song. Trong một giai đoạn,
bảng phụ thuộc ngay dưới cho biết bước nào chạy song song được.

## Phụ thuộc giữa các bước (D-192)

| Bước | Bắt đầu khi | Song song được với |
|---|---|---|
| 31 Hợp đồng API, chống hồi quy | ngay | 30, 32 |
| 32 Dọn bố cục | ngay; `media` của nhóm A làm sau khi WebP (30) merge | 30, 31 |
| 33 Khách hàng, vị trí dữ liệu | 31 và nhóm A của 32 xong (31 giữ API và bản nâng cấp không vỡ khi mọi bảng đổi) | hai mục độc lập của 40 |
| 34 Tổ chức, quyền, danh tính | 33 | — |
| 35 Gói, thuê bao · 36 Tiền, thuế · 37 Tuân thủ | 34 | nhau; hai mục cuối của 35 chờ 36 và 37 |
| 38 Xuyên database, báo cáo gộp | 34 và 36 | 39, 40, 41–45 |
| 39 Vận hành, hiệu năng | 38 | 40, 41–46 |
| 40 Năng lực còn thiếu | header bảo mật và MCP: ngay; còn lại: 34 | 35–39 |
| 41 Sản phẩm | 35 và 36 | 37–40 |
| 42 Kho · 43 Khuyến mại · 44 Vận chuyển | 41 | nhau |
| 45 Giỏ hàng, đặt hàng, đơn hàng | 42, 43, 44 và 37 (thông báo) | — |
| 46 Sàn · 47 Khách hàng · 48 Đa kênh · 49 Tiếp thị liên kết | 45 (46 và 49 cần thêm 37) | nhau |
| 50 Công khai repo | chủ dự án nói đã công khai repo (D-199) | mọi bước |
| 51 Hợp đồng cho giao diện (D-200) | nhúng bản ghi: sau 33; hộp việc, tìm, số liệu, hoạt động: sau 34; menu: sau 35 | 36–49 — nên làm trước 41 để module thương mại khai luôn phần của mình |
| 52 Trải nghiệm quản trị nâng cao (D-201) | 51 | 41–49 |
| 53 Hỗ trợ khách hàng (D-204) | 37 (thông báo, vụ việc khiếu nại) | 41–49 |
| 54 Phân quyền chuẩn hàng đầu (D-205) | 34; P6, P7 cần P12 của 51; P1 làm ở bước 31 | 41–49, 53 |
| 55 Giao diện cửa hàng (D-219) | 41 (banner và thông tin cửa hàng làm được từ sau 34) | 42–49 |
| 56 Chuyển dữ liệu từ nền tảng khác (D-219) | 47 | 48, 49 |
| 57 Phân tích truy cập (D-220) | 37 (đồng ý), 34 | 41–49 |
| 58 Tự động hoá quy trình (D-220) | 51 (P12 duyệt, P15 job) | 41–49, 57 |
| 59 Bán sỉ B2B (D-221) | 45, 47 | 46, 48, 49 |
| 60 Đặt lịch dịch vụ (D-221) | 34, 36 (37 cho nhắc lịch) | 41–49 |
| 61 Chat chăm sóc khách (D-221) | 40 (SSE), 53 | 41–49 |
| 62 Sản phẩm bán theo kỳ (D-221) | 45, 36 | 46–49 |
| 63 Đa tiền tệ (D-221) | 36, 41, 44, 45 | 46–49 |
| 64 Quảng cáo trong sàn (D-221) | 46, 57 | 47–49 |
| 65 Chợ ứng dụng OAuth (D-221) | 54, 33 | 41–49 |
| 66 SCIM (D-221) | 34, 40 (OIDC) | 35–49 |

Bước lớn nhất là 33 (sửa mọi bảng của mọi module): một người làm, không chia song song.

## Giai đoạn A — Hợp đồng và nền lõi

## 31. Hợp đồng API và chống hồi quy (chủ dự án giao ngày 13/11/2026, D-177)

Đặc tả: [`docs/steps/31-api-contract.md`](steps/31-api-contract.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Người viết giao diện (repo riêng, D-097), nhà phát triển ngoài và AI agent dùng API không gọi sai;
hợp đồng, tài liệu và database của bản cũ không bao giờ âm thầm lệch khỏi code. Bước này sửa
`xtask`, `testkit`, `gate.sh`, tài liệu và thêm thư mục SDK — làm ngay được; riêng test nâng cấp
cần image Docker của bản cũ (bước 30). Test chạy trên cả SQLite và Postgres; thư viện chỉ cho test
khai ở `[dev-dependencies]`.

Nhóm A — hợp đồng và người gọi API:

- [ ] Gate chặn thay đổi phá vỡ của `/api/v1` (D-059): so `docs/openapi.json` với bản ở tag phát
      hành gần nhất (`git show vX.Y.Z:docs/openapi.json`) và báo đỏ khi xoá endpoint hay method,
      thêm tham số hay trường bắt buộc vào request, xoá trường hay đổi kiểu trong response, bỏ
      giá trị của enum. Endpoint đã khai deprecated và quá ngày `sunset` thì được bỏ. Viết bằng
      Rust trong `xtask` (thêm lệnh), có test cho từng loại thay đổi; thông báo nói rõ chỗ nào
      phá và cách làm đúng (endpoint mới + deprecate endpoint cũ)
- [ ] SDK TypeScript sinh từ `docs/openapi.json` (D-177: trong repo, không đăng npm), đủ kiểu cho
      request, response, lỗi `Problem` và pagination; gate kiểm SDK khớp hợp đồng như cách test
      đang kiểm `openapi.json`. Gate không bắt người lạ cài Node: nếu công cụ sinh cần Node thì chạy
      qua image Docker ghim phiên bản (như gitleaks) hoặc chỉ kiểm trong CI — chọn và ghi quyết định
- [ ] Hướng dẫn theo tình huống (EN + VI, thư mục mới trong `docs/`): website đọc bài và ảnh
      (`srcset`, `body_media`, ETag / 304, cache), đăng nhập và làm mới token, API key cho máy,
      gọi lại an toàn bằng `Idempotency-Key`, pagination, đọc lỗi (`code`, `errors`, `Retry-After`).
      Mỗi tình huống có ví dụ `curl` và TypeScript dùng SDK
- [ ] Hướng dẫn nối AI agent qua MCP (EN + VI), mở rộng đoạn ngắn trong README: tạo API key quyền
      tối thiểu cho agent, cấu hình Claude Code và client MCP khác, danh sách tool theo module và
      permission cần, giới hạn (rate limit theo API key, bản nháp chờ duyệt — D-109)
- [ ] Ví dụ trong tài liệu được test: các ví dụ của hai mục trên chạy thật trên app dựng bằng
      `TestApp` trong gate (hay một lệnh `xtask` gate gọi), để tài liệu không lệch code

Nhóm B — chống hồi quy:

- [ ] Test nâng cấp từ bản phát hành trước: dựng database bằng image Docker của bản trước, ghi dữ
      liệu mẫu qua API của nó, rồi chạy bản hiện tại trên database đó (migration, đọc lại dữ liệu,
      vài lệnh ghi) — trên cả SQLite và Postgres; chạy trong CI và trước mỗi lần phát hành
- [ ] Migration thật của mọi module chạy vòng up → down → up trên database trống và trên database
      có dữ liệu, trên cả hai database (hiện chỉ có test cơ chế bằng bảng mẫu,
      `crates/kernel/tests/migration.rs`)
- [ ] Property test cho các bộ kiểm đầu vào: thân bài ProseMirror (`content`), slug, email
      (`autoxide_kernel::email`), con trỏ pagination, CSV chống injection, chữ và link của bình
      luận (chống XSS) — sinh đầu vào ngẫu nhiên, không panic, giữ bất biến; thêm thư viện thì ghi
      `STACK.md`
- [ ] Response thật khớp hợp đồng: test HTTP qua `TestApp` kiểm mỗi response (thân và mã) khớp
      schema của endpoint đó trong `docs/openapi.json`, để tài liệu không hứa điều code không làm
- [ ] Hợp đồng quyền đọc được bằng máy (D-205, đặc tả 54 mục P1): mỗi operation OpenAPI có
      `x-permissions` hay `x-public`, sinh từ một khai báo cạnh handler; `Permission` có mức rủi ro
      (`read` / `write` / `destructive` / `sensitive_data` / `money`)
- [ ] Test độ phủ phân quyền trong gate (D-205): mọi operation không công khai trả 401 khi không token và
      403 khi token không có quyền; operation thiếu khai báo quyền làm gate đỏ; tool MCP `read_only` chỉ
      đòi quyền mức `read`

## 32. Dọn bố cục và việc để lại (agent đề xuất ngày 13/11/2026)

Đặc tả: [`docs/steps/32-layout-cleanup.md`](steps/32-layout-cleanup.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bước 29 đã merge — làm được ngay; riêng dời file của `media` làm sau khi WebP (bước 30) merge vì
cùng sửa module đó. Làm nhóm A trước: nhóm B sửa đúng
các module nhóm A dời file. Mỗi mục có test trên cả hai database và dòng CHANGELOG.

Nhóm A — bố cục:

- [ ] Áp quy ước "File layout" của hướng dẫn viết module cho `newsletter`, `comments`, `media`,
      `forms`: phần service chia theo tính năng, test dài thành thư mục `tests/`; không đổi hành vi,
      đếm số test trước và sau phải bằng nhau
- [ ] `kernel`: gom các file nằm phẳng trong `src/` thành nhóm (vd hợp đồng cho module: `ai`,
      `mail`, `storage`, `search`, `privacy`…), giữ nguyên đường dẫn công khai bằng `pub use` để
      module ở repo riêng không phải sửa (D-167)

Nhóm B — việc nhật ký quyết định đã ghi "để sau":

- [ ] `auth`: đổi email thì gửi thêm mail báo tới địa chỉ cũ (kèm cách báo "không phải tôi" — vd
      khoá phiên và đặt lại mật khẩu), để chủ tài khoản biết khi tài khoản bị chiếm (đã ghi để sau
      ở mục về xác minh email trong `docs/DECISIONS.md`); mail qua job, không làm chậm request
- [ ] `auth`: quản trị viên xem và gỡ passkey của người khác (permission riêng, ghi event cho
      audit), và lệnh CLI gỡ passkey của một tài khoản cho khi admin duy nhất mất thiết bị —
      `autoxide user reset-mfa` (D-159) hiện chỉ tắt TOTP, không gỡ passkey
- [ ] `forms`: xuất CSV lớn bằng job ghi file vào `Storage`, trả link tải có hạn; xuất đồng bộ hiện
      tại giữ cho số bài nhỏ (cách xuất đang có: `modules/forms/src/csv.rs`)
- [ ] `comments`: Markdown giới hạn (đậm, nghiêng, link, trích dẫn, code) với bộ lọc HTML chống
      XSS — thêm thư viện thì đo và ghi `STACK.md`; giữ test XSS sẵn có; chữ thường cũ hiển thị như cũ

Chờ chủ dự án quyết: định dạng AVIF cho ảnh thu nhỏ (cần thư viện mã hoá AVIF, nặng hơn WebP).

## 33. Nền đa khách: khách hàng, vị trí dữ liệu, mặt điều khiển (D-185)

Đặc tả: [`docs/steps/33-tenancy.md`](steps/33-tenancy.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Thiết kế: `docs/research/business-foundation.md`, trụ 1. Bắt đầu SAU bước 31 và nhóm A của bước 32.
Làm theo ba nhóm A (nền kernel), B (chuyển 13 module, mỗi module một commit), C (vận hành) của đặc tả;
một người làm tuần tự. Mọi mục chạy và có test trên cả SQLite và Postgres, ở cả chế độ `single` và nhiều khách. Ở chế độ
`single`, `/api/v1` không đổi (gate chặn thay đổi phá vỡ của bước 31 kiểm) và bản cài cũ nâng cấp
được: migration gán mọi dữ liệu có sẵn cho khách mặc định, test nâng cấp của bước 31 phải xanh.

- [ ] Mặt điều khiển: sổ khách hàng (trạng thái, khách cha), vị trí dữ liệu (`shared` /
      `dedicated`, database nào), tên miền → khách / nút; chế độ `single` mặc định (một khách,
      mặt điều khiển và dữ liệu chung một database, không cấu hình gì thêm)
- [ ] Ngữ cảnh khách hàng cho mọi request: suy từ tên miền, API key, access token, header của nhân
      sự nền tảng; sổ kết nối database theo khách (mở lười, có giới hạn); extractor cho handler,
      tool MCP; thiếu ngữ cảnh ở route cần khách thì lỗi rõ
- [ ] Phân loại bảng: mặt điều khiển hay cấp khách hàng; mọi bảng cấp khách hàng của 13 module có
      `tenant_id` (migration mới), khoá unique và index bắt đầu bằng nó; testkit và `cargo xtask
      check` chặn bảng cấp khách hàng thiếu `tenant_id`
- [ ] Mọi truy vấn của service lọc theo khách qua một công cụ của kernel; testkit có bài **rò rỉ
      giữa khách** chạy cho mọi module (ghi ở khách A, đọc bằng người của khách B: không thấy gì)
- [ ] Job, event, outbox, lịch chạy theo từng database; worker và bộ phát event duyệt mọi database
      được giao; `autoxide migrate up --all` cho cả đội database; dời một khách `shared` ↔ `dedicated`
      bằng chép dữ liệu (mở rộng `autoxide copy`)
- [ ] Đo đường nóng trước và sau (`scripts/loadtest.sh`); hướng dẫn viết module (EN + VI) có quy
      tắc phân loại bảng và lọc theo khách
- [ ] Mã hoá dữ liệu nhạy cảm khi lưu (dời từ bước 37, D-190 — kho secret theo khách cần nó):
      secret TOTP của MFA (hiện lưu chữ thường trong `auth`), secret ký webhook và mọi secret module cần đọc lại được — mã hoá bằng khoá nằm ngoài
      database (dẫn xuất từ `AUTOXIDE_SECRET_KEY`), có số phiên bản khoá để xoay khoá; một công cụ
      dùng chung trong `kernel`, thuật toán chuẩn (AEAD) qua thư viện đã kiểm; migration mã hoá dữ
      liệu cũ; bản backup không còn chứa các secret này ở dạng đọc được. Khi `AUTOXIDE_SECRET_KEY`
      chưa đặt thì khoá gốc nằm trong database (`kernel/src/secret.rs`), mã hoá không bảo vệ được
      bản backup: `autoxide doctor` và log lúc khởi động nói rõ điều đó. Ghi quyết định
- [ ] Cấu hình và secret theo khách (D-190): thông tin kết nối đối tác của từng khách (cổng thanh
      toán, SMTP, hoá đơn điện tử, khoá AI) lưu mã hoá bằng cơ chế trên; module đọc qua một trait
      của kernel, không đọc biến môi trường cho thứ thuộc về khách
- [ ] Mọi thứ ngoài bảng cũng theo khách (D-190): khoá file lưu trữ có tiền tố khách (khách
      `dedicated` được bucket riêng); cache trong bộ nhớ (vd cache tìm theo ý nghĩa của `content`),
      rate limiting, login throttling, idempotency key, con trỏ pagination có khách trong khoá;
      token và URL đã ký mang khách, dùng ở khách khác thì bị từ chối
- [ ] Tên miền riêng của khách (D-190): xác minh quyền sở hữu bằng bản ghi DNS TXT, trạng thái tên
      miền, endpoint "có phục vụ tên miền này không" cho reverse proxy cấp chứng chỉ TLS tự động
      (vd Caddy on-demand TLS); tài liệu deploy (EN + VI)
- [ ] An toàn khi nhiều database (D-190): mỗi database ghi phiên bản migration của nó; database
      chưa migrate xong thì chỉ các khách trên nó nhận lỗi `tenant_maintenance` (503), các khách khác
      chạy bình thường; `migrate --all` chạy lại được từ chỗ dừng; worker chia lượt công bằng giữa
      các khách trong database chung (một khách không chiếm hết job queue)
- [ ] Đối chiếu thiết kế 2 (D-210, D-218 mục 33): cấu hình nghiệp vụ của module theo khách (`Module::tenant_settings`
      cho `comments`, `forms`, `content`, `media`); `GET /kernel/tenants` có lọc và `expand`; API job di chuyển
      database chỉ cho nhân sự nền tảng, dry-run mặc định; tách "tạm khoá" (chặn mọi route) khỏi "chỉ đọc"
- [ ] Đối chiếu đợt 2 (D-223…D-227): gắn tên miền với nút; cấu hình kết nối đối tác theo nhóm `connection` với
      tên thống nhất

## 34. Tổ chức, quyền theo độ sâu, danh tính (D-185)

Đặc tả: [`docs/steps/34-organizations-identity.md`](steps/34-organizations-identity.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Thiết kế: trụ 2. Bắt đầu SAU bước 33.

- [ ] Cây tổ chức trong khách hàng: loại nút tự đặt, độ sâu có giới hạn, bảng tổ tiên – con cháu,
      dời nút (khoá dòng), vòng đời nút (chờ duyệt, hoạt động, tạm khoá, lưu trữ — khoá một nút thì
      cả nhánh ngừng ghi)
- [ ] Thành viên và quyền có độ sâu `own` / `unit` / `subtree` / `tenant`; `Principal` mang quyền
      theo nút; công cụ lọc theo nhánh cho service; dữ liệu nghiệp vụ mang `org_id`; module mẫu
      `notes` dùng làm ví dụ; testkit có bài rò rỉ giữa hai nhánh anh em
- [ ] Cài đặt kế thừa theo cây (múi giờ, tiền tệ, ngôn ngữ, hồ sơ thuế, chính sách) và khuôn mẫu
      để module áp cùng cách cho dữ liệu của mình (vd giá: chuỗi đặt, cửa hàng ghi đè)
- [ ] Danh tính hai lớp: nhân sự là tài khoản toàn nền tảng (mặt điều khiển), một người làm cho
      nhiều khách / nút; người mua là tài khoản theo từng khách; API key gắn với khách và nút.
      Nâng cấp: tài khoản, role và API key có sẵn thành nhân sự của khách mặc định, giữ quyền cũ
- [ ] Địa chỉ và đơn vị hành chính Việt Nam (module nền `address`, D-193): chỉ danh mục sau sáp nhập
      — hai cấp tỉnh / thành → xã / phường / đặc khu, 34 tỉnh và 3.321 xã theo QĐ 19/2025/QĐ-TTg,
      hiệu lực 01/7/2025; không có cấp huyện, không đối chiếu địa chỉ cũ. Dữ liệu dùng chung mọi
      khách (mặt điều khiển), lấy từ nguồn chính thức, ghi nguồn và ngày; mỗi đơn vị có mã, tên, loại,
      mã cấp trên, ngày hiệu lực từ / tới; cập nhật bằng task khi nhà nước đổi (không sửa code).
      Kiểu `Address` trong `api` (quốc gia, mã tỉnh, mã xã, dòng địa chỉ, kèm tên tại thời điểm ghi);
      kiểm mã xã thuộc đúng tỉnh; endpoint công khai danh sách tỉnh, xã theo tỉnh, tìm theo tên (không
      dấu cũng tìm được), có ETag / cache. Nút tổ chức (cửa hàng, kho, người bán), hồ sơ KYC và hồ sơ
      pháp lý (bước 37), sổ địa chỉ người mua (bước 44) đều dùng kiểu này
- [ ] Uỷ quyền theo cây khách hàng: role ở khách cha có hiệu lực ở các khách con khi được cấp; vào
      ngữ cảnh khách con bằng cùng API; audit ghi ở cả hai phía
- [ ] Đăng nhập bằng link qua email (không cần mật khẩu): token dùng một lần, hết hạn ngắn, gắn
      thiết bị yêu cầu khi được; MFA vẫn áp dụng; trả lời như nhau về thời gian dù email có hay
      không (như D-154, D-162); tắt mặc định, bật bằng cấu hình
- [ ] Mời người dùng qua email: quản trị viên mời kèm role định sẵn, người được mời đặt mật khẩu
      (hay passkey) qua link dùng một lần; lời mời hết hạn, thu hồi được, có event cho audit; lệnh
      CLI tương ứng
- [ ] Hồ sơ công khai của nút (D-200): tên hiển thị, địa chỉ (`Address`), giờ mở cửa theo ngày, có
      nhận hàng tại quầy hay không, tên miền gắn với nút; endpoint công khai danh sách cửa hàng của khách
      theo tên miền (chỉ nút bật "hiện công khai"), có ETag / cache — website dùng cho "cửa hàng gần bạn"
      và chọn nơi nhận hàng
- [ ] Nhân sự nền tảng vào dữ liệu của khách (D-190): chỉ khi có quyền vào cấp có thời hạn, ghi lý
      do; mọi lần vào ghi audit mà quản trị của khách xem được (Luật BVDLCN 2025); khách tắt được
      quyền vào hỗ trợ, trừ yêu cầu của cơ quan có thẩm quyền (có audit)
- [ ] Tạo tài khoản từ số điện thoại đã xác thực qua tra đơn vãng lai (D-202; làm cùng bước 45 khi
      `orders` có token xem đơn): `POST /api/v1/auth/phone-signups`, đăng nhập bằng passkey hay OTP, không
      bắt buộc mật khẩu
- [ ] Đối chiếu thiết kế 2 (D-208, D-211): `org_id` cho `content`, `media` (`forms` tuỳ chọn), quyền theo độ
      sâu, công khai theo website; xác nhận lại danh tính `POST /auth/me/reauth` thay `current_password`;
      ngôn ngữ và múi giờ ưa thích; khách tự thu hồi quyền vào hỗ trợ; `memberships?inherited=true`;
      bắt buộc MFA theo role, `last_login_at"
- [ ] Đối chiếu đợt 2: ghi hồ sơ công khai của nút (giờ mở cửa, ngày đặc biệt, ảnh); tự đổi tên hiển thị qua
      `PATCH /auth/me`

## 35. Gói, quyền dùng tính năng, hạn mức, thuê bao (D-185)

Đặc tả: [`docs/steps/35-plans-subscriptions.md`](steps/35-plans-subscriptions.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Thiết kế: trụ 3. Bắt đầu SAU bước 34; mục tính tiền chờ `PaymentGateway` của bước 36, mục cấp phát
chờ KYC của bước 37 — làm hai mục đó sau cùng. Giá, kỳ thanh toán, hạn mức là dữ liệu, không phải code.

- [ ] Module khai tính năng (`Module::features`) và hạn mức (`Module::limits`), mặc định rỗng
      (D-038); gói = tập module + tính năng + hạn mức + giá theo kỳ; dùng thử, gói thêm, ghi đè
      riêng cho từng khách; API và tool MCP quản trị gói
- [ ] Kernel cưỡng chế theo khách: module chưa bật thì route, tool MCP, job, event của nó bị chặn
      (`module_not_enabled`); `has_feature`; hạn mức giữ chỗ / trả lại nguyên tử; số đo dùng ghi
      lại cho tính tiền; test đồng thời trên cả hai database
- [ ] Vòng đời thuê bao: dùng thử → đang dùng → quá hạn (ân hạn) → tạm khoá (chỉ đọc) → huỷ (cho
      xuất dữ liệu) → xoá sau thời hạn lưu; thu hồi module giữ dữ liệu theo chính sách lưu; gia hạn
      và nhắc nợ bằng job
- [ ] Tính tiền thuê bao SaaS: hoá đơn theo kỳ, đánh số tuần tự, thu qua `PaymentGateway` (bước 36)
      bằng tài khoản của nhà vận hành nền tảng
- [ ] Cấp phát: khách tự đăng ký (kèm KYC khi gói yêu cầu, bước 37) → tạo khách → chọn vị trí dữ
      liệu theo gói → migrate → dữ liệu mẫu → mời quản trị; lệnh CLI và API tương ứng
- [ ] Đối chiếu thiết kế 2: xuất toàn bộ dữ liệu một khách bằng job khi huỷ thuê bao; `suspend` thuê bao
      cho nhân sự nền tảng; hạn mức nút đếm mọi nút trừ gốc, hạn mức theo loại nút khai riêng; thông báo
      của nền tảng tới quản trị mọi khách

## 36. Tiền, sổ cái, thuế, chứng từ (D-187, D-188)

Đặc tả: [`docs/steps/36-money-ledger-tax.md`](steps/36-money-ledger-tax.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Thiết kế: trụ 4. Bắt đầu SAU bước 34 (làm song song với 35 được; mục tính tiền của 35 dùng cổng
thanh toán của bước này). Core không bao giờ tự giữ tiền.

- [ ] Số thập phân `rust_decimal` cho tính toán (ghi `STACK.md`), quy tắc làm tròn theo từng chỗ
      dùng, chuyển qua lại với `Money` không mất đơn vị
- [ ] Module `ledger`: tài khoản theo khách / nút / người bán, bút toán nợ = có, chỉ thêm, đảo
      bút toán để sửa, số dư suy từ bút toán, khoá dòng khi ghi đồng thời; test đồng thời trên cả
      hai database
- [ ] Trait `PaymentGateway`, `PayoutGateway` trong kernel (như `TextModel`); nhận webhook của đối
      tác: kiểm chữ ký, ghi nguyên văn, xử lý qua job, không trùng (mã tham chiếu D-184); đối soát
      định kỳ sổ cái với sao kê đối tác; một bộ cài đặt giả cho test
- [ ] Ba chế độ dòng tiền theo khách / sàn: tự thu, thu hộ qua đối tác, chia tại cổng — mỗi chế độ
      sinh đúng bút toán (phải trả người bán, phí, thuế khấu trừ, tạm giữ tới hết hạn đổi trả)
- [ ] Engine thuế: nhóm hàng hoá, biểu thuế theo vùng thuế với ngày hiệu lực, giá gồm / chưa gồm
      thuế, hồ sơ thuế người bán, thuế khấu trừ thay; ảnh chụp thuế suất trên từng dòng chứng từ;
      gói dữ liệu "vn" (GTGT, mức giảm tạm thời, thuế tiêu thụ đặc biệt, tỉ lệ khấu trừ hộ kinh
      doanh) cập nhật bằng CLI / API; `autoxide doctor` cảnh báo thuế suất sắp hết hạn
- [ ] Chứng từ: đánh số tuần tự không nhảy số theo phạm vi (khách / người bán / năm / ký hiệu), an
      toàn đồng thời; trait `EInvoiceProvider` cho hoá đơn điện tử; phát hành thay người bán khi
      được uỷ quyền
- [ ] Lưu trữ và khoá sổ kế toán (D-190): chứng từ, sổ cái, hoá đơn giữ ít nhất 10 năm (Luật Kế
      toán 88/2015, Điều 41; NĐ 174/2016) — xoá khách hay yêu cầu xoá dữ liệu cá nhân thì phần kế
      toán được giữ lại (lưu trữ / xuất ra), phần còn lại xoá; khoá sổ theo kỳ (kỳ đã khoá không
      nhận bút toán, sửa bằng bút toán ở kỳ mở)
- [ ] Hoá đơn khởi tạo từ máy tính tiền (D-190): `EInvoiceProvider` hỗ trợ cả loại này cho bán lẻ
      tại quầy (NĐ 123/2020 sửa bởi NĐ 70/2025: hộ kinh doanh doanh thu từ 1 tỷ đồng / năm từ
      01/6/2025; ngưỡng theo lộ trình — để dạng dữ liệu, cần đối chiếu văn bản mới nhất)
- [ ] Chuyển khoản VietQR tự khớp (D-194): sinh mã QR theo chuẩn VietQR với nội dung mang mã tham
      chiếu, nhận biến động số dư qua trait `BankFeed` (ngân hàng hay dịch vụ đọc sao kê), khớp tự
      động theo mã và số tiền, sai lệch vào hàng chờ xử lý tay; tiền vào thẳng tài khoản của người
      bán / khách (chế độ tự thu, D-187)
- [ ] Tỉ giá có ngày hiệu lực theo cặp tiền (nhập tay hay lấy từ nguồn qua trait); chứng từ ngoại tệ
      lưu ảnh chụp tỉ giá; báo cáo gộp nhiều tiền tệ (bước 38) dùng bảng này
- [ ] Mẫu in chứng từ (hoá đơn bán lẻ, phiếu giao hàng, phiếu nhập, phiếu thu): mẫu theo khách và
      i18n, xuất HTML và PDF — chọn thư viện sau khi kiểm, ghi `STACK.md`
- [ ] Xuất dữ liệu kế toán: sổ nhật ký chung và chứng từ theo kỳ ra CSV / Excel theo định dạng nhập
      của phần mềm kế toán phổ biến, để kế toán của khách không nhập lại tay
- [ ] Đối chiếu thiết kế 2 (D-216, D-218): một bộ tên tài khoản, một cơ chế tạm giữ (`ledger_holds`), một
      hạn thanh toán cho cả VietQR; `cod_remittance` (COD không về tài khoản nhà vận hành sàn); mã số thuế 12
      số (TT 86/2024 — cần đối chiếu); `GET /ledger/entries?source_module&source_id`; phí sàn tách GTGT,
      khấu trừ thay chỉ cho hộ / cá nhân (NĐ 117/2025 — cần đối chiếu)
- [ ] Đối chiếu đợt 2: `sealed_at` và số đếm niêm phong theo kỳ; `Ledger::balances` theo nhiều đối tượng (công
      nợ nhà cung cấp)

## 37. Tuân thủ pháp luật Việt Nam (D-186)

Đặc tả: [`docs/steps/37-compliance.md`](steps/37-compliance.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Thiết kế: trụ 5 và mục 4 (đăng ký Bộ Công Thương). Bắt đầu SAU bước 34. Mỗi mục ghi căn cứ pháp lý
trong quyết định của nó; quy định hay đổi để dạng dữ liệu có ngày hiệu lực.

- [ ] Định danh (KYC): hồ sơ cá nhân / tổ chức, trait `IdentityVerifier` (VNeID qua nhà cung cấp,
      eKYC), trạng thái và hạn, lưu tối thiểu (kết quả và mã tham chiếu, hạn chế lưu ảnh giấy tờ,
      sinh trắc học); cổng chặn: nút loại người bán, người livestream, tiếp thị liên kết chưa xác
      thực thì không được bán / phát (Luật TMĐT 122/2025, Điều 17, từ 01/7/2026)
- [ ] Hồ sơ pháp lý website và logo Bộ Công Thương: hồ sơ chủ sở hữu theo khách và theo người bán;
      loại website (bán hàng → thông báo, logo xanh; nền tảng trung gian / sàn → đăng ký, logo đỏ)
      theo từng tên miền; danh sách kiểm tuân thủ là dữ liệu có phiên bản; API và `autoxide doctor`
      báo thứ còn thiếu; xuất bộ hồ sơ (dữ liệu tờ khai, nháp đề án, quy chế); lưu trạng thái và
      đoạn mã logo, endpoint công khai trả về cho giao diện
- [ ] Trang bắt buộc có mẫu (module `content`): thông tin chủ sở hữu, chính sách bảo vệ dữ liệu cá
      nhân, đổi trả, vận chuyển, thanh toán, kiểm hàng, giải quyết khiếu nại, điều khoản; với sàn
      thêm quy chế hoạt động, mẫu hợp đồng với người bán, thông tin người bán trên trang sản phẩm
      (EN + VI, nội dung mẫu ghi rõ cần người có chuyên môn pháp lý duyệt)
- [ ] Báo vi phạm sở hữu trí tuệ / hàng giả: tiếp nhận → ẩn → phản hồi của người bị báo → quyết
      định, có hạn xử lý, lưu vết; kênh khiếu nại của sàn; cung cấp thông tin người bán khi cơ quan
      có thẩm quyền yêu cầu (có audit)
- [ ] Xác thực tài khoản đăng nội dung (NĐ 147/2024): cờ "đã xác thực số điện thoại / định danh";
      khách bật yêu cầu cho bình luận, đánh giá, livestream; đăng ký `PostingVerifier` của kernel (D-202)
      để `comments` (`AUTOXIDE_COMMENTS_VERIFIED_ONLY_TYPES`) dùng trạng thái thật
- [ ] Dữ liệu cá nhân (Luật 91/2025, NĐ 356/2025): sổ căn cứ xử lý / đồng ý theo mục đích; thời hạn
      lưu theo loại dữ liệu; vị trí lưu theo khách (lưu tại Việt Nam chọn được); quy trình sự cố có
      mốc 72 giờ kể từ khi phát hiện trong sổ tay vận hành
- [ ] Văn bản pháp lý có phiên bản và bằng chứng chấp nhận (D-190): điều khoản, chính sách dữ liệu
      cá nhân, quy chế sàn, hợp đồng với người bán, điều kiện giao dịch chung — mỗi bản có số phiên
      bản và ngày hiệu lực; ghi ai chấp nhận bản nào, lúc nào, qua kênh nào (Luật Giao dịch điện tử
      20/2023); bản mới thì yêu cầu chấp nhận lại khi cần
- [ ] Bảo vệ người tiêu dùng (D-190; Luật 19/2023/QH15 + NĐ 55/2024, từ 01/7/2024): nền tảng
      không chặn, không ẩn đánh giá trái quy định; minh bạch điều kiện giao dịch chung; kênh tiếp nhận
      và hạn xử lý khiếu nại của người tiêu dùng — cần đối chiếu văn bản gốc từng điều
- [ ] Gắn nhãn nội dung do AI tạo (D-190; Luật Trí tuệ nhân tạo 134/2025/QH15, từ 01/3/2026, và
      nghị định hướng dẫn — cần đối chiếu): nội dung tạo hay sửa từ gợi ý AI mang dấu nguồn gốc
      `ai_assisted`, API trả về để giao diện gắn nhãn khi công khai
- [ ] Gửi SMS (D-190): trait `SmsSender` (nhà cung cấp SMS có brandname), mã một lần qua SMS để xác
      thực số điện thoại theo NĐ 147/2024; một bộ cài đặt giả cho test
- [ ] Module nền `notifications` (D-194; thay mục "module thông báo chờ quyết"): kênh email, SMS,
      Zalo ZNS (trait), web push, trong ứng dụng; mẫu theo i18n và theo khách; tuỳ chọn nhận của người
      dùng; **tách tin giao dịch và tin quảng cáo** — tin quảng cáo chỉ gửi khi đã đồng ý trước, từ
      chối là dừng ngay, khung giờ gửi là dữ liệu (NĐ 91/2020: tin nhắn 7h–22h, chỉ một tin đăng ký
      đầu tiên — cần đối chiếu bản thay thế đang dự thảo); `newsletter` dùng chung sổ đồng ý
- [ ] Mẫu mail và thông báo động khách tự soạn (D-206): bộ dựng mẫu an toàn dùng chung trong kernel (biến,
      điều kiện, vòng lặp, giới hạn chạy, luôn thoát HTML, kiểm biến theo schema lúc lưu); mail HTML theo
      khung MJML có thương hiệu của khách, luôn kèm bản chữ thuần; nháp / xuất bản / quay lại phiên bản,
      xem trước, gửi thử tới địa chỉ của chính người gọi; `newsletter` cá nhân hoá từng người nhận
- [ ] Đối chiếu thiết kế 2 (D-217): sổ yêu cầu của chủ thể dữ liệu qua kênh khác, có hạn là dữ liệu (Luật
      91/2025 — cần đối chiếu); KYC `needs_info`; số người chấp nhận theo phiên bản văn bản; bản nháp thông
      báo sự cố; đối chiếu "bán trước khi thông báo website" bị cấm hay bị phạt rồi mới chọn chặn hay nhãn
- [ ] Kênh đẩy cho ứng dụng di động (D-220): trait `MobilePushSender` (Firebase / Apple là module riêng,
      cần khoá — dừng lại hỏi), token thiết bị mã hoá, cùng luật đồng ý và khung giờ như web push

## 40. Năng lực còn thiếu của core (agent đề xuất; số cũ 34, D-189)

Đặc tả: [`docs/steps/40-core-capabilities.md`](steps/40-core-capabilities.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

So với những gì một lõi backend hiện đại thường có sẵn, core còn thiếu sáu năng lực chung, không
gắn với ngành nào. Header bảo mật và MCP đầy đủ hơn không phụ thuộc nền đa khách: làm được ngay.
Bốn mục còn lại (OIDC, tải thẳng lên S3, SSE, rate limit) bắt đầu SAU bước 34 vì phải theo khách hàng
và tổ chức. Endpoint mới có OpenAPI và tool MCP
tương ứng; thêm thư viện thì kiểm đúng phiên bản và ghi `STACK.md`; mọi mục chạy và có test trên
cả SQLite và Postgres.

- [ ] Đăng nhập qua nhà cung cấp danh tính theo OpenID Connect (Google, Microsoft, GitHub qua
      OAuth 2.0, và một nhà cung cấp OIDC bất kỳ khai bằng cấu hình): PKCE, kiểm `state` / `nonce`,
      kiểm chữ ký `id_token` qua JWKS; liên kết với tài khoản sẵn có theo email đã xác minh, không
      tự gộp khi email chưa xác minh; MFA của Autoxide vẫn áp dụng; tắt mặc định, bật bằng cấu hình
- [ ] Tải file lớn thẳng lên nơi lưu: khi nơi lưu là S3, cấp link tải lên ký sẵn có hạn và giới hạn
      cỡ / loại, rồi một lệnh xác nhận kiểm file (cỡ, loại thật, hash) trước khi ghi bản ghi; nơi
      lưu là ổ đĩa thì đi qua máy chủ như cũ. Một trait trong `Storage`, `files` và `media` dùng
      chung; file tải lên mà không xác nhận được dọn bằng job
- [ ] Header bảo mật chung cho mọi response của API (`X-Content-Type-Options: nosniff`,
      `Referrer-Policy`, `X-Frame-Options` hay `frame-ancestors`, HSTS khi cấu hình chạy sau
      HTTPS, CSP chặt cho trang tài liệu API): một lớp trong `kernel/src/http/`, cấu hình được,
      có test; hai endpoint trả file đang tự đặt header thì dùng chung lớp này
- [ ] Đẩy event theo thời gian thực bằng Server-Sent Events — stream event do module phát, KHÔNG
      tự đẩy mọi thay đổi của bảng ("realtime theo bảng" mà `docs/research/frameworks.md` đã không
      mượn); ghi quyết định nêu khác biệt đó. Một endpoint stream các event mà người
      gọi có permission được xem (lọc theo tên event và theo quyền đọc bản ghi), nối lại bằng
      `Last-Event-ID` không mất event trong khoảng giữ, giới hạn số kết nối; chạy đúng khi nhiều
      máy chủ (đọc từ bảng event, không chỉ từ bộ nhớ một tiến trình)
- [ ] MCP đầy đủ hơn: resources (vd tài liệu của module, schema một loại bản ghi), prompts mẫu cho
      việc hay làm (viết nháp bài, kiểm duyệt bình luận) do module khai qua `Module`, kèm cài đặt
      mặc định rỗng (D-038); một trang mô tả cho agent tự khám phá API (vd `/llms.txt` trỏ tới
      OpenAPI, MCP và hướng dẫn), sinh từ code để không lệch
- [ ] Rate limit đúng khi chạy nhiều máy chủ: giới hạn theo API key (và tuỳ chọn theo địa chỉ) đếm
      chung qua database như `throttle` (D-116) hay một cách khác chạy trên cả hai database; đo
      chi phí mỗi request trước và sau; giữ đếm trong bộ nhớ làm mặc định nếu đo thấy đắt, ghi
      quyết định
- [ ] Bộ máy tìm kiếm thay được (D-220): trait `SearchBackend`, mặc định chạy trong database (bảng
      `kernel_search_docs`, đúng dấu xếp trước), backend ngoài (Meilisearch, Typesense…) cắm vào không sửa
      module; module khai loại tài liệu tìm được; lọc theo khách và nút trong backend; lệnh dựng lại chỉ mục

## Giai đoạn B — Lớp cho giao diện và phân quyền

## 51. Hợp đồng cho khung quản trị và mặt tiền (thiết kế giao diện 2, D-200)

Đặc tả: [`docs/steps/51-ui-contracts.md`](steps/51-ui-contracts.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).
Thiết kế: [`docs/research/ui-design.md`](research/ui-design.md).

Giao diện (repo riêng, D-097) dựng menu, hộp việc, tìm kiếm, trang tổng quan từ dữ liệu của core để
cùng một giao diện phục vụ mọi mô hình kinh doanh. Module khai phần của mình (mặc định rỗng, D-038);
kernel gom và lọc theo khách, gói, permission ở nút đang chọn. Kernel không có danh sách phân hệ viết cứng.
Bước 41–49 khai phần của module mình ngay trong bước đó nếu phần tương ứng của bước 51 đã xong.

- [ ] Nhúng bản ghi của module khác vào nội dung: node `embed` (`kind`, `id`) trong thân bài của
      `content`, kiểm qua sổ đối tượng (D-123) khi lưu, đọc công khai trả kèm tiêu đề, tóm tắt, đường dẫn,
      ảnh của đối tượng còn công khai; `Resource` thêm `image`. `content` không biết khái niệm sản phẩm
- [ ] Sổ điều hướng: `Module::navigation` (nhóm phân hệ, mục, permission, tính năng, nguồn số đếm,
      thứ tự); nhóm trùng phải khai giống nhau; endpoint trả menu đã lọc — ẩn khi module chưa bật hay
      thiếu permission, **khoá (vẫn hiện) khi thiếu tính năng của gói**; nhãn theo i18n; ETag; tool MCP
- [ ] Hộp việc: trait `WorkSource` (đếm, liệt kê việc chờ theo phạm vi nút), việc tính từ dữ liệu của
      module, không có bảng việc riêng; endpoint gom sắp theo mức gấp và hạn, endpoint số đếm cho menu;
      hạn thời gian mỗi nguồn, nguồn chậm không làm hỏng cả hộp; cache số đếm có khách trong khoá; khác
      hộp thư của `notifications`
- [ ] Tìm xuyên phân hệ: trait `FinderSource`; endpoint gom kết quả theo loại đối tượng, không dấu vẫn
      tìm được (`kernel::search`), chỉ chạy nguồn người gọi có quyền, mỗi nguồn áp phạm vi nút
- [ ] Số liệu tóm tắt cho trang tổng quan của phân hệ: trait `SummaryMetric` (tiền, số đếm, tỉ lệ) theo
      phạm vi nút; số nặng đọc từ bảng dẫn xuất, không quét bảng giao dịch mỗi lần mở trang
- [ ] Hoạt động gần đây (module `audit`): câu đọc được theo i18n key mà module khai trên `EventInfo`,
      chỉ hiện bản ghi người gọi có quyền đọc ở nút của bản ghi
- [ ] Khai menu, nguồn việc, nguồn tìm, số liệu cho mọi module đang có; MODULE_GUIDE (EN + VI) mục
      "khai phần của module cho giao diện"; test rò rỉ giữa hai nhánh anh em cho số đếm và kết quả tìm
- [ ] View đã lưu (D-201): `/me/views` lưu tham số lọc, sắp xếp, cột, nhóm, mật độ của một danh sách;
      phạm vi cá nhân / nút / khách; tham số kiểm theo **schema bộ lọc mà mỗi endpoint danh sách công bố**
      trong OpenAPI (máy và agent dựng được chip lọc mà không viết cứng)
- [ ] Thao tác hàng loạt chung (D-201): nhận `ids[]` hoặc `filter` kèm `expected_count` (lệch thì từ
      chối), `Idempotency-Key` bắt buộc, chạy bằng job có tiến độ (SSE của bước 40), trả kết quả từng
      bản ghi; module khai hành động hàng loạt của mình
- [ ] Tìm nâng cao (D-201): toán tử `is:`, `status:`, so sánh số, phủ định; tham số phạm vi
      `node | subtree | accessible`; endpoint `resolve` đổi một mã (số đơn, mã vận đơn, ID) thành bản
      ghi người gọi được xem
- [ ] Hộp việc theo người (D-201): mỗi việc có khoá ổn định, "vì sao bạn thấy" (permission, nút), hoãn
      tới một thời điểm, giao cho người khác — bảng cấp khách hàng có `tenant_id`; việc thiết lập theo mô
      hình (`SetupStep`, trạng thái tính từ dữ liệu, bước pháp lý chặn bán được đánh dấu)
- [ ] Giải thích quyền (D-201): `POST /me/can` kiểm nhiều cặp (permission, nút) một lần, trả lý do
      `missing_permission` / `module_not_enabled` / `feature_not_in_plan` / `quota_exceeded` và gói mở
      được — giao diện hiện "vì sao bị khoá, ai cấp được, nâng gói mở gì"
- [ ] Agent là người dùng thật (D-201): loại API key `agent`; `actor_kind` (người / agent / hệ thống) trong
      audit; lệnh rủi ro (hoàn tiền, chi trả, khoá sổ, xoá dữ liệu) do agent gọi trả `202
      approval_required` kèm yêu cầu duyệt có hạn, người có quyền duyệt thì lệnh chạy — như nhau cho REST
      và MCP; việc chờ duyệt vào hộp việc
- [ ] Dòng hoạt động và ghi chú nội bộ trên mọi bản ghi (D-201): gộp audit, ghi chú (nhắc tên bằng @,
      người được nhắc nhận thông báo), khoá bằng loại đối tượng của sổ đối tượng (D-123); một chỗ dùng
      chung thay cho mỗi module tự làm ghi chú
- [x] Cờ theo người dùng cho trợ giúp trong giao diện (D-207): `GET` / `PUT` / `DELETE /api/v1/me/flags`
      (bảng `kernel_user_flags`), chỉ cờ của chính mình, hạn mức `AUTOXIDE_USER_FLAGS_MAX`, có trong bản
      xuất dữ liệu và xoá khi xoá tài khoản; `tenant_id` thêm ở bước 33 cùng mọi bảng
- [ ] Định nghĩa số liệu tóm tắt (D-207): `SummaryMetric::definition` (i18n key, mặc định `None`),
      `GET /api/v1/kernel/summary` trả `definition` đã dịch; gate kiểm mọi số liệu đều khai và có bản dịch
      — làm cùng mục số liệu tóm tắt (P4)
- [ ] Đối chiếu thiết kế 2 (D-212, D-213): job API chung (danh sách, chi tiết có tiến độ và file, chạy lại
      có audit); xuất danh sách theo bộ lọc dùng chung (CSV qua job); yêu cầu duyệt đầy đủ (danh sách, từ
      chối, event `kernel.approval_decided`, tool MCP); bộ lọc nội dung (`review`, `scheduled`,
      `ai_generated`, `missing_locale`, `missing_alt`); bỏ hoãn việc; thử cài đặt nối ngoài; hộp việc không
      có "Đã xử lý"
- [ ] Nhập dữ liệu (D-220): không làm ở kernel — module `imports` (bước 56) thêm nguồn `autoxide` cùng cột với
      xuất chung và đích `content`, `media`, `newsletter` (người đăng ký chỉ khi có bằng chứng đồng ý)
- [ ] Danh sách lý do theo khách (P18, D-223) và `ActorKind` thống nhất cho audit, nhật ký, event (D-224)

## 54. Phân quyền chuẩn hàng đầu (chủ dự án giao ngày 13/11/2026, D-205)

Đặc tả: [`docs/steps/54-authorization.md`](steps/54-authorization.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 34. Mô hình: RBAC theo cây tổ chức có độ sâu (bước 34) + chia sẻ bản ghi một bước + deny
và rào + điều kiện theo ngữ cảnh — luật là dữ liệu có kiểu, không ngôn ngữ chính sách tự do. Đối chiếu với
Google Zanzibar, AWS IAM, GitHub, Microsoft Entra, Salesforce, Shopify, Stripe và chuẩn OpenID AuthZEN.

- [ ] Một hàm quyết định duy nhất, thu hồi tức thì (quyền không nằm trong token; cache theo phiên bản quyền
      của khách); test so khớp "lọc danh sách = kiểm từng bản ghi" sinh ngẫu nhiên trên cả hai database
- [ ] Deny thắng allow trên role; rào quyền tối đa cho khách con (kể cả admin của khách con); API key chi
      tiết: hạn bắt buộc, chọn tài nguyên, dải IP, lần dùng cuối, xoay vòng, quản trị duyệt key
- [ ] Xác thực lại cho việc nhạy cảm (passkey / MFA gần đây, RFC 9470); điều kiện theo IP và khung giờ
      trên gán role
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

## 52. Trải nghiệm quản trị nâng cao (D-201)

Đặc tả: [`docs/steps/52-admin-experience.md`](steps/52-admin-experience.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 51. Những điều nên có theo nghiên cứu các trang quản trị hàng đầu (Stripe, Shopify,
Linear, GitHub, Odoo); mỗi mục chạy và có test trên cả hai database.

- [ ] Bàn kỹ thuật cho nhà phát triển và agent: nhật ký request theo khách (API key, đường dẫn mẫu,
      status, `code`, có hạn lưu, không lưu thân request), danh sách lần giao webhook kèm gửi lại, event
      theo bản ghi, chạy lại job trong dead-letter queue
- [ ] Hoàn tác an toàn: lưu trữ / ẩn có `restore`; lệnh ghi lớn có `dry_run` trả trước những gì sẽ đổi
- [ ] AI trong quản trị (D-109): đổi câu lời thường thành bộ lọc đã kiểm theo schema (`ai/filters/parse`),
      trợ lý dùng tool MCP với đúng quyền người dùng, mọi lệnh ghi là bản nháp chờ duyệt
- [ ] Thông báo: lý do nhận, trạng thái xong / lưu, bộ lọc, cấu hình kênh theo loại event; ghim màn hình
      hay dùng (`/me/pins`)
- [ ] Nhật ký lệnh của agent (D-202): lệnh MCP ghi `mcp:<tên tool>` vào đường dẫn mẫu, không ghi tham
      số; lọc theo `actor_kind` và theo key; dòng của lệnh nhận `202 approval_required` giữ `approval_id`
- [ ] Sức khoẻ người bán và sức khoẻ tuân thủ: chỉ số so với ngưỡng (ngưỡng là dữ liệu có ngày hiệu lực),
      vi phạm, hạn phải xử lý; huy hiệu người bán tin cậy tính từ số liệu thật, công bố tiêu chí
- [ ] Đối chiếu thiết kế 2: tên endpoint bàn kỹ thuật, event theo bản ghi đọc từ audit, `kernel.devtools.manage`,
      `request_id` / `trace_id` trong nhật ký; trợ lý AI trong quản trị chạy tool MCP với quyền người gọi
- [ ] Nhật ký lệnh dùng đủ `ActorKind` của D-224

## Giai đoạn C — Vận hành

## 38. Cây khách hàng xuyên database và báo cáo gộp (D-185)

Đặc tả: [`docs/steps/38-cross-database-reporting.md`](steps/38-cross-database-reporting.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Thiết kế: trụ 2, phần xuyên database. Bắt đầu SAU bước 34 và 36 (báo cáo gộp cần số liệu tiền).

- [ ] Kho báo cáo gộp: event của các khách con đổ về khách cha (hay một database báo cáo) gần thời
      gian thực, mang `tenant_id`, `org_id`; báo cáo gộp nhiều cửa hàng, nhiều tiền tệ; người có quyền
      ở cha mới xem
- [ ] Cha thao tác trên con: vào ngữ cảnh con bằng cùng API (uỷ quyền của bước 34); job phân phối dữ
      liệu cha → con (vd danh mục, bảng giá) gọi service của con, chạy lại được, không trùng
- [ ] Tách một nhánh tổ chức thành khách riêng (database riêng) và gộp ngược lại, giữ ID và lịch sử;
      test trên cả hai database
- [ ] Đối chiếu thiết kế 2 (D-210): tách / gộp khách qua API chỉ cho nhân sự nền tảng, khách chỉ gửi yêu cầu

## 39. Vận hành và hiệu năng (chủ dự án giao ngày 13/11/2026; số cũ 33, D-189)

Đặc tả: [`docs/steps/39-operations-performance.md`](steps/39-operations-performance.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Khi đã có người dùng thật: biết sớm khi hệ thống kẹt, không mất dữ liệu, xử lý sự cố không phải
đoán, và chịu được dữ liệu cỡ thật. Bắt đầu SAU bước 38 (nền đa khách): backup, số đo, cảnh báo
theo từng khách hàng và từng database. Làm nhóm A trước: số đo tồn đọng dùng để quan sát lúc chạy tải ở nhóm B. Mọi số đo ghi kèm máy,
ngày, cỡ dữ liệu; mọi thay đổi chạy và có test trên cả hai database; không đổi tính đúng lấy tốc độ.

Nhóm A — vận hành:

- [ ] Backup tự động theo lịch: job định kỳ (qua `Schedule` của kernel) tạo bản backup như
      `autoxide backup` (`crates/app/src/commands/backup.rs`) rồi ghi vào `Storage` (ổ đĩa hay
      S3), giữ N bản gần nhất, lịch và số bản cấu hình qua `Setting` (sinh lại `.env.example`);
      bản hỏng thì ghi log lỗi và đếm bằng metric. `autoxide doctor` cảnh báo khi bản mới nhất cũ
      hơn ngưỡng. Không giữ transaction ghi trong lúc backup (dùng `Database::begin_snapshot`)
- [ ] Diễn tập restore: test đầu-cuối backup → restore vào database trống → chương trình khởi
      động, dữ liệu và file khớp, trên cả hai database (cả chép chéo SQLite ↔ Postgres nếu
      `autoxide copy` hỗ trợ); `docs/DEPLOY.md` (EN + VI) có quy trình restore từng bước
- [ ] Số đo tồn đọng (gauge, cập nhật định kỳ, tên có tiền tố như luật code): job đang chờ và tuổi
      của job chờ lâu nhất theo module, job trong dead-letter queue, event chưa giao và tuổi của
      event cũ nhất, kết nối database đang dùng / còn rảnh. Hiện chỉ có bộ đếm cộng dồn
      (`jobs_failed_total`, `events_failed_total`…), không thấy được hàng đợi kẹt
- [ ] Dashboard và cảnh báo có sẵn: file dashboard Grafana và luật cảnh báo Prometheus trong repo
      (tỉ lệ 5xx, độ trễ p95, job chờ quá lâu, dead-letter tăng, event tồn, hết kết nối database,
      backup quá cũ), mỗi cảnh báo kèm câu "làm gì"; hướng dẫn dùng trong `docs/DEPLOY.md` (EN + VI)
- [ ] Xoay khoá không gián đoạn: `AUTOXIDE_AUTH_TOKEN_SECRET` (và các khoá ký khác) nhận thêm khoá
      cũ trong một khoảng chuyển tiếp — ký bằng khoá mới, kiểm bằng cả hai — để đổi khoá (định kỳ
      hay khi bị lộ) không đăng xuất mọi người; test; quy trình trong `docs/DEPLOY.md`
- [ ] Sổ tay xử lý sự cố trong `docs/DEPLOY.md` (EN + VI): hàng đợi kẹt, job trong dead-letter
      (`autoxide jobs retry`), database đầy, lộ khoá, mất admin cuối cùng (`autoxide user …`),
      nâng cấp khi nhiều máy chủ chạy hai phiên bản code cùng lúc (migration theo kiểu mở rộng
      trước, thu hẹp sau). Mỗi tình huống: dấu hiệu, lệnh kiểm, cách sửa

Nhóm B — hiệu năng ở quy mô thật (bài đo hiện có chạy trên database gần như trống, chỉ đi qua
`notes` và `auth`):

- [ ] Bộ dữ liệu lớn để đo: một lệnh (vd trong `xtask` hay `scripts/`) sinh nhanh dữ liệu cỡ thật
      trên cả hai database — vd 100 nghìn bài nhiều ngôn ngữ có danh mục, thẻ, tác giả; 50 nghìn
      ảnh kèm bản nhỏ; 1 triệu bình luận; vài nghìn người dùng — chỉ ghi qua service hay đúng định
      dạng bảng của module, không dùng cho dữ liệu thật
- [ ] Đo tải đường đọc công khai trên bộ dữ liệu đó, mở rộng `scripts/loadtest.sh`: một bài
      (`/public/resolve`), danh sách bài và theo danh mục (`/public/posts`, `/public/terms/…`),
      `/public/sitemap`, `/public/feed`, `/public/search`, ảnh và bản nhỏ, bình luận của bài
      (`/public/threads`, `/public/comments`). Đặt ngưỡng p95 mỗi đường, ghi vào `docs/BENCHMARKS.md`
- [ ] Rà index bằng `EXPLAIN` (SQLite `EXPLAIN QUERY PLAN`, Postgres `EXPLAIN ANALYZE`) cho mọi
      truy vấn danh sách, lọc, đếm của các module trên bộ dữ liệu lớn; index thiếu thêm bằng
      migration mới (không sửa migration cũ); ghi bảng truy vấn → kế hoạch trước / sau
- [ ] Nén response (gzip, brotli): đo cỡ và thời gian trên JSON thật (danh sách, `openapi.json`
      hơn 1 MB), rồi quyết nén trong app (`tower-http`) hay để reverse proxy làm; không nén lại
      file đã nén (ảnh). Ghi quyết định; sửa `docs/DEPLOY.md` (EN + VI) theo
- [ ] Cache cho CDN: `Cache-Control`, `ETag`, `Vary` đúng cho mọi endpoint công khai; file media có
      địa chỉ mang phiên bản thì cache dài hạn (`immutable`); nội dung riêng tư hay xem trước không
      bao giờ được cache chung; hướng dẫn đặt CDN trước Autoxide trong `docs/DEPLOY.md` (EN + VI)
- [ ] Tìm theo ý nghĩa khi dữ liệu lớn: đo cách duyệt hết vector hiện tại trên bộ dữ liệu lớn so
      với ngưỡng 50 ms của D-110 (`docs/BENCHMARKS.md`, mục "Tìm theo ý nghĩa"); vượt ngưỡng thì
      chọn cách giảm (vector nhỏ hơn, chia nhiều nhân, hay index sau một trait) chạy được trên cả
      hai database, ghi quyết định; không vượt thì ghi số đo và giữ nguyên
- [ ] Quan sát theo khách (D-190): log và trace mang trường `tenant_id`; **không** dùng `tenant_id`
      làm nhãn số đo (trái luật code về nhãn); lượng dùng theo khách lấy từ số đo dùng của bước 35

## Giai đoạn D — Module nền dùng chung

Module trung lập với nghiệp vụ, không phải core: `support`, `analytics`, `automations`, `apps`; SCIM nằm trong `auth`.

## 53. Hỗ trợ khách hàng (`support`) (chủ dự án giao ngày 13/11/2026, D-204)

Đặc tả: [`docs/steps/53-support.md`](steps/53-support.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 37; song song được với 41–49. Phiếu hỗ trợ, không phải chat thời gian thực (vẫn để sau,
D-194). Khiếu nại có hạn luật định vẫn ở `compliance` (bước 37), tranh chấp ở `marketplace` (bước 46):
`support` chuyển phiếu sang đó, không làm lại.

- [ ] Phiếu và tin nhắn: tạo từ website (có tài khoản; vãng lai qua OTP email / số điện thoại; từ token
      xem đơn của bước 45), nhân viên ghi hộ; trạng thái qua một máy trạng thái; tin nhắn chỉ thêm, ghi
      chú nội bộ có nhắc tên, đính kèm qua `media`; người mua vãng lai xem bằng token phiếu, gộp phiếu vào
      tài khoản sau khi đăng ký
- [ ] Phạm vi theo nút tổ chức (bước 34): nhân viên thấy phiếu của nút mình và nhánh dưới; người bán trên
      sàn chỉ thấy phiếu gian hàng mình; nhóm nhân viên, quy tắc tự phân công là dữ liệu, chia vòng
- [ ] Hạn trả lời (SLA) theo ưu tiên, tính theo giờ làm việc và ngày nghỉ của nút, dừng khi chờ người
      mua; job đánh dấu quá hạn; nguồn việc cho hộp việc (bước 51); báo cáo thời gian phản hồi, tỉ lệ quá
      hạn, mức hài lòng
- [ ] Gắn phiếu với đơn, sản phẩm qua sổ đối tượng (D-123); câu trả lời mẫu có biến; chuyển phiếu thành
      khiếu nại người tiêu dùng của `compliance` (gọi lại không tạo hai vụ việc) hay lên sàn
- [ ] Tool MCP (cùng permission như REST); gợi ý trả lời bằng AI chỉ là bản nháp (D-109); email trả lời
      vào lại phiếu qua trait `InboundMail` (tắt mặc định); hướng dẫn `docs/SUPPORT.md` (EN + VI); test
      trên cả hai database

## 57. Phân tích truy cập và hành vi (`analytics`) (chủ dự án giao ngày 14/11/2026, D-220)

Đặc tả: [`docs/steps/57-analytics.md`](steps/57-analytics.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 37. Số liệu truy cập ngay trong Autoxide, không gửi dữ liệu người đọc cho bên thứ ba.

- [ ] Thu sự kiện từ website (endpoint công khai, gom lô, chống bot) và từ event của module; loại sự kiện do
      module khai; phiên ẩn danh không cookie với khoá xoay mỗi ngày, không lưu IP hay user agent đầy đủ
- [ ] Gộp theo giờ / ngày, báo cáo chỉ đọc bảng gộp: tổng quan, trang, nguồn, chiến dịch UTM, đối tượng xem
      nhiều, phễu do module khai; số liệu cho trang tổng quan (bước 51) và "thịnh hành" cho `catalog`
- [ ] Theo dõi theo người chỉ khi đồng ý (sổ đồng ý bước 37), rút đồng ý thì xoá phần gắn người; hạn lưu là
      cấu hình; tool MCP chỉ đọc; phạm vi theo website của nút; test trên cả hai database

## 58. Tự động hoá quy trình (`automations`) (chủ dự án giao ngày 14/11/2026, D-220)

Đặc tả: [`docs/steps/58-automations.md`](steps/58-automations.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 51 (lệnh chờ duyệt, job API). Luật "khi … thì …" như Shopify Flow; mỗi hành động là một tool
MCP đã có, chạy với quyền của người tạo luật.

- [ ] Luật: kích hoạt bằng event, lịch hay gọi tay; điều kiện là cây toán tử cố định (không biểu thức tự do);
      hành động là tool MCP có tham số điền từ event; lưu luật kiểm event, tool, quyền, tham số một lần
- [ ] Chạy bằng job ngoài transaction, nhận lại event không chạy hai lần, mỗi bước có `Idempotency-Key`; hành
      động rủi ro chờ duyệt; chống vòng lặp và giới hạn số lần chạy; nhật ký từng lần chạy, chạy lại
- [ ] Chạy thử khô, danh mục kích hoạt và hành động người gọi dùng được, mẫu luật do module khai; agent tạo
      luật thì chờ duyệt; người tạo mất quyền thì luật tự tắt; test trên cả hai database

## 66. Đồng bộ nhân viên qua SCIM 2.0 (chủ dự án giao ngày 14/11/2026, D-221)

Đặc tả: [`docs/steps/66-scim.md`](steps/66-scim.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 34 và 40 (đăng nhập OIDC).

- [ ] Endpoint SCIM 2.0 trong `auth` (Users, Groups, bộ lọc, PATCH theo RFC 7644) với token SCIM riêng mỗi khách
- [ ] Nhân viên `active=false` thì khoá và thu hồi phiên, API key ngay; nhóm ánh xạ sang role ở nút; test trên
      cả hai database

## 65. Chợ ứng dụng qua OAuth 2.0 (`apps`) (chủ dự án giao ngày 14/11/2026, D-221)

Đặc tả: [`docs/steps/65-apps.md`](steps/65-apps.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 54 và 33. Ứng dụng bên ngoài cắm vào như Shopify App Store.

- [ ] Máy chủ ủy quyền OAuth 2.0 trong `auth`: mã ủy quyền + PKCE bắt buộc, refresh token xoay vòng, phạm vi là
      tên permission, thu hồi và kiểm tra token, siêu dữ liệu máy chủ (RFC 7009, 7636, 7662, 8414)
- [ ] Đăng ký ứng dụng, nền tảng duyệt ứng dụng công khai, khách cài / đồng ý / gỡ (token mất hiệu lực ngay),
      webhook theo ứng dụng, audit `actor_kind = app`; test trên cả hai database

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

- [ ] Chuẩn bị chung cho bước 42–49 (đặc tả 41, commit đầu tiên): `Subscription::optional` (nhận
      event của module có thể chưa cài), dời bộ CSV của `forms` thành `autoxide_kernel::csv` và slug
      của `content` thành `autoxide_kernel::slug` — để ba bước song song 42–44 không cùng sửa kernel
- [ ] Sản phẩm và biến thể: tuỳ chọn (màu, cỡ…), SKU và mã vạch duy nhất trong khách, trạng thái
      (nháp, đang bán, ngừng bán, lưu trữ), ảnh qua `Media`, nội dung nhiều ngôn ngữ, cân nặng và
      kích thước (cho vận chuyển), nhóm thuế của engine thuế; slug và đường dẫn công khai
- [ ] Danh mục sản phẩm dạng cây và thuộc tính theo danh mục; **thuộc tính bắt buộc theo danh mục là
      dữ liệu** (vd xuất xứ, hạn dùng, thông tin nhãn hàng hoá theo NĐ 43/2017 và NĐ 111/2021 — cần
      đối chiếu); danh mục hàng cấm và hàng kinh doanh có điều kiện là dữ liệu, kèm giấy tờ phải có
- [ ] Bảng giá: giá theo cửa hàng / nhóm khách (B2B), kế thừa theo cây (chuỗi đặt, cửa hàng ghi
      đè — bước 34), giá có ngày hiệu lực, **lịch sử giá** (khuyến mại cần "giá ngay trước khi khuyến
      mại"); giá niêm yết bằng Đồng Việt Nam, gồm thuế khi bán cho người tiêu dùng (Luật Giá 16/2023 —
      cần đối chiếu)
- [ ] Đọc công khai cho website: danh sách, lọc theo danh mục / thuộc tính / khoảng giá, tìm kiếm
      (hợp đồng tìm kiếm sẵn có của kernel), ETag và cache như `content`; sitemap
- [ ] Tìm và lọc như các site hàng đầu (D-201): gợi ý tìm công khai gom sản phẩm, danh mục, bài viết,
      cửa hàng qua một trait; khớp đúng dấu xếp trước khớp không dấu; từ đồng nghĩa theo khách; sắp xếp
      bán chạy và đánh giá; số đếm theo từng giá trị lọc (có giới hạn, có index, cả hai database); thẻ
      sản phẩm có tóm tắt biến thể, điểm đánh giá, đơn giá theo đơn vị cơ sở; dữ liệu cho JSON-LD
      (GTIN, thương hiệu, URL từng biến thể, giá gạch là giá ngay trước khuyến mại)
- [ ] Nhập / xuất CSV bằng job (kiểm từng dòng, báo lỗi theo dòng, chống CSV injection); gợi ý mô tả
      và SEO bằng AI chỉ thành bản nháp, đánh dấu `ai_assisted` (bước 37)
- [ ] API hợp đồng cho module khác (`catalog::api`): đọc sản phẩm / biến thể / giá tại một thời
      điểm để chụp vào chứng từ; event khi giá hay trạng thái đổi; tool MCP như REST
- [ ] Loại sản phẩm: hàng vật lý, hàng số (giao file bằng link ký có hạn qua `files`, giới hạn lượt
      tải), dịch vụ (không giao hàng); combo / bộ sản phẩm trừ kho từng thành phần; sản phẩm tương tự
      theo ý nghĩa (embedding sẵn có của module `ai`)
- [ ] Hỏi đáp và ảnh song ngữ (D-202): đăng ký `catalog.product` vào sổ đối tượng, bộ tính nhãn
      `seller` (permission quản lý sản phẩm ở nút sở hữu) và event vòng đời (lưu trữ / xoá → đóng và ẩn
      hỏi đáp); `catalog_product_media` ghi đè `alt` theo ngôn ngữ như `media_alt_translations`
- [ ] Đối chiếu thiết kế 2: sản phẩm của người bán trên website của chuỗi (`seller`, `sold_by`, facet,
      `ProductCard.seller`), `warranty_months`, điểm cắm `set_listing_gate`; giá sau giảm tự động và lọc
      `on_sale`; "thường mua cùng"
- [ ] Bảng giá theo các nền tảng lớn (D-222): một thứ tự chọn bảng giá (nút gần nhất → công ty B2B → nhóm
      khách → kênh → thị trường → ưu tiên); bảng giá suy ra theo công thức ghi thành dòng giá thật; quy tắc làm
      tròn giá đẹp; sàn giá (giá tối thiểu, biên lợi nhuận trên giá vốn); đổi giá hàng loạt có xem trước, duyệt,
      hẹn giờ, giá tạm thời
- [ ] Đối chiếu đợt 2: trạng thái của lần nhập / xuất CSV; `readiness` (thiếu thuộc tính, giấy tờ) để lọc và
      đếm sản phẩm chưa mở bán được

## 42. Kho (`inventory`)

Đặc tả: [`docs/steps/42-inventory.md`](steps/42-inventory.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 41. Không bao giờ bán quá tồn khi nhiều request cùng lúc, trên cả hai database.

- [ ] Kho là nút tổ chức loại `warehouse` hay cửa hàng (bước 34); tồn theo biến thể × kho; **sổ biến
      động kho chỉ thêm** (nhập, xuất, điều chỉnh, chuyển), tồn hiện tại suy từ sổ và giữ bằng khoá
      dòng của kernel
- [ ] Giữ chỗ tồn: giữ có hạn theo `order_id` → xác nhận khi đặt hàng → trả lại khi huỷ hay hết hạn
      (job); chọn kho theo quy tắc (gần người mua, đủ hàng); test đồng thời "100 người mua 1 cái cuối"
- [ ] Chuyển kho (đang đi đường), kiểm kê (chênh lệch thành bút toán điều chỉnh có lý do), cảnh báo
      tồn thấp (event), cho phép bán khi hết hàng hay không theo sản phẩm
- [ ] Giá vốn bình quân gia quyền theo kho cho báo cáo lãi gộp; lô và hạn dùng (bán trước hết hạn
      trước), số serial — bật theo sản phẩm
- [ ] Nhận event của `orders` (xác nhận, huỷ, trả hàng nhập lại kho) — chịu được nhận lại event
- [ ] Nhập hàng (D-194): nhà cung cấp, đơn đặt hàng nhà cung cấp, nhận hàng một phần, công nợ phải
      trả nhà cung cấp vào sổ cái, trả hàng cho nhà cung cấp — chuỗi bán lẻ cần cho việc hằng ngày
- [ ] Báo khi có hàng lại theo cửa hàng (D-201): người mua đăng ký (xác nhận hai bước, đồng ý riêng cho
      mục đích này), nghe event tồn kho rồi gửi qua `notifications`; mỗi người một lần mỗi đợt có hàng
- [ ] Tình trạng hàng công khai theo cửa hàng (D-200): endpoint công khai trả mức `in_stock` / `low`
      (kèm số khi dưới ngưỡng, ngưỡng là cấu hình) / `out` cho biến thể × cửa hàng, không lộ số tồn lớn;
      `catalog` lọc "chỉ hàng có sẵn ở cửa hàng X" qua `api` của `inventory`; có ETag, cache ngắn
- [ ] Hàng đặt trước (D-204): biến thể có chế độ bán `preorder` với ngày dự kiến có hàng và giới hạn số
      suất (bước 41 khai); giữ suất không vượt giới hạn khi nhiều người đặt cùng lúc; hàng về thì chia cho
      đơn đặt trước theo thứ tự đặt
- [ ] Đối chiếu đợt 2: lý do điều chỉnh tồn là danh sách theo khách; ngưỡng sắp hết hạn của lô; trạng thái ghi
      sổ của phiếu nhập; công nợ nhà cung cấp qua `ledger`

## 43. Khuyến mại (`promotions`)

Đặc tả: [`docs/steps/43-promotions.md`](steps/43-promotions.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 41. Luật khuyến mại đổi thường xuyên: hạn mức là dữ liệu có ngày hiệu lực, không viết
cứng (Thông tư 39/2025/TT-BCT trần 50% giá ngay trước khuyến mại, có ngoại lệ chương trình tập trung
quốc gia; NĐ 239/2026 sửa NĐ 81/2018 — cần đối chiếu văn bản hiện hành).

- [ ] Mã giảm giá và giảm tự động: theo sản phẩm, danh mục, giá trị đơn, nhóm khách; mua X tặng Y;
      miễn phí vận chuyển; khung thời gian; quy tắc cộng dồn; giới hạn lượt dùng tổng và mỗi người
- [ ] Giữ chỗ lượt dùng và số lượng flash sale như tồn kho (giữ → xác nhận → trả lại), test đồng thời
- [ ] API cho `orders`: tính giảm giá cho một giỏ, trả từng dòng giảm kèm lý do (người mua thấy rõ
      điều kiện — Luật 19/2023)
- [ ] Kiểm tuân thủ: chặn chương trình vượt hạn mức đang hiệu lực; xuất dữ liệu thông báo khuyến mại
      gửi Sở Công Thương nơi tổ chức; lưu vết chương trình đã chạy
- [ ] Mã dùng được cho một giỏ (D-201): trả mã dùng được và mã chưa đủ điều kiện kèm lý do ("thêm X
      để dùng"); giao diện thay ô nhập mã mở sẵn bằng bộ chọn; số lượt còn lại chỉ hiện khi là số thật
- [ ] Giá khi áp mã cho từng sản phẩm (D-200): endpoint công khai trả các mã công khai áp được cho một
      sản phẩm / biến thể, giá sau mã, điều kiện, số lượt còn lại — để trang sản phẩm hiện "còn X với mã
      …"; giá niêm yết vẫn là giá không mã (mã phải nhập thì không hiện như giá đã giảm)
- [ ] Đối chiếu thiết kế 2: event giá sau giảm tự động cho danh sách công khai; flash sale công khai với
      suất còn là số thật; điều kiện `first_order_only`
- [ ] Chương trình, coupon, voucher (D-222): chương trình là luật; coupon là mã ai có cũng dùng được; voucher
      là quyền dùng một lần trong ví của một tài khoản (lưu, nhập mã nhận, được tặng qua `grant_voucher`); ví
      voucher của người mua; mã gắn với một người luôn là voucher
- [ ] Cộng dồn theo ba lớp sản phẩm / đơn / vận chuyển: độc quyền trong lớp hay toàn giỏ, `combines_with`,
      dừng sau chương trình, chế độ ưu tiên hay lợi nhất cho người mua (tất định); 1 voucher mỗi lớp cho mỗi bên
      trả tiền
- [ ] Ai trả tiền giảm giá (sàn / người bán / nhãn hàng / đối tác thanh toán, chia theo tỉ lệ) chia về từng
      dòng; người bán đăng ký tham gia chương trình của sàn; ngân sách tiền, lượt theo ngày, khung giờ
- [ ] Loại mới: bậc thang, combo đồng giá, mua kèm deal sốc, chọn quà, mua 2 tính 1 / món rẻ nhất; điều kiện
      theo phương thức thanh toán, hãng giao, tỉnh, phân khúc, hạng, app / web / quầy, thương hiệu, người bán,
      khách mới, loại trừ hàng đang giảm
- [ ] Chống lạm dụng theo nhiều khoá người mua (tài khoản, số điện thoại, địa chỉ — chỉ bản băm), giới hạn
      theo kỳ; quy tắc trả hàng (hoàn theo tỉ lệ, quà, voucher về ví khi huỷ); chạy thử, kiểm xung đột, lịch,
      số liệu chương trình; tính giỏ trong bộ nhớ, đo p99 < 5 ms cho 50 dòng × 500 chương trình
- [ ] Đối chiếu đợt 2 (D-225): thời hạn bắt buộc và giới hạn số ngày giảm giá là dữ liệu; thông báo theo từng
      nơi nhận; kiểm trước khi bật; nhắc khi flash sale bắt đầu (đồng ý riêng cho một lần nhắc)

## 44. Vận chuyển (`shipping`)

Đặc tả: [`docs/steps/44-shipping.md`](steps/44-shipping.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 41. Không phụ thuộc `orders`: nhận đơn qua event, báo trạng thái qua event.

- [ ] Sổ địa chỉ của người mua dùng `Address` của module nền `address` (bước 34, D-193); địa chỉ
      nước ngoài dạng chung
- [ ] Phí vận chuyển của khách tự đặt: vùng theo tỉnh, bậc cân nặng / kích thước, miễn phí từ một
      giá trị đơn, nhận tại cửa hàng (chuỗi); API báo giá cho `orders`
- [ ] Trait `ShippingCarrier` (báo giá, tạo vận đơn, huỷ, in nhãn, theo dõi) và bộ giả cho test;
      webhook trạng thái của hãng: kiểm chữ ký, không trùng, ánh xạ về trạng thái chung
- [ ] Vận đơn: một đơn nhiều kiện, giao một phần, giao thất bại, hoàn hàng; event cho `orders`
- [ ] Thu hộ khi giao (COD): tiền do hãng vận chuyển thu và chuyển trả theo hợp đồng với hãng; sổ cái
      ghi phải thu từ hãng, đối soát với bảng kê COD của hãng; lệch thì báo
- [ ] Ngày giao dự kiến là ngày cụ thể (D-201): khoảng ngày tuyệt đối kèm giờ chốt đơn trong ngày, tính
      theo múi giờ của khách; ngày làm việc, giờ chốt, ngày lễ là dữ liệu có ngày hiệu lực; nhận tại
      cửa hàng trả giờ sẵn sàng (thời gian chuẩn bị + giờ mở cửa của bước 34) và số ngày giữ hàng
- [ ] Ước tính phí giao công khai (D-200): theo tỉnh và một sản phẩm / số lượng, chưa cần giỏ, cho trang
      sản phẩm; cùng bảng phí với báo giá của `orders`, ghi rõ "ước tính", ngưỡng miễn phí
- [ ] Đối chiếu thiết kế 2 (D-216): thuế GTGT trên phí giao (Luật 48/2024 — cần đối chiếu);
      `shipping.cod_settled` mang `seller_org_id` và nơi nhận tiền
- [ ] Đối chiếu đợt 2: lịch giao và ngày nghỉ, giờ chốt giao trong ngày, trạng thái kiểm tài khoản hãng, khớp
      dòng lệch COD có lý do và audit, thời hạn giữ hàng chờ nhận

## 45. Giỏ hàng, đặt hàng, đơn hàng (`orders`)

Đặc tả: [`docs/steps/45-orders.md`](steps/45-orders.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 42, 43, 44. Gọi `catalog`, `inventory`, `promotions`, `shipping` qua `api` của chúng;
tiền qua `PaymentGateway` và sổ cái của bước 36.

- [ ] Giỏ hàng cho người mua và khách vãng lai (gộp khi đăng nhập); giá, giảm giá, thuế, phí vận
      chuyển luôn tính lại ở máy chủ, không tin số từ client
- [ ] Đặt hàng: báo giá cuối → giữ tồn và lượt khuyến mại → tạo đơn với số đơn tuần tự (đánh số chứng
      từ của bước 36) → thanh toán theo chế độ dòng tiền của khách (tự thu / thu hộ qua đối tác /
      chia tại cổng / COD); `Idempotency-Key` bắt buộc; hết hạn thanh toán thì trả chỗ
- [ ] Vòng đời đơn: chờ thanh toán → đã xác nhận → đang chuẩn bị → đang giao → đã giao → hoàn tất;
      huỷ theo quy tắc; mọi chuyển trạng thái qua service, ghi event và bút toán sổ cái tương ứng
- [ ] Đổi trả và hoàn tiền: yêu cầu trả trong thời hạn chính sách, duyệt, nhận hàng về (nhập kho),
      hoàn tiền qua cổng, bút toán đảo, hoá đơn điều chỉnh / thay thế (NĐ 123/2020 sửa bởi NĐ 70/2025
      — cần đối chiếu)
- [ ] Hoá đơn điện tử cho đơn (qua `EInvoiceProvider`), kể cả hoá đơn từ máy tính tiền cho bán tại
      quầy; mail xác nhận đơn và cập nhật trạng thái qua job, nội dung theo i18n
- [ ] Bán tại quầy (POS) qua API cho chuỗi cửa hàng: đơn tạo tại cửa hàng, tiền mặt / thẻ / QR, trừ
      kho của cửa hàng đó
- [ ] Đánh giá sau mua: chỉ người đã mua, có kiểm duyệt nhưng không chặn hay ẩn đánh giá trái quy định
      (Luật 19/2023); dùng lại `comments` nếu hợp, không thì module `reviews` — ghi quyết định
- [ ] Báo cáo bán hàng theo cửa hàng / kênh / sản phẩm / thời gian, theo quyền độ sâu; event cho kho
      báo cáo gộp (bước 38)
- [ ] Chống "bom hàng" COD (D-194): tỉ lệ nhận hàng thành công của người mua trong khách (chỉ dữ
      liệu của chính khách, có căn cứ xử lý theo Luật BVDLCN), quy tắc buộc trả trước khi rủi ro cao,
      danh sách chặn có lý do và thời hạn; không chia sẻ danh sách giữa các khách
- [ ] Mua không cần tài khoản cho trọn (D-201): báo giá giỏ có `free_shipping_remaining`; trạng thái
      thanh toán công khai bằng token (hỏi định kỳ hay SSE) cho màn QR; tra đơn vãng lai bằng số đơn +
      OTP số điện thoại; gắn các đơn vãng lai vào tài khoản mới sau khi xác thực số điện thoại; người
      nhận hộ khi nhận tại cửa hàng; cách trả hàng (lấy tận nơi / bưu cục / cửa hàng) và thời gian hoàn
      tiền theo phương thức thanh toán; đánh giá lọc theo có ảnh / số sao / sắp xếp, bộ sưu tập ảnh của
      người mua, tóm tắt AI chỉ là bản nháp nhân viên duyệt và gắn nhãn (D-109)
- [ ] Trả góp qua đối tác (trait của bước 36); bảo hành theo số serial hay theo đơn: phiếu bảo hành,
      tiếp nhận, trạng thái xử lý, hạn bảo hành
- [ ] Bổ sung theo D-202: nhãn `verified_buyer` cho hỏi đáp sản phẩm (bộ tính nhãn của kernel); trả
      hàng kèm ảnh (lý do bắt buộc ảnh là chính sách theo nút) và `POST /me/orders/{id}/returns/preview`
      (tiền hoàn, cách gửi trả và phí, thời gian hoàn, hạn — không ghi); thao tác bằng token xem đơn vãng
      lai (huỷ, đổi địa chỉ có OTP lần hai, mở hỗ trợ) qua cùng service, giới hạn tra / gửi lại OTP / hạn
      token là cấu hình, che số điện thoại và địa chỉ tới cấp phường
- [ ] Hàng đặt trước trong đơn (D-204): tách lần giao hàng sẵn và hàng đặt trước, người mua đồng ý điều
      kiện và thấy ngày giao dự kiến trước khi đặt, dời ngày thì báo và cho huỷ miễn phí, ngừng bán đặt
      trước thì huỷ và hoàn tiền tự động; nút "mở hỗ trợ" của đơn tạo phiếu ở `support` (bước 53)
- [ ] Nút "Đánh giá này hữu ích" (D-203): mỗi người một phiếu, rút lại được, đếm cùng transaction,
      `sort=helpful` phân trang bằng con trỏ; không tự bỏ phiếu cho mình, người bán không bỏ phiếu cho sản
      phẩm của mình; không có nút "không hữu ích"
- [ ] Đối chiếu thiết kế 2 (D-209, D-215, D-216, D-218): `invoice_timing = on_delivered` mặc định,
      `buyer_cancel_until = preparing`, `allowed_return_reasons`, `acceptance_hours`; `payment_actions[]` khi
      giỏ nhiều chế độ dòng tiền; nhận tại cửa hàng (mã 6 số, giao cho người nhận hộ); chứng từ của người
      mua; giỏ bỏ dở, gửi lại link thanh toán; xuất đơn qua xuất chung; hạn thanh toán đọc từ `ledger`
- [ ] Coupon và voucher trong giỏ (D-222): mã và voucher tách nhau, điền dữ liệu điều kiện cho `promotions`,
      chụp khoản giảm kèm ai trả tiền vào đơn; trả hàng hoàn theo tỉ lệ, xử lý quà; hoá đơn ghi khoản giảm và
      hàng khuyến mại (cần đối chiếu NĐ 123/2020 sửa bởi NĐ 70/2025)
- [ ] Đối chiếu đợt 2 (D-227): giữ hàng chờ nhận có hạn; đóng ca có chênh lệch ghi sổ cái; danh sách lý do theo
      khách; đếm đánh giá mới theo mốc đã xem; bản nháp tóm tắt đánh giá bằng AI chờ duyệt

## 46. Sàn (`marketplace`)

Đặc tả: [`docs/steps/46-marketplace.md`](steps/46-marketplace.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 45 và 37. Sàn là một khách; người bán là nút loại `seller` (bước 34).

- [ ] Người bán đăng ký: hồ sơ, KYC qua cổng chặn của bước 37 (chưa xác thực thì không đăng bán),
      chấp nhận quy chế sàn và hợp đồng với người bán có phiên bản (bước 37), giấy tờ cho ngành hàng
      có điều kiện; gian hàng của người bán; thông tin người bán hiện trên trang sản phẩm
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

## 47. Khách hàng và giữ chân khách (`customers`)

Đặc tả: [`docs/steps/47-customers.md`](steps/47-customers.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 45. Người mua là tài khoản theo từng khách (bước 34); module này giữ hồ sơ bán hàng.

- [ ] Hồ sơ khách: lịch sử mua (từ event của `orders`), tổng chi, nhóm và phân khúc theo quy tắc,
      nhãn, ghi chú của nhân viên; nhóm khách dùng cho bảng giá (bước 41) và khuyến mại (bước 43)
- [ ] Điểm thưởng và hạng thành viên: sổ điểm chỉ thêm, điểm có hạn dùng, quy tắc tích / đổi là dữ
      liệu; điểm chưa dùng là nghĩa vụ ghi trong sổ cái
- [ ] Thẻ quà tặng và số dư cửa hàng: chỉ dùng mua hàng của chính khách đó, không rút ra tiền,
      không chuyển ra ngoài — **tắt mặc định**, chỉ bật sau khi đối chiếu NĐ 52/2024 về ví điện tử /
      thẻ trả trước (D-194); số dư là nghĩa vụ trong sổ cái
- [ ] Danh sách yêu thích, sản phẩm vừa xem; nhắc giỏ hàng bỏ dở qua `notifications` — chỉ khi người
      mua đã đồng ý nhận tin quảng cáo (bước 37); giới thiệu bạn bè có thưởng
- [ ] Đối chiếu thiết kế 2: gộp danh sách yêu thích của trình duyệt vào tài khoản khi đăng nhập
- [ ] Tặng voucher (D-222): sinh nhật, đổi điểm lấy voucher, sau khi mua, giới thiệu bạn — qua `grant_voucher`,
      gọi lại không cấp thêm; phân khúc và hạng thành viên làm điều kiện khuyến mại
- [ ] Đối chiếu đợt 2 (D-226): che số điện thoại / email, xem đủ cần permission và ghi audit; cơ sở tính điểm
      thống nhất

## 48. Bán đa kênh (`channels`)

Đặc tả: [`docs/steps/48-channels.md`](steps/48-channels.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 45. Người bán ở Việt Nam bán cùng lúc trên website, Shopee, Lazada, TikTok Shop,
Facebook / Zalo; một kho, một danh sách sản phẩm, đơn từ mọi nơi về một chỗ.

- [ ] Trait `SalesChannel` (đăng sản phẩm, cập nhật giá và tồn, lấy đơn, cập nhật trạng thái giao)
      và bộ giả cho test; bộ nối từng sàn là module riêng, làm khi có tài khoản thử (D-191)
- [ ] Liên kết sản phẩm / biến thể với mã trên từng kênh; giá riêng theo kênh; đồng bộ tồn có phần
      để dành cho từng kênh, không bán quá khi nhiều kênh cùng bán
- [ ] Đơn từ kênh khác vào `orders` với kênh nguồn, trừ kho chung, không trùng khi kênh gửi lại;
      báo cáo theo kênh; đối soát tiền sàn trả với sổ cái
- [ ] Nguồn cấp sản phẩm cho Google Merchant Center và danh mục của Meta (D-204): tệp XML / CSV dựng bằng
      job theo cửa hàng, ngôn ngữ, tiền tệ; link công khai có token; báo lỗi từng sản phẩm (thiếu GTIN,
      ảnh); không lộ số tồn hay giá vốn
- [ ] Đối chiếu đợt 2: trạng thái đơn kênh sau khi xử lý (`resolved`, `cancelled_by_channel`); số đẩy lên kênh
      ở chế độ `reserved`

## 49. Tiếp thị liên kết và bán qua livestream (`affiliate`)

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

## 55. Giao diện cửa hàng qua API: banner, bộ sưu tập, thông tin cửa hàng (D-219)

Đặc tả: [`docs/steps/55-storefront.md`](steps/55-storefront.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 41. Website và app của khách tự vẽ giao diện (D-026: core chỉ có API); bước này cho
họ dữ liệu để vẽ trang chủ, trang danh mục, chân trang mà không phải sửa code giao diện mỗi lần đổi.

- [ ] Vị trí hiển thị và banner (`content`): vị trí theo khoá (vd `home_hero`), banner có ảnh máy tính
      và điện thoại qua `Media`, tiêu đề, nút, link (địa chỉ, bài, danh mục, sản phẩm, bộ sưu tập,
      chương trình khuyến mại), ngôn ngữ, khung giờ hiển thị, thứ tự, áp cho cửa hàng / nhánh (bước 34)
- [ ] Đọc công khai `GET /api/v1/content/public/placements/{key}`: banner đang hiệu lực lúc gọi, ảnh có
      `srcset` và WebP, cache tới mốc bắt đầu / kết thúc gần nhất; banner trỏ tới chương trình khuyến
      mại tự ẩn khi chương trình kết thúc (event tuỳ chọn của `promotions`) — quảng cáo không được hứa
      ưu đãi đã hết (Luật 19/2023, Luật Quảng cáo — cần đối chiếu)
- [ ] Bộ sưu tập sản phẩm (`catalog`): chọn tay (có thứ tự) hay theo quy tắc (danh mục, thuộc tính,
      khoảng giá, hàng mới, đang khuyến mại), sắp theo mới nhất / giá / bán chạy (số bán nhận từ event
      tuỳ chọn của `orders`); trang công khai có slug, SEO, ảnh; dùng được trong menu và banner
- [ ] Thông tin cửa hàng công khai (`organizations`): tên, logo, biểu tượng trang, liên hệ, địa chỉ
      (`Address`), giờ mở cửa theo ngày và ngày nghỉ, mạng xã hội — kế thừa theo cây (chuỗi đặt, cửa
      hàng ghi đè); chân trang lấy thêm thông tin chủ sở hữu và logo Bộ Công Thương từ bước 37
- [ ] Liên kết tới đối tượng của module đăng ký sau (sản phẩm, bộ sưu tập, chương trình) qua điểm cắm
      `ContentLinks` trong `content::api`: module đó gắn bộ dựng đường dẫn và bộ kiểm "còn hiển thị
      không"; thiếu module thì link loại đó bị từ chối khi lưu
- [ ] Tool MCP cho banner và bộ sưu tập (soạn, lên lịch, sắp xếp) như REST; AI chỉ gợi ý chữ, đánh dấu
      `ai_assisted` (bước 37); hướng dẫn cho người làm giao diện (EN + VI)

## 56. Chuyển dữ liệu từ nền tảng khác (`imports`, D-219)

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

## 59. Bán sỉ B2B (`wholesale`) (chủ dự án giao ngày 14/11/2026, D-221)

Đặc tả: [`docs/steps/59-wholesale.md`](steps/59-wholesale.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 45 và 47. Bán cho doanh nghiệp trên cùng hệ thống bán lẻ.

- [ ] Công ty mua có nhiều người mua và người duyệt; bảng giá sỉ, bậc giá theo số lượng, số lượng tối thiểu —
      không lộ giá sỉ ra công khai
- [ ] Báo giá có hạn, chấp nhận thành đơn với giá đã chốt; đặt nhanh theo SKU / CSV
- [ ] Trả chậm theo điều khoản, hạn mức tín dụng không vượt khi đặt đồng thời, công nợ trong sổ cái, tuổi nợ,
      nhắc nợ, chặn đơn trả chậm khi quá hạn; hoá đơn ghi mã số thuế công ty; test trên cả hai database

## 60. Đặt lịch dịch vụ (`bookings`) (chủ dự án giao ngày 14/11/2026, D-221)

Đặc tả: [`docs/steps/60-bookings.md`](steps/60-bookings.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 34 và 36; không cần phần bán hàng. Spa, salon, phòng khám, lớp học, sân thể thao.

- [ ] Dịch vụ (thời lượng, đệm, sức chứa, cọc), nhân viên và phòng / thiết bị, lịch làm việc lặp và ngày nghỉ
      theo múi giờ của nút; tính giờ trống công khai
- [ ] Đặt chỗ không bao giờ chồng giờ khi nhiều người đặt cùng lúc (ô giờ có khoá duy nhất, chạy trên cả hai
      database); giữ tạm khi trả cọc qua đối tác; huỷ, đổi giờ, vắng mặt theo chính sách
- [ ] Nhắc lịch qua `notifications`, khách vãng lai đặt bằng OTP, `/me/bookings`, tool MCP; test trên cả hai
      database

## 61. Chat chăm sóc khách (`conversations`) (chủ dự án giao ngày 14/11/2026, D-221)

Đặc tả: [`docs/steps/61-conversations.md`](steps/61-conversations.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 40 (SSE) và 53 (`support`).

- [ ] Cuộc trò chuyện giữa người mua và cửa hàng / người bán, gắn với sản phẩm hay đơn; tin nhắn chỉ thêm, gửi
      lại không trùng, thu hồi trong thời hạn ngắn; hộp thư chung, phân công, đã đọc
- [ ] Tin mới thời gian thực qua SSE, người không online nhận thông báo gom lô; chuyển thành phiếu hỗ trợ
- [ ] Chống lạm dụng (chặn, báo vi phạm, xác thực tài khoản khi khách bật), hạn lưu là cấu hình, gợi ý trả lời
      bằng AI chỉ là bản nháp; test trên cả hai database

## 62. Sản phẩm bán theo kỳ (`recurring`) (chủ dự án giao ngày 14/11/2026, D-221)

Đặc tả: [`docs/steps/62-recurring.md`](steps/62-recurring.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 45 và 36. Người mua trả cho cửa hàng theo kỳ — khác `plans` (khách trả cho nền tảng, bước 35).

- [ ] Gói bán theo kỳ trên sản phẩm (chu kỳ, giá, dùng thử, số kỳ tối thiểu); mỗi kỳ tự tạo đơn và thu tiền qua
      đối tác, không tạo trùng khi job chạy lại; thanh toán lỗi thì thử lại rồi tạm dừng
- [ ] Người mua tự tạm dừng, bỏ qua kỳ, đổi sản phẩm, đổi phương thức, huỷ dễ như đăng ký; báo trước khi trừ tiền
      và khi đổi giá; không lưu số thẻ; số liệu MRR và tỉ lệ rời bỏ; test trên cả hai database

## 63. Đa tiền tệ và bán xuyên biên giới (chủ dự án giao ngày 14/11/2026, D-221)

Đặc tả: [`docs/steps/63-multi-currency.md`](steps/63-multi-currency.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 36, 41, 44, 45. Mở rộng module có sẵn, không module mới.

- [ ] Tỉ giá có ngày hiệu lực dạng phân số nguyên (nhập tay hay trait nguồn tỉ giá), thị trường (quốc gia, tiền
      tệ, ngôn ngữ) theo website; giá theo thị trường, đơn chốt tỉ giá lúc đặt
- [ ] Sổ cái ghi số gốc và số quy đổi, chênh lệch tỉ giá; thuế GTGT 0% cho xuất khẩu khi đủ hồ sơ; người mua
      trong nước chỉ VND (Pháp lệnh Ngoại hối — cần đối chiếu); mã HS và xuất xứ cho giao hàng quốc tế; test trên
      cả hai database

## 64. Quảng cáo trong sàn (`ads`) (chủ dự án giao ngày 14/11/2026, D-221)

Đặc tả: [`docs/steps/64-ads.md`](steps/64-ads.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Bắt đầu SAU bước 46 và 57 (`analytics`).

- [ ] Chiến dịch của người bán (vị trí, từ khoá / danh mục, CPC / CPM, giá thầu, ngân sách, lịch); chèn vị trí
      quảng cáo vào danh sách công khai qua điểm cắm của `catalog`, luôn có nhãn "Quảng cáo"
- [ ] Đếm hiển thị và bấm qua `analytics` (chống bấm giả), trừ ngân sách không vượt khi đồng thời; phí là doanh
      thu dịch vụ của sàn thu qua quyết toán hay hoá đơn, không giữ ví trả trước; báo cáo hiệu quả cho người bán;
      test trên cả hai database

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
- [ ] Thử cài như người lạ bằng image đã đăng (cả SQLite và Postgres), sửa README /
      `docs/DEPLOY.md` (EN + VI cùng commit) theo chỗ vấp

## Sau core (chưa mở)

GraphQL · tách service — xem bảng "Chưa làm, đã cân nhắc" trong `STACK.md`.

Đã đưa lên lộ trình (D-221): bán sỉ B2B (bước 59), đặt lịch dịch vụ (60), chat chăm sóc khách (61), sản
phẩm bán theo kỳ (62), đa tiền tệ (63), quảng cáo trong sàn (64), chợ ứng dụng OAuth (65), SCIM (66).

Để sau (D-201, theo nghiên cứu): khách thử nghiệm (sandbox) cách ly, khách duyệt từng lần nhân sự nền
tảng xin vào dữ liệu, nhóm nút ngoài cây cho báo cáo, quản trị trên điện thoại có thanh đáy và tóm tắt
cuối ngày.

Để sau: bán qua ứng dụng trả phí trên chợ ứng dụng (chia doanh thu) · tiền cọc một phần cho hàng đặt trước
· đấu thầu và hợp đồng khung B2B · gọi điện / tổng đài trong chat. (Đo hành vi người xem đã thành bước 57
`analytics` không cookie, D-220.)

Chờ chủ dự án quyết (không làm khi chưa có quyết định): đăng nhập thay người dùng để hỗ trợ · định
dạng ảnh AVIF. Đã cân nhắc và không làm: API dữ liệu tự
sinh từ bảng, plugin WASM nạp lúc chạy, trang quản trị dựng sẵn (`docs/research/frameworks.md`,
D-005, D-026). Thư viện số thập phân và module sổ cái: đã quyết ở D-187, D-188.
