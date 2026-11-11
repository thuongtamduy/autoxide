# Nhật ký quyết định

Chỉ thêm ở cuối, không sửa mục cũ. Đổi một quyết định = thêm mục mới ghi rõ nó
thay mục nào. Mục ghi **chủ dự án quyết** thì agent không tự đảo.

Mẫu:

```
## D-NNN · <tiêu đề> — <ngày> — <chủ dự án quyết | agent quyết>
Quyết định: …
Lý do: …
Đã cân nhắc: …
```

---

## D-001 · Database: SQLite và Postgres — 2026-11-11 — chủ dự án quyết

Quyết định: cam kết hai database, đổi bằng `DATABASE_URL`. SQLite cho dev, test,
bản cài nhỏ một máy; Postgres cho production. MySQL không cam kết, MongoDB ngoài
phạm vi.

Lý do: mỗi database được hỗ trợ là một database phải test ở CI. MySQL kéo core
xuống mức chung thấp nhất (không partial index, không `RETURNING`, migration không
chạy trong transaction). MongoDB không ORM SQL nào phủ được.

Đã cân nhắc: cả ba database SQL (bỏ vì lý do trên).

## D-002 · UUIDv7 cho mọi ID — 2026-11-11 — chủ dự án quyết

Quyết định: mọi bảng dùng UUIDv7, sinh ở phía ứng dụng.

Lý do: chỉ Postgres 18 có sẵn hàm sinh UUIDv7; sinh trong ứng dụng là cách duy
nhất để hai database giống nhau. Thứ tự ID trùng thứ tự tạo nên dùng được cho phân
trang.

Lưu ý: UUIDv7 chứa thời điểm tạo — ID lộ ra ngoài là lộ thời gian tạo bản ghi.

## D-003 · Stack: tokio + axum + SeaORM + utoipa — 2026-11-11 — chủ dự án quyết

Quyết định: Rust với tokio, axum, SeaORM, utoipa kèm `utoipa-axum`.

Lý do: tổ hợp phổ biến nhất của Rust cho backend. SeaORM là phần nặng nhất nhưng là
thứ cho phép một bộ code chạy trên hai database. `utoipa-axum` đăng ký route và tài
liệu cùng một lệnh nên đường dẫn và method không lệch nhau.

Đã cân nhắc: sqlx thuần (nhẹ hơn, nhưng SQL phải viết riêng cho từng database).

## D-004 · GraphQL để sau, OpenAPI từ đầu — 2026-11-11 — chủ dự án quyết

Quyết định: core chỉ có REST + OpenAPI. GraphQL thêm sau.

Lý do: GraphQL độc lập với REST, thêm lúc nào chi phí cũng như nhau miễn là logic
nằm ở service layer. OpenAPI thì ngược lại: gắn sau phải rà lại mọi endpoint, và nó
là nguồn sinh SDK cho FE.

## D-005 · Một binary, ranh giới module chặt — 2026-11-11 — chủ dự án quyết

Quyết định: mỗi module là một crate, biên dịch chung thành một binary, đăng ký ở
một chỗ. Luật ranh giới: module sở hữu bảng của mình; gọi nhau qua trait công bố;
việc phụ đi qua event; không foreign key xuyên module; CI kiểm tự động.

Lý do: cái giá của microservice nằm ở lúc chạy (mất transaction xuyên service,
không join được, vận hành nhân theo số service) chứ không ở lúc viết code. Ranh
giới chặt từ đầu giữ cho việc tách sau này chỉ là thêm network layer.

Đã cân nhắc: microservice ngay từ đầu; plugin nạp lúc chạy (WASM, thư viện động).

## D-006 · Phạm vi core — 2026-11-11 — chủ dự án quyết

Quyết định: core gồm cấu hình, migration theo module, kiểu lỗi, đăng nhập, role
và permission, pagination, OpenAPI, phiên bản API, health check, log, metrics, tracing,
audit log, job queue, lưu file, i18n, MFA, rate limiting, login
throttling, idempotency key. Kèm CI trên hai database, backup và restore.

**Không** có multi-tenant: nhiều khách thì mỗi khách một bản cài riêng.

Lý do: các thứ trên đều khó gắn sau. Multi-tenant chung database chạm vào mọi bảng;
chủ dự án chọn bỏ.

## D-007 · Quy ước tiền và thời gian — 2026-11-11 — chủ dự án quyết

Quyết định: tiền là số nguyên theo đơn vị nhỏ nhất kèm mã tiền tệ; thời gian lưu
UTC, hệ thống có một múi giờ cấu hình.

Lý do: SQLite không có kiểu thập phân chính xác; số nguyên chạy giống nhau trên hai
database và không sai số.

## D-008 · Cách làm mặc định của các dịch vụ core — 2026-11-11 — chủ dự án quyết

Quyết định (chủ dự án không phản đối các đề xuất của agent):

- Job queue lưu trong database, không Redis; có retry, lịch định kỳ, dead-letter queue.
- Lưu file qua một trait, cài đặt cho ổ đĩa cục bộ và S3.
- i18n áp cho thông báo lỗi và hệ thống (vi, en), chọn theo header.
- MFA: TOTP kèm recovery code; passkey để sau.
- Audit log: bảng chỉ thêm; chuỗi băm chống sửa để sau.

## D-009 · Xử lý giới hạn single-writer của SQLite — 2026-11-11 — chủ dự án quyết

Quyết định: tách kết nối ghi (đúng một kết nối trên SQLite) và kết nối đọc; bật
WAL; transaction ghi ngắn; job queue có hai cách claim job sau một trait;
test đồng thời trên cả hai database. Thêm lệnh CLI chép dữ liệu giữa hai database.

Lý do: SQLite chỉ cho một lệnh ghi tại một thời điểm. Đo ngày 11/11/2026 trên bản
thử: đọc tăng theo số thread, ghi đứng ở khoảng 4.200–4.800 lệnh/giây.

## D-010 · Tên, chỗ đặt, deploy — 2026-11-11 — chủ dự án quyết

Quyết định: tên dự án Autoxide (auto + oxide), đặt tại
`~/Documents/source/autoxide`; deploy bằng Docker trên VPS.

## D-011 · AI agent quyết định xuyên suốt — 2026-11-11 — chủ dự án quyết

Quyết định: dự án do AI agent quyết và thực hiện, không cần con người can thiệp,
nhưng phải chuẩn chỉnh và đầy đủ. Agent chỉ dừng hỏi khi cần secret thật, cần trỏ
tên miền, sắp làm việc không hoàn tác được ở bên ngoài, hoặc muốn đảo một quyết
định của chủ dự án.

Hệ quả: luật cho agent nằm trong `CLAUDE.md`; gate tự động (`gate.sh`) thay cho
người duyệt; quyết định của agent ghi vào file này; tài liệu viết cho người mới học
Rust.

## D-012 · Rust edition 2024, toolchain ghim — 2026-11-11 — agent quyết

Quyết định: dùng edition 2024 và ghim toolchain 1.99.1 trong `rust-toolchain.toml`.

Lý do: edition 2024 là mặc định của `cargo new` hiện tại; ghim toolchain để máy dev,
CI và Docker build ra cùng một kết quả.

## D-013 · Tài liệu tiếng Việt, thuật ngữ kỹ thuật tiếng Anh — 2026-11-11 — chủ dự án quyết

Quyết định: tài liệu, comment và báo cáo viết tiếng Việt; thuật ngữ kỹ thuật giữ
nguyên tiếng Anh (danh sách ở `CLAUDE.md`, mục "Luật ngôn ngữ của tài liệu"). Tên
trong code, OpenAPI và mã lỗi vẫn là tiếng Anh.

Lý do: chủ dự án cần đọc được lộ trình, quyết định và luật để kiểm soát dự án, nên
không chuyển hẳn sang tiếng Anh. Nhưng từ dịch dễ trùng nghĩa ("cổng" vừa là gate
vừa là port, "khoá" vừa là key vừa là lock) — đó là rủi ro hiểu sai lớn nhất của
tài liệu tiếng Việt đối với agent.

Đã cân nhắc: viết toàn bộ bằng tiếng Anh (tốn ít token hơn, nhưng chủ dự án khó
đọc); dịch hết sang tiếng Việt (dễ nhập nhằng).

Ghi chú: cùng ngày, các mục D-001 đến D-012 và các tài liệu khác được rà lại để
thay từ dịch bằng thuật ngữ gốc. Chỉ đổi cách gọi, không đổi nội dung quyết định.

## D-014 · Mục tiêu của dự án — 2026-11-11 — chủ dự án quyết

Quyết định: mục tiêu là dựng Autoxide — một lõi backend bằng Rust chuẩn chỉnh, đầy
đủ, dễ nâng cấp, dễ bảo trì, dễ phát triển; chạy được trên cả SQLite và Postgres;
có REST + OpenAPI; core dựng trước, module cắm vào sau; AI agent quyết và thực hiện
xuyên suốt. Core xong khi mọi bước trong `docs/ROADMAP.md` xanh trên cả hai
database. Mọi việc phải đối chiếu với mục tiêu này ("làm gì làm, đi đến đâu cũng
được, đừng quên mục tiêu").

Mục đích sản phẩm (phục vụ nghiệp vụ gì, module đầu tiên là gì, ai dùng) chủ dự án
để mở. Agent không tự đặt ra; core giữ trung lập với nghiệp vụ.

Lý do: agent làm việc qua nhiều phiên, không nhớ phiên trước. Mục tiêu viết ở đầu
`CLAUDE.md` là thứ duy nhất chắc chắn được đọc ở mọi lượt.

## D-015 · Tương thích SQLite và Postgres là luật số một — 2026-11-11 — chủ dự án quyết

Quyết định: mọi thứ chạm tới database phải chạy được trên cả SQLite và Postgres;
tính năng chỉ chạy trên một bên là chưa xong. Agent không tự bỏ yêu cầu này cho bất
kỳ trường hợp nào — phải hỏi chủ dự án.

Lý do: chủ dự án nhắc lại yêu cầu này như một lưu ý riêng. Bổ sung cho D-001.

## D-016 · Mục đích sản phẩm — 2026-11-11 — chủ dự án quyết

Quyết định: Autoxide là nền cho một sản phẩm mới. Ai cũng dùng được. Module đầu
tiên chưa quyết.

Hệ quả: core trung lập với nghiệp vụ; người lạ phải tự cài và dùng được; agent
không tự chọn hay dựng module nghiệp vụ ngoài module mẫu của lộ trình.

Thay: phần "mục đích sản phẩm để mở" của D-014.

Còn mở: giấy phép (license) của mã nguồn — "ai cũng dùng được" chưa nói là mã nguồn
mở hay không. Repo chưa có file LICENSE; agent không tự chọn license.

## D-017 · Mã nguồn mở, công khai toàn bộ — 2026-11-11 — chủ dự án quyết

Quyết định: Autoxide là mã nguồn mở; không có phần nào riêng tư.

Hệ quả: mọi file, commit message và tài liệu viết với giả định người lạ sẽ đọc;
không ghi thông tin hạ tầng hay cá nhân của chủ dự án. Secret vẫn không bao giờ vào
git — công khai mã nguồn không có nghĩa là công khai secret.

Còn mở: chọn license cụ thể (xem đề xuất của agent trong báo cáo cùng ngày). Repo
chưa có file LICENSE; chủ dự án chọn.

Thay: phần "còn mở" của D-016.

## D-018 · Được chia agent và subagent — 2026-11-11 — chủ dự án quyết

Quyết định: dự án làm bằng Claude Code; được chia agent và subagent, số lượng tuỳ
tài nguyên máy và độ khó của việc. Quy trình ở skill `split-work`; ba luật cứng ở
`CLAUDE.md`, mục "Chia agent và subagent".

Số agent không cố định: co giãn theo máy của người đang dev và theo việc. Agent đo
CPU và RAM còn rảnh trước mỗi lần chia rồi chỉnh theo tình trạng máy; không ghi số
đo của một máy cụ thể vào repo.

Lý do của các luật: biên dịch Rust dùng nhiều nhân và vài GB RAM mỗi lượt, nên số
agent biên dịch phụ thuộc máy. Hai agent build chung một thư mục `target/` khoá
nhau, nên mỗi agent biên dịch cần worktree riêng.

## D-019 · Bảy tiêu chí chất lượng — 2026-11-11 — chủ dự án quyết

Quyết định: Autoxide phải tiêu chuẩn, đầy đủ, nhanh, tối ưu, dễ phát triển, hiện
đại, phù hợp với AI nói chung và AI agent nói riêng. Chủ dự án cũng xác nhận ba hệ
quả của D-016: core trung lập với nghiệp vụ; người lạ tự cài và dùng được; agent
không tự chọn hay dựng module nghiệp vụ ngoài module mẫu.

Nghĩa cụ thể của từng tiêu chí: bảng trong `CLAUDE.md`, mục "Mục tiêu".

## D-020 · "Phù hợp với AI agent" nghĩa là gì — 2026-11-11 — agent quyết

Quyết định: hiểu theo hai nghĩa, cả hai đều là yêu cầu. (1) AI agent phát triển
được repo: luật, lộ trình, nhật ký, skill, gate. (2) AI agent dùng được sản phẩm:
OpenAPI đầy đủ và khớp code, lỗi có `code` ổn định và `message` nói rõ cách sửa,
hành vi nhất quán giữa các endpoint, lệnh ghi có idempotency key, có API key cho
máy.

Thêm vào lộ trình: API key (bước đăng nhập), bộ đo hiệu năng có số gốc (bước công
cụ và deploy).

Chưa mở, chờ chủ dự án: MCP endpoint (cho AI agent gọi thẳng công cụ của hệ thống).

Lý do: chủ dự án nêu tiêu chí nhưng không nêu nghĩa; agent diễn giải để có thứ kiểm
được. Chủ dự án có thể sửa cách hiểu này.

## D-021 · License: MIT hoặc Apache-2.0 — 2026-11-11 — chủ dự án quyết

Quyết định: phát hành theo hai license, người dùng chọn một: MIT hoặc Apache-2.0
(`LICENSE-MIT`, `LICENSE-APACHE`; trong `Cargo.toml` là `MIT OR Apache-2.0`).

Lý do: cách phổ biến nhất của các dự án Rust; cho dùng tự do kể cả thương mại,
Apache-2.0 thêm điều khoản về bằng sáng chế.

Agent quyết kèm theo: dòng bản quyền ghi "The Autoxide Authors" vì chủ dự án chưa
nêu tên người giữ bản quyền và repo không ghi thông tin cá nhân. Chủ dự án đổi được.

Hệ quả: `cargo-deny` phải chặn thư viện có license không tương thích.

Thay: phần "còn mở" của D-017.

## D-022 · MCP endpoint nằm trong core — 2026-11-11 — chủ dự án quyết

Quyết định: core có MCP endpoint để AI agent gọi chức năng của hệ thống như công cụ.
Module khai báo tool qua hợp đồng module; mỗi tool gắn permission; tool gọi vào
service layer, không có logic riêng; lệnh ghi đi qua audit log. Dùng SDK chính thức
`rmcp`.

Lý do: tiêu chí "phù hợp với AI agent" (D-019). Làm trong core thì module nào cũng
tự có tool mà không phải tự dựng.

Thay: dòng "chưa mở: MCP endpoint" của D-020.

## D-023 · Ngôn ngữ tài liệu: tách cho người ngoài và nội bộ — 2026-11-11 — chủ dự án quyết

Quyết định: tài liệu cho người ngoài (README, hướng dẫn cài đặt và sử dụng, mô tả
trong OpenAPI và MCP) viết tiếng Anh, kèm bản tiếng Việt song song cho README và
các hướng dẫn. Tài liệu nội bộ (luật, lộ trình, nhật ký quyết định, skill, comment,
báo cáo) viết tiếng Việt với thuật ngữ kỹ thuật tiếng Anh.

Lý do: repo là mã nguồn mở cho mọi người (D-017) và người lạ phải tự cài được
(D-016), trong khi chủ dự án cần đọc được tài liệu điều hành dự án (D-013).

Thay: phạm vi của D-013 (nay chỉ áp cho tài liệu nội bộ).

## D-024 · Email, đặt lại mật khẩu, xác minh email nằm trong core — 2026-11-11 — chủ dự án quyết

Quyết định: core có gửi mail qua SMTP (sau một trait, đi qua job queue), đặt lại
mật khẩu và xác minh email.

Lý do: có đăng nhập mà không có cách đặt lại mật khẩu là chưa đạt tiêu chí "đầy đủ"
(D-019). Bổ sung cho D-006.

## D-025 · Không foreign key xuyên module, không ngoại lệ — 2026-11-11 — chủ dự án quyết

Quyết định: luật ở D-005 giữ mức chặt tuyệt đối, kể cả với bảng người dùng. Module
khác chỉ lưu ID.

Lý do: là điều kiện để tách module thành service sau này. Cái giá đã biết: database
không tự kiểm ID tham chiếu có tồn tại; service phải tự kiểm.

## D-026 · Core chỉ có API, không có giao diện web — 2026-11-11 — chủ dự án quyết

Quyết định: repo này chỉ có API (REST, MCP) và CLI. Giao diện là dự án riêng dùng
API. Trang xem tài liệu API sinh tự động thì được.

## D-027 · Thông báo mặc định bằng tiếng Anh — 2026-11-11 — chủ dự án quyết

Quyết định: client không nêu ngôn ngữ thì thông báo trả về bằng tiếng Anh. Hỗ trợ
vi và en.

Lý do: sản phẩm dành cho mọi người.

## D-028 · Tạo tài khoản: quản trị viên tạo, tự đăng ký là tuỳ chọn — 2026-11-11 — chủ dự án quyết

Quyết định: hỗ trợ cả hai; tự đăng ký bật tắt bằng cấu hình, mặc định tắt.

Lý do: mặc định an toàn; ai cần mở đăng ký thì bật.

## D-029 · SeaORM mặc định, SQL trực tiếp chỉ khi đã đo — 2026-11-11 — agent quyết

Quyết định: mọi truy vấn viết bằng SeaORM. Chỉ viết SQL trực tiếp ở chỗ đã đo được
là chậm; câu SQL đó phải chạy trên cả hai database và phải ghi lý do.

Lý do: SeaORM chậm hơn SQL trực tiếp nhưng là thứ giữ một bộ code cho hai database.
Tiêu chí "nhanh" (D-019) được hiểu là đo được và đủ nhanh, không phải nhanh nhất
bằng mọi giá.

## D-030 · Phiên bản database tối thiểu — 2026-11-11 — agent quyết

Quyết định: Postgres 16 trở lên. SQLite dùng bản đóng gói kèm binary.

Lý do: Postgres 16 còn được hỗ trợ dài và có đủ tính năng core cần; đóng gói SQLite
để người cài không phụ thuộc phiên bản của hệ điều hành. CI test trên phiên bản
Postgres tối thiểu và phiên bản mới nhất.


## D-031 · Bố cục workspace và luật ranh giới kiểm tự động — 2026-11-11 — agent quyết

Quyết định: workspace gồm `crates/kernel` (nền, không biết module nào),
`crates/testkit` (đồ test, chỉ là dev-dependency), `crates/app` (binary `autoxide`,
nơi duy nhất đăng ký module), `modules/<tên>` (mỗi module một crate
`autoxide-<tên>`) và `xtask` (công cụ). Lệnh `cargo xtask check` chạy trong gate và
kiểm:

- crate nằm đúng chỗ; mọi crate khai `MIT OR Apache-2.0` và dùng lint chung;
- chiều phụ thuộc: kernel và testkit không phụ thuộc module; testkit chỉ là
  dev-dependency; không ai phụ thuộc vào binary hay xtask;
- crate của module chỉ mở ra ngoài `pub mod api;` và `pub fn module(…)`; module
  khác chỉ dùng `autoxide_<tên>::api::…`; `api` không tham chiếu `crate::…` ngoài
  chính nó; module không xuất macro;
- trong `src/` không được `#[allow]` / `#[expect]` các lint an toàn (`unsafe_code`,
  `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, `dbg_macro`). File
  trong `tests/` được khai `#![allow(…)]` ở đầu file.

Tên gọi: "kernel" là crate nền; "core" vẫn chỉ toàn bộ phần lõi theo lộ trình (kernel
cộng các module đăng nhập, audit, job…), để hai từ không lẫn nhau.

Lý do: trình biên dịch đã chặn truy cập vào thứ không `pub`, nên chỉ cần ép bề mặt
`pub` của module về đúng hai cửa là entity (bảng), service và migration của module
tự động kín với module khác — đó là cách rẻ nhất để giữ luật "module sở hữu bảng
của mình" và "gọi nhau qua trait công bố" (D-005). Phần trình biên dịch không biết
(chiều phụ thuộc, license, nới lint) thì xtask kiểm. Luật đọc mã nguồn theo dòng,
dựa vào định dạng đã chuẩn hoá bởi `cargo fmt`; nó chặn lỗi vô ý, không phải bộ
phân tích cú pháp đầy đủ.

Đã cân nhắc: tách mỗi module thành hai crate (`-api` và phần cài đặt) — chặt hơn
nhưng gấp đôi số crate và thêm việc cho mỗi module mới; script shell với `jq` —
không viết test được; dùng `syn` để phân tích cú pháp — thêm thư viện cho việc vài
chục dòng làm được.

Chưa kiểm ở bước này (đã thêm vào lộ trình bước 1): tên bảng mang tiền tố module và
không foreign key xuyên module — cần có migration theo module mới kiểm được.

Ghi chú: `"modules/*"` chưa nằm trong `members` của `Cargo.toml` gốc vì cargo báo lỗi
khi một glob không khớp thư mục nào; thêm vào khi tạo module đầu tiên. Thông báo của
công cụ (`xtask`, `gate.sh`) viết tiếng Anh vì người đóng góp bên ngoài đọc chúng;
comment vẫn tiếng Việt (D-023).

## D-032 · Ngoại lệ cho lỗ hổng của SQLite đóng gói kèm (RUSTSEC-2026-0459) — 2026-11-11 — agent quyết

Quyết định: `deny.toml` bỏ qua đúng một cảnh báo, RUSTSEC-2026-0459 (SQLite trước
3.54.0, đóng gói trong `libsqlite3-sys` trước 0.39). Trong lúc ngoại lệ còn hiệu
lực, Autoxide **không mở file SQLite do bên không tin cậy cung cấp** — áp cho lệnh
chép dữ liệu và restore ở bước 11.

Lý do: không có phiên bản nào chọn được. `sqlx-sqlite` 0.9.0 (mới nhất, là bản SeaORM
2.0 dùng) yêu cầu `libsqlite3-sys` < 0.38, còn bản đã vá là ≥ 0.39. Lỗ hổng chỉ khai
thác được khi tiến trình mở một file database kèm file `-wal` do kẻ tấn công chuẩn
bị; file SQLite của Autoxide do chính ứng dụng tạo và giữ, ai ghi được vào đó thì đã
có toàn bộ dữ liệu. Đây là ngoại lệ cho luật "gate đỏ thì sửa code" theo đúng điều
khoản ngoại lệ của luật đó.

Tự hết hạn: gate chạy `cargo deny check -D advisory-not-detected`, nên ngay khi cây
phụ thuộc không còn bản lỗi (sau khi sqlx cho phép `libsqlite3-sys` ≥ 0.39 và ta
`cargo update`), gate đỏ cho tới khi gỡ dòng ngoại lệ. Mỗi lần nâng `sea-orm` hoặc
`sqlx` phải thử gỡ ngoại lệ này.

Đã cân nhắc: dùng SQLite của hệ điều hành thay cho bản đóng gói — trái D-030 và
phiên bản khi đó không kiểm soát được (có thể cũng chưa vá); ghi đè `libsqlite3-sys`
bằng một bản fork — phá luật "chỉ lấy thư viện từ crates.io" và không qua được ràng
buộc phiên bản của sqlx.

## D-033 · Database cho test: mỗi test một database, qua `TestDatabase::run` — 2026-11-11 — agent quyết

Quyết định: test chạm database dùng `TestDatabase::run(async |database| { … })` của
`autoxide-testkit`. Biến `TEST_DATABASE_URL` không đặt thì mỗi test một file SQLite
trong thư mục tạm; đặt `postgres://…` thì mỗi test một database Postgres riêng
(`CREATE DATABASE`), xoá bằng `DROP DATABASE … WITH (FORCE)` sau khi bài test chạy
xong. Tên database chứa UUIDv7; database sót lại do tiến trình test bị giết sẽ được
lần chạy sau dọn khi đã cũ hơn một giờ. Gate dựng Postgres bằng một container Docker
dùng xong bỏ (mật khẩu ngẫu nhiên mỗi lần, `fsync=off`), image mặc định là phiên bản
tối thiểu (D-030); CI truyền Postgres có sẵn qua `GATE_POSTGRES_URL`.

Lý do chọn `run(…)` bọc quanh bài test thay vì một biến tự dọn khi ra khỏi scope
(`Drop`): xoá database là việc `async`, `Drop` không `await` được. Đã thử và đo ngày
11/11/2026 trên Postgres 16: (1) đứng chờ trong `Drop` chặn thread của runtime tokio,
trong khi `DROP DATABASE` chờ mọi kết nối đang bắt tay dở do chính thread đó phục vụ
— test đồng thời treo tới hết `authentication_timeout` (60 giây), 2 trên 4 lượt; (2)
giao cho thread nền thì không treo nhưng tiến trình test thoát trước khi dọn xong —
sót 94 database sau 12 lượt. Với `run`: 15 trên 15 lượt xanh, không sót database nào.

Ngoại lệ cho D-029 (SQL trực tiếp): `CREATE DATABASE` và `DROP DATABASE` trong
testkit là SQL viết tay, chỉ chạy trên Postgres — lệnh quản trị không có trong query
builder và không có tương đương trên SQLite. Chỉ áp cho đồ test.

Đã cân nhắc: mỗi test một schema trong cùng một database — tạo nhanh hơn nhưng test
chạy với `search_path` khác production, dễ che lỗi; SQLite trong bộ nhớ — mỗi kết
nối thấy một database khác nhau, không dùng được với pool hay kết nối đọc / ghi tách
nhau (D-009).

## D-034 · Quy ước cấu hình — 2026-11-11 — agent quyết

Quyết định:

- Cấu hình đọc từ biến môi trường và file `.env` trong thư mục hiện tại; biến môi
  trường thật thắng `.env`. Chuỗi rỗng coi như không đặt.
- Mỗi biến khai một lần bằng `Setting` (tên, mô tả, mặc định, có phải secret) và đọc
  qua `ConfigReader` của `autoxide_kernel::config`. Không gọi `std::env::var` trực
  tiếp trong kernel, binary hay module — `cargo xtask check` chặn.
- Tên biến: tên chuẩn ngành nếu có (`DATABASE_URL`, về sau `OTEL_*`); còn lại mang
  tiền tố `AUTOXIDE_`, biến của module là `AUTOXIDE_<MODULE>_…`.
- Mọi biến có mặc định dùng được trừ secret thật; không đặt gì thì chạy bằng SQLite
  tại `data/autoxide.sqlite`.
- Kiểm lúc khởi động: gom mọi lỗi của kernel và mọi module rồi báo một lượt, mỗi lỗi
  nói rõ phải sửa gì; giá trị của secret không bao giờ xuất hiện trong thông báo.
  Tiến trình thoát với mã 2.
- `.env.example` sinh từ code (`autoxide config --env-example`); một test so file
  trong repo với bản sinh ra.
- Không có biến "chế độ" kiểu `APP_ENV=production`: mỗi hành vi có biến riêng (vd
  bật trang xem API, định dạng log). Một công tắc ngầm đổi nhiều hành vi cùng lúc
  là thứ khó đoán với người cài đặt và với AI agent.
- Thông báo của CLI và log viết tiếng Anh. Luật "thông báo đi qua i18n key" áp cho
  thông báo trả về qua API (chọn ngôn ngữ theo header), không áp cho thông báo của
  người vận hành.

Lý do: tiêu chí "người lạ tự cài được" (D-016) và "phù hợp AI agent" (D-020) — cấu
hình tự mô tả, tài liệu không lệch code, lỗi nói rõ cách sửa. Đọc qua một nguồn duy
nhất còn để test truyền cấu hình vào mà không sửa môi trường tiến trình (việc đó
`unsafe` từ Rust 2024).

Đã cân nhắc: thư viện `config` / `figment` (nhiều nguồn, nhiều định dạng) — thừa so
với nhu cầu, và không cho danh sách biến tự mô tả; tự viết bộ đọc `.env` — `dotenvy`
đã nằm sẵn trong cây phụ thuộc và xử lý đúng dấu nháy, ký tự thoát.
