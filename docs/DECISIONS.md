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

## D-035 · Chi tiết kết nối database tách đọc / ghi — 2026-11-11 — agent quyết

Quyết định (cài đặt cho D-009, trong `autoxide_kernel::database::Database`):

- Kết nối đọc bị cấm ghi trên **cả hai** database: SQLite mở read-only, Postgres đặt
  `default_transaction_read_only=on`. Lỡ ghi qua phía đọc thì hỏng giống nhau ở cả
  hai bên, ngay ở test.
- Transaction ghi mở qua `Database::begin_write()`; trên SQLite nó dùng
  `BEGIN IMMEDIATE`.
- SQLite: phía ghi đúng một kết nối, bật WAL, `synchronous=NORMAL`, bật foreign key
  trên mọi kết nối, tự tạo thư mục và file. SQLite trong bộ nhớ bị từ chối ở bước
  kiểm cấu hình.
- `DATABASE_READ_URL` (read replica) chỉ dành cho Postgres.
- Một biến `AUTOXIDE_DATABASE_WAIT_SECONDS` cho cả thời gian chờ kết nối rảnh và
  thời gian chờ lock của tiến trình ghi khác.

Lý do: luật số một đòi hành vi giống nhau trên hai database; cấm ghi ở phía đọc
biến một quy ước ("đọc và ghi tách kết nối") thành thứ database tự kiểm. Về
`BEGIN IMMEDIATE`: đo ngày 11/11/2026 bằng test `hai_tien_trinh_cung_ghi_mot_database`
(hai `Database` cùng ghi một file, mỗi transaction đọc rồi ghi) — với kiểu mặc định
của SQLite (`DEFERRED`) test hỏng 3 trên 3 lượt với lỗi "database is locked"; với
`IMMEDIATE` thì xanh. `synchronous=NORMAL` là mức khuyến nghị cho WAL: file không
hỏng khi mất điện, chỉ có thể mất vài transaction cuối; ai cần bền tuyệt đối thì
dùng Postgres. Không bật foreign key thì SQLite lặng lẽ nhận dữ liệu sai mà Postgres
từ chối (test `foreign_key_duoc_kiem` bắt được khi tắt thử).

Đã cân nhắc: để phía đọc ghi được và chỉ dựa vào quy ước — rẻ hơn nhưng lỗi chỉ lộ
khi chạy SQLite dưới tải hoặc khi bật read replica; `synchronous=FULL` — bền hơn
nhưng mỗi commit thêm một lần fsync, trái mục tiêu của WAL.

## D-036 · Migration theo module — 2026-11-11 — agent quyết

Quyết định:

- Mỗi module khai migration của mình bằng `ModuleMigrations` và có bảng lịch sử
  riêng `<module>_migrations`. `Migrator` của kernel chạy `up` cho mọi module theo
  thứ tự đăng ký; `down` gỡ từng module một; `status` chỉ đọc, không tạo bảng nào.
- Mọi bảng của module `x` mang tiền tố `x_`. Tên module không được là tiền tố kèm
  `_` của tên module khác (`auth` và `auth_keys` không cùng tồn tại).
- Mọi migration chạy trong một transaction trên cả hai database; migration phải có
  `down`.
- Migration không tự chạy khi khởi động máy chủ: người vận hành chạy `autoxide
  migrate up`; `serve` từ chối chạy khi còn migration chưa áp dụng.

Lý do: một bảng lịch sử chung cho mọi module (cách mặc định của sea-orm-migration)
buộc các module vào một thứ tự chung và làm việc gỡ hay tách một module thành
service riêng phải đụng tới bảng dùng chung — trái D-005. Thư viện mặc định không
dùng transaction trên SQLite; ép dùng để migration hỏng giữa chừng không để lại gì
ở cả hai bên (test `migration_hong_giua_chung_khong_de_lai_gi` đỏ trên SQLite khi
bỏ phần ép này). Không tự migrate lúc khởi động vì nhiều bản máy chủ khởi động cùng
lúc sẽ tranh nhau chạy migration; một lệnh riêng chạy một lần mỗi lần deploy là
cách làm chuẩn và để người vận hành thấy rõ điều gì sắp đổi.

Đã cân nhắc: một danh sách migration chung có tên kèm tiền tố module — đơn giản hơn
nhưng `down` theo module phải nhảy cóc trong lịch sử chung; tự chạy migration lúc
`serve` — tiện cho người mới nhưng nguy hiểm khi chạy nhiều bản.

## D-037 · Kiểm luật sở hữu bảng bằng cách hỏi chính database — 2026-11-11 — agent quyết

Quyết định: `autoxide_testkit::schema_ownership_violations` đọc danh mục của database
đã migrate và báo (1) bảng không mang tiền tố `<module>_` của một module đã đăng ký,
(2) foreign key trỏ sang bảng của module khác. Một test trong binary
(`crates/app/src/modules.rs`) chạy mọi migration của chương trình trên database
trống, gọi bộ kiểm này, rồi gỡ hết và áp dụng lại — nên gate chạy nó trên cả SQLite
và Postgres mỗi lần. Module được đăng ký ở đúng một chỗ: `crates/app/src/modules.rs`.

Lý do: hai luật này (D-005, D-025) nằm trong cấu trúc database, trình biên dịch và
`cargo xtask check` không thấy được. Hỏi database sau khi migrate bắt được cả
foreign key thêm bằng `ALTER TABLE` ở một migration về sau.

Ngoại lệ cho D-029 (SQL trực tiếp): bốn câu lệnh đọc danh mục (`sqlite_master`,
`pragma_foreign_key_list`, `information_schema.tables`, `pg_constraint`) là SQL viết
tay, mỗi database một nhánh, cả hai nhánh đều có test. Chỉ áp cho đồ test (mở rộng
ngoại lệ ở D-033).

Đã cân nhắc: bắt module tạo bảng qua một lớp bọc biết tên module — chặn được từ lúc
viết nhưng giới hạn cách viết migration và vẫn lọt nếu ai đó gọi thẳng thư viện.

## D-038 · Hình dạng của hợp đồng module — 2026-11-11 — agent quyết

Quyết định: module cài trait `Module` (`autoxide_kernel::module`) và crate của nó mở
đúng `pub fn module() -> impl Module` cùng `pub mod api`. Binary đăng ký mọi module ở
một chỗ (`crates/app/src/modules.rs`); kernel giữ chúng trong `Modules` và chỉ làm
việc qua trait. Hiện hợp đồng gồm `name`, `migrations`, `configure`. Mỗi phần sau
của core (route, permission, event, job, tool MCP) thêm một hàm vào trait, **luôn
kèm cài đặt mặc định**. `configure(&mut self, reader)` cho module đọc cấu hình có
kiểu của riêng nó và giữ trong chính nó.

Tên module: chữ thường, số, `_`; không trùng; không là tiền tố kèm `_` của module
khác; không là `sqlite` hay `pg` (tiền tố bảng hệ thống của hai database). Tên module
cũng là tiền tố bảng (`<tên>_`) và tiền tố biến cấu hình (`AUTOXIDE_<TÊN>_`).

Lý do: tiêu chí "thêm module không phải sửa core" (D-019). Trait với cài đặt mặc
định cho phép hợp đồng lớn dần mà module đã viết không phải sửa. Giữ cấu hình trong
module (thay vì trả về một giá trị không kiểu cho core giữ hộ) để trình biên dịch
kiểm kiểu, không phải ép kiểu lúc chạy.

Đã cân nhắc: một struct gồm các con trỏ hàm dựng bằng builder — cấu hình có kiểu
của module phải đi qua `Any` và ép kiểu lúc chạy; trait có kiểu liên kết cho cấu
hình — không để chung nhiều module trong một danh sách được; tự phát hiện module
lúc biên dịch (`inventory`, `linkme`) — bớt một dòng đăng ký nhưng thêm thư viện
dùng `unsafe` và làm thứ tự đăng ký khó thấy.

Chưa có ở bước này: cách một module lấy trait của module khác lúc khởi động — thêm
cùng lúc với việc module dựng service và route (mục HTTP).

## D-039 · Kiểu lỗi, hình dạng response, pagination — 2026-11-11 — agent quyết

Quyết định:

- **Một kiểu lỗi** `autoxide_kernel::error::Error` cho service, REST và MCP: loại lỗi
  (quyết định mã HTTP), `code` ổn định cho máy, thông báo, lỗi theo từng trường, và
  nguyên nhân gốc chỉ dành cho log. Lỗi database và lỗi của thư viện validator tự đổi
  sang kiểu này. Lỗi nội bộ luôn ra ngoài với mã `internal_error` và một câu chung.
- **`code` là i18n key.** Chỗ tạo lỗi viết mã cùng câu tiếng Anh mặc định (có chỗ
  điền `{tên}`); bước 6 thêm bản dịch tra theo `code` mà không phải sửa chỗ tạo lỗi.
- **Lỗi trả về theo RFC 9457** (Problem Details, `application/problem+json`): `type`
  luôn là `about:blank`, `title`, `status`, `detail`, cộng hai trường mở rộng `code`
  và `errors` (danh sách `field` / `code` / `detail`). Request đọc được nhưng giá trị
  sai là 422; request không đọc được là 400.
- **Thành công:** `{"data": …}` cho một đối tượng; `{"data": […], "pagination":
  {"limit", "next_cursor"}}` cho danh sách.
- **Pagination theo con trỏ trên ID** (UUIDv7): tham số `limit` (1–200, mặc định
  50), `cursor`, `order` (`desc` mặc định, `asc`); luôn sắp theo thời điểm tạo. Hàm
  `fetch_page` lấy dư một dòng để biết còn trang sau, không chạy câu lệnh đếm.
- Doc comment trên kiểu và handler có mặt trong OpenAPI viết tiếng Anh (utoipa chép
  nguyên vào tài liệu API).

Lý do: RFC 9457 là chuẩn của IETF cho lỗi của HTTP API (tiêu chí "tiêu chuẩn"), và
`code` + `errors` cho AI agent đủ thông tin để tự sửa request (D-020). Gộp `code`
với i18n key để mỗi lỗi chỉ có một định danh phải giữ ổn định. Pagination theo con
trỏ: trang sâu nhanh như trang đầu vì đi bằng index của khoá chính, và dữ liệu mới
chen vào không làm trùng hay sót dòng — test
`du_lieu_them_giua_chung_khong_lam_trung_hay_sot` kiểm điều này trên cả hai database.

Giới hạn đã chấp nhận: danh sách chỉ sắp theo thời điểm tạo. Sắp theo cột khác cần
con trỏ ghép (giá trị cột + ID); thêm khi một module thật sự cần, qua một quyết định
mới.

Đã cân nhắc: envelope lỗi tự chế `{"error": {…}}` — đơn giản nhưng không chuẩn nào
biết; `type` là URL tới trang mô tả từng lỗi — cần một địa chỉ công khai mà dự án
chưa có; pagination `page` / `per_page` — quen thuộc nhưng chậm dần theo số trang và
trùng dòng khi dữ liệu thay đổi; trả tổng số dòng — tốn một câu lệnh đếm mỗi request.

## D-040 · Lớp HTTP và giai đoạn khởi động của module — 2026-11-11 — agent quyết

Quyết định:

- **Hợp đồng module thêm hai hàm** (đều có cài đặt mặc định): `start(context)` —
  dựng service với database, công bố trait cho module khác (`provide::<dyn Trait>`)
  và lấy trait của module khác (`require::<dyn Trait>`); `routes()` — route REST kèm
  tài liệu OpenAPI. Module khởi động theo thứ tự đăng ký; bên cần trait đăng ký sau
  bên cung cấp; mỗi trait đúng một bên cung cấp.
- **Route của module `x` nằm dưới `/api/v1/x`**; module chỉ khai phần đuôi. Thay đổi
  phá vỡ thì mở `/api/v2`.
- **Mọi response lỗi là Problem Details**, kể cả lỗi do thư viện sinh ra (404, 405,
  413, query sai kiểu…): một lớp bọc ngoài đổi chúng về cùng dạng, giữ câu mô tả của
  lỗi 4xx và không bao giờ chép thân của lỗi 5xx ra ngoài.
- **Request id** ở header `x-request-id`: nhận ID của client nếu nó ngắn (≤ 128 ký
  tự) và chỉ gồm chữ, số, `-_.:`; còn lại sinh UUIDv7. Luôn trả lại trong response.
- **Timeout** trả 503 mã `request_timeout`; **body quá lớn** trả 413; **handler
  panic** thành 500 mã `internal_error`, nội dung panic chỉ vào log.
- **CORS** mặc định không cho origin nào; cấu hình bằng `AUTOXIDE_HTTP_CORS_ORIGINS`.
- **Graceful shutdown:** nhận SIGTERM hoặc Ctrl-C thì ngừng nhận kết nối mới, chờ
  request đang chạy tối đa `AUTOXIDE_HTTP_SHUTDOWN_SECONDS` rồi dừng hẳn.
- Handler nhận thân JSON qua `ValidJson<T>`: đọc và kiểm trong một bước, mỗi kiểu lỗi
  đầu vào một mã riêng (`invalid_json`, `invalid_body`, `validation_failed`,
  `unsupported_media_type`).

Lý do: gắn route theo tên module làm đường dẫn không thể đụng nhau giữa hai module và
nhìn URL là biết module nào sở hữu. Chuẩn hoá lỗi ở một lớp ngoài cùng là cách duy
nhất bảo đảm "cùng envelope" mà không phụ thuộc vào việc mọi handler đều viết đúng.
Timeout trả 503 thay vì 408 vì 408 nghĩa là client gửi request quá chậm; ở đây máy
chủ mới là bên không kịp. Sổ trait tra theo kiểu (`TypeId`) giữ cho module khác chỉ
phụ thuộc vào `api` của bên cung cấp, không biết kiểu service cụ thể.

Đã cân nhắc: để module tự chọn đường dẫn đầy đủ — linh hoạt hơn nhưng hai module có
thể khai trùng đường dẫn, và axum panic khi ghép; tự động sắp thứ tự khởi động theo
phụ thuộc — thêm một bộ giải phụ thuộc trong khi thứ tự đăng ký đã đủ và lỗi sai thứ
tự nói rõ cách sửa.

## D-041 · OpenAPI: sinh từ code, nằm trong repo, kiểm trong gate — 2026-11-11 — agent quyết

Quyết định:

- Tài liệu OpenAPI sinh từ chính khai báo route. Core tự thêm phần giống nhau ở mọi
  endpoint: tag = tên module, `operationId` mang tiền tố `<module>_`, response
  `default` là Problem Details, và phần mô tả chung nêu các quy ước của API.
- Tài liệu được phục vụ ở `/api/v1/openapi.json` (luôn có) và nằm trong repo ở
  `docs/openapi.json`, sinh bằng `autoxide openapi`. Một test so file trong repo với
  bản sinh từ code — gate chạy nó trên cả hai database.
- `http::lint` kiểm mỗi endpoint có summary, tag, `operationId` không trùng, ít nhất
  một response thành công và mô tả cho từng response. `autoxide openapi` và test
  trên coi tài liệu thiếu là lỗi.
- `cargo xtask check` thêm luật `module-routes`: module không được thêm route bằng
  `.route(…)` trần (route không có tài liệu).
- Trang xem tài liệu (`/api/docs`, Scalar) tắt mặc định, bật bằng
  `AUTOXIDE_HTTP_API_DOCS=true`; trang này tải script từ CDN công cộng.

Lý do: đưa `docs/openapi.json` vào repo làm mọi thay đổi của API hiện ra khi xem lại
thay đổi, và cho AI agent đọc hợp đồng mà không cần chạy máy chủ (D-020). Để
`autoxide openapi` in được tài liệu, module phải được khởi động (route gắn với
service của module), nên lệnh này kết nối database nhưng không cần migration.

Đã cân nhắc: chỉ phục vụ tài liệu lúc chạy, không đưa vào repo — thay đổi API không
ai thấy khi xem lại; bật trang xem tài liệu mặc định — tiện, nhưng mặc định nên là
không tải script của bên thứ ba.

## D-042 · Health check — 2026-11-11 — agent quyết

Quyết định: `/health/live` trả 200 miễn là tiến trình trả lời được, không đụng
database. `/health/ready` trả 200 khi database trả lời trong 2 giây, ngược lại 503
dạng Problem Details với mã `not_ready` (chi tiết lỗi của database chỉ vào log). Hai
endpoint nằm ngoài `/api/v1`, có trong tài liệu OpenAPI với tag `health`, và dùng
cùng envelope `{"data": {"status": "ok"}}`.

Lý do: tách hai câu hỏi vì hệ thống vận hành xử lý khác nhau — `live` hỏng thì khởi
động lại tiến trình, `ready` hỏng thì chỉ ngừng gửi request. Nếu `live` cũng kiểm
database thì database tạm mất sẽ khiến mọi bản máy chủ bị khởi động lại liên tục.

Chưa làm: báo "chưa sẵn sàng" trong một khoảng trước khi tắt (để bộ cân bằng tải rút
request trước) — cần khi chạy nhiều bản sau bộ cân bằng tải; xem lại ở bước deploy.

## D-043 · Log, metrics, tracing — 2026-11-11 — agent quyết

Quyết định (chốt các thư viện của bảng "Observability" trong `STACK.md`):

- **Log:** `tracing` + `tracing-subscriber`. Ra stderr (stdout dành cho kết quả của
  lệnh), dạng chữ hoặc JSON theo `AUTOXIDE_LOG_FORMAT`, lọc theo `AUTOXIDE_LOG_LEVEL`.
  Mỗi request một dòng khi xong, nằm trong span `request` mang request id nên mọi log
  phát ra trong lúc xử lý cũng mang id đó. Chỉ ghi đường dẫn, không ghi query string
  hay thân request. Lỗi 5xx ghi cả nguyên nhân gốc. Health check ghi ở mức debug.
- **Metrics:** `metrics` + `metrics-exporter-prometheus`, phục vụ ở `/metrics` khi
  `AUTOXIDE_HTTP_METRICS=true` (mặc định tắt vì endpoint không có xác thực). Lớp HTTP
  ghi `http_requests_total` và `http_request_duration_seconds` với nhãn `route` là
  **mẫu** route; request không khớp route nào dùng chung nhãn `unmatched`.
- **Tracing:** `opentelemetry` 0.33 + `tracing-opentelemetry` 0.34, xuất OTLP qua HTTP
  khi đặt `OTEL_EXPORTER_OTLP_ENDPOINT`; request mang `traceparent` (W3C Trace
  Context) thì nối vào trace của bên gọi. Chỉ nhận địa chỉ `http://`.

Lý do của giới hạn `http://`: bật TLS cho bộ gửi OTLP kéo vào thư viện mật mã
`aws-lc-rs`, trong khi kết nối Postgres (sqlx) dùng `ring`. Khi cả hai cùng có mặt,
`rustls` không tự chọn được thư viện mặc định và panic ở lần dùng TLS đầu tiên — lỗi
chỉ lộ ở production, nơi mới có TLS. `aws-lc-rs` còn làm bản build nặng hơn. Cách
chuẩn là chạy một OpenTelemetry collector cạnh máy chủ và để nó chuyển tiếp qua TLS.
Đã kiểm ngày 11/11/2026: không bật TLS cho bộ gửi thì cây phụ thuộc chỉ có `ring`.

Ràng buộc kèm theo: bộ xuất trace phải được bật và tắt ngoài runtime tokio (nó dùng
HTTP client kiểu đồng bộ trên thread riêng); mọi lệnh của CLI đi qua
`run_with_observability` để giữ đúng thứ tự đó.

Đã cân nhắc: OTLP qua gRPC (`tonic`) — thêm `tonic` và `prost` đầy đủ cho một việc
HTTP làm được; bật `/metrics` mặc định — tiện, nhưng lộ số liệu vận hành khi port mở
ra internet; đọc `RUST_LOG` — trái quy ước mọi cấu hình đi qua `Setting` (D-034).

## D-044 · Người dùng nằm trong module `auth`; mật khẩu hash bằng Argon2id — 2026-11-11 — agent quyết

Quyết định:

- **Phần tài khoản của core là một module**: crate `autoxide-auth` ở `modules/auth`,
  tên module `auth`, bảng `auth_…`. Kernel không có bảng nào. Các mục còn lại của bước 2
  và bước 3 (đăng nhập, refresh token, role, permission, API key, MFA) thêm vào chính
  module này. Module được đăng ký đầu tiên trong `crates/app/src/modules.rs`.
- **Bảng `auth_user`:** `id` (UUIDv7), `email` (≤ 254 ký tự, index unique),
  `name` (1–200 ký tự), `password_hash` (chuỗi PHC, có thể `NULL` = chưa đặt mật
  khẩu), `disabled_at`, `created_at`, `updated_at`. Email chuẩn hoá trước khi lưu: bỏ
  khoảng trắng hai đầu, chữ thường cả địa chỉ — index thường chặn được trùng không
  phân biệt hoa thường trên cả hai database.
- **Argon2id** (thư viện `argon2` 0.6), tham số OWASP: 19 MiB, 2 lượt, 1 luồng. Hash
  chạy trong `spawn_blocking`, số lần hash cùng lúc giới hạn bằng số nhân CPU. Hash
  lưu tham số cũ thì hash lại ở lần đăng nhập đúng kế tiếp.
- **Luật mật khẩu:** chỉ độ dài, 12–128 ký tự. Không luật ghép ký tự (NIST SP
  800-63B).
- **Không lộ email nào đã đăng ký:** email lạ và mật khẩu sai trả cùng lỗi
  `auth.invalid_credentials`, sau cùng một lần kiểm Argon2 (email lạ kiểm với một hash
  giả cùng tham số). Tài khoản bị khoá chỉ báo `auth.user_disabled` sau khi mật khẩu
  đúng.
- **Hợp đồng cho module khác:** `autoxide_auth::api::Users` (`find(id)`), công bố qua
  `context.provide`. Trait có hàm `async` gọi qua `dyn` nên dùng thư viện
  `async-trait` (đã có sẵn trong cây phụ thuộc qua SeaORM).

Lý do: luật sở hữu bảng (mọi bảng thuộc một module, gate kiểm) và migration theo
module đã có sẵn cho module; đặt người dùng trong kernel sẽ phải mở ngoại lệ cho cả
hai. Gộp đăng nhập, role và MFA vào một module vì chúng đọc ghi chung dữ liệu tài
khoản trong cùng transaction; tách ra thì phải đi vòng qua trait cho từng câu hỏi
nhỏ. Tham số Argon2id lấy theo OWASP Password Storage Cheat Sheet thay vì tự chọn.

Đã cân nhắc: bảng người dùng trong kernel — phá luật sở hữu bảng; tách `users`,
`sessions`, `roles` thành ba module — nhiều trait qua lại mà không được gì vì không
module nào dùng một phần mà bỏ phần kia; bcrypt — cắt mật khẩu ở 72 byte và không
tốn bộ nhớ nên dễ bẻ bằng GPU hơn; so sánh email bằng `lower(email)` trong câu truy
vấn — cần index trên biểu thức, cú pháp khác nhau giữa hai database.

## D-045 · Đăng nhập: access token JWT HS256 tự viết, refresh token xoay vòng — 2026-11-11 — agent quyết

Quyết định:

- **Access token** là JWT (RFC 7519) ký HMAC-SHA256, mang `iss`, `sub` (ID người
  dùng), `iat`, `exp`; sống 15 phút (`AUTOXIDE_AUTH_ACCESS_TOKEN_SECONDS`). Máy chủ kiểm
  chữ ký, không hỏi database về token — nhưng vẫn đọc người dùng để chặn tài khoản
  đã khoá. Chỉ nhận header `alg = HS256`.
- **Tự viết JWT** (khoảng 100 dòng, `modules/auth/src/token.rs`) bằng `hmac` 0.13,
  `sha2` 0.11, `base64` 0.22, `getrandom` 0.4 — cả bốn đã có sẵn trong cây phụ thuộc.
  **Không dùng `jsonwebtoken` 11** như `STACK.md` ghi trước đây: crate này buộc bật
  một backend mật mã đầy đủ; backend `rust_crypto` kéo `rsa` 0.9 (RUSTSEC-2023-0071,
  chưa có bản vá) và `crypto-bigint` 0.5 (RUSTSEC-2026-0387) — `cargo deny` chặn;
  backend `aws_lc_rs` gây xung đột `rustls` đã ghi ở D-043. Autoxide chỉ cần đúng
  HS256, nên mở ngoại lệ trong `deny.toml` cho thư viện không dùng tới là sai hướng.
- **Khoá ký** `AUTOXIDE_AUTH_TOKEN_SECRET`, ≥ 32 ký tự. Không đặt thì sinh khoá ngẫu
  nhiên khi khởi động và ghi cảnh báo — người lạ chạy thử được ngay; production phải
  đặt (README hướng dẫn).
- **Refresh token** là 32 byte ngẫu nhiên (base64url), sống 30 ngày
  (`AUTOXIDE_AUTH_REFRESH_TOKEN_DAYS`), database chỉ lưu SHA-256 của nó (bảng
  `auth_refresh_token`). Mỗi lần đổi cấp token mới cùng "họ" (family) với thời hạn
  mới tính từ lúc đổi.
- **Phát hiện dùng lại:** token đã đổi bị gửi lại → thu hồi cả họ, trả
  `auth.refresh_token_reused`. Đánh dấu "đã dùng" bằng `UPDATE … WHERE used_at IS
  NULL` trong transaction; hai request đổi cùng một token đồng thời thì bên thua cũng
  bị coi là dùng lại (test đồng thời trên cả hai database).
- Đổi mật khẩu thu hồi mọi refresh token của người dùng trong cùng transaction. Đăng
  xuất thu hồi họ của token gửi lên; gọi lại vẫn trả 204.
- Endpoint: `POST /api/v1/auth/login`, `/refresh`, `/logout`, `GET /api/v1/auth/me`.
  Kernel thêm security scheme `bearer` vào OpenAPI và header `WWW-Authenticate:
  Bearer` cho mọi lỗi 401 (RFC 6750, RFC 9110).

Lý do: access token sống ngắn không thu hồi được nhưng kiểm rẻ; refresh token lưu
database thu hồi được — cặp này là cách làm chuẩn (OAuth 2.0 Security BCP, RFC 9700
mục 4.14). Thu hồi cả họ khi dùng lại là khuyến nghị của chính tài liệu đó.

Giới hạn đã chấp nhận: access token còn hiệu lực tới 15 phút sau khi đăng xuất, đổi
mật khẩu hay bị khoá — trừ `/me` và mọi chỗ đọc người dùng, vốn kiểm tài khoản khoá.
Client gửi đúng một token hai lần do retry mạng sẽ bị đăng xuất; tài liệu API dặn
"mỗi refresh token dùng một lần".

Đã cân nhắc: `jsonwebtoken` kèm ngoại lệ `cargo deny` — giữ lỗ hổng trong cây chỉ để
dùng một hàm; session lưu database thay cho JWT — mỗi request một câu truy vấn;
thời gian ân hạn cho dùng lại — mở lại đúng lỗ hổng mà rotation muốn đóng.

## D-046 · Role và permission — 2026-11-11 — agent quyết

Quyết định:

- **Permission do module khai** qua hàm mới `Module::permissions()` (mặc định rỗng —
  D-038). Tên `<module>.<việc>`, các đoạn chữ thường, số, `_`; core kiểm lúc đăng ký
  module (tên sai, trùng, thiếu mô tả là lỗi khởi động) và gom thành
  `PermissionCatalog`, đưa cho mọi module qua `StartContext::permissions()`.
- **Kernel có khái niệm "ai đang gọi"** (`autoxide_kernel::access`): `Principal` (ID
  người dùng + tập permission, hoặc "mọi permission"), trait `Authenticator` (đọc
  header `Authorization` → `Principal`), và extractor: handler viết `principal:
  Principal` rồi `principal.require(&PERM)?`; thiếu quyền trả 403 `permission_denied`
  nêu tên permission. Module `auth` công bố `dyn Authenticator`; binary lấy nó bằng
  `Modules::api()` và `http::build` gắn vào mọi request. Module nghiệp vụ chỉ phụ
  thuộc kernel, không phụ thuộc `auth`.
- **Role lưu trong module `auth`:** `auth_role`, `auth_role_permission`,
  `auth_user_role`. Người dùng có hợp các permission của mọi role của mình. Tính
  permission ở mỗi request bằng hai câu truy vấn (có role "mọi permission" không;
  các permission), không phụ thuộc số role — đổi role có hiệu lực ngay ở request kế,
  không chờ access token hết hạn.
- **Role `admin` dựng sẵn** (migration tạo), cờ `all_permissions`: có mọi permission,
  kể cả permission của module thêm sau. Không sửa, không xoá được; API không đặt
  được cờ này cho role khác. Không gỡ được role `admin` khỏi người cuối cùng đang có
  nó (`auth.last_admin`) — tránh khoá cả hệ thống.
- Permission của `auth`: `auth.roles.read`, `auth.roles.manage` (tạo/sửa/xoá role
  và gán role — tương đương toàn quyền, mô tả nói rõ), `auth.users.read`.
- Endpoint: `GET /api/v1/auth/permissions`, CRUD `/api/v1/auth/roles`,
  `GET|PUT /api/v1/auth/users/{id}/roles`; `/me` trả thêm permission.
- Gán role cho một người: transaction cập nhật dòng người dùng trước để khoá nó, nên
  các request đồng thời chạy lần lượt (test đồng thời trên hai database).

Lý do: kiểm theo permission thay vì tên role để người cài tự đặt role theo tổ chức
của họ mà code không đổi (RBAC chuẩn, NIST). Kernel giữ `Principal` để mọi module
— và sau này API key, MCP — dùng chung một cách kiểm quyền.

Đã cân nhắc: nhét permission vào access token — đổi role phải chờ token hết hạn;
permission dạng ký tự đại diện (`notes.*`) — khó đoán quyền thật của một role, và
role `admin` đã phủ nhu cầu "tất cả"; role theo cây kế thừa — phức tạp mà chưa có
nhu cầu.

## D-047 · Rate limiting và login throttling — 2026-11-11 — agent quyết

Quyết định:

- **Rate limiting** là một lớp HTTP của kernel, tự viết (~100 dòng,
  `crates/kernel/src/http/rate_limit.rs`) theo thuật toán GCRA: mỗi địa chỉ client
  nhớ một mốc thời gian, chỉ dùng số nguyên và `Instant`. Mặc định 600 request/phút,
  gửi dồn 60 (`AUTOXIDE_HTTP_RATE_LIMIT_PER_MINUTE`, `_BURST`; `0` là tắt). Quá giới
  hạn trả 429 `rate_limited` kèm `Retry-After`. Đếm trong bộ nhớ từng tiến trình;
  không đếm `/health/*`; IPv6 gom theo khối /64.
- **Địa chỉ client:** mặc định là địa chỉ của kết nối (máy chủ chạy với
  `into_make_service_with_connect_info`). Sau reverse proxy thì đặt
  `AUTOXIDE_HTTP_CLIENT_IP_HEADER`; lấy phần tử CUỐI của header (phần do proxy của
  mình thêm), vì các phần trước do client tự viết được.
- **`Error` mang được `Retry-After`** (`with_retry_after`), dùng chung cho 429 và 503.
  Lưu dạng `NonZeroU32` để `Error` vẫn ≤ 128 byte (clippy `result_large_err`).
- **Login throttling** trong module `auth`, đếm trong database (bảng
  `auth_login_throttle`) nên mọi máy chủ dùng chung: quá 5 lần sai trong 15 phút
  (`AUTOXIDE_AUTH_LOGIN_MAX_FAILURES`, `_LOCK_MINUTES`) thì khoá đăng nhập của email đó
  15 phút, kể cả mật khẩu đúng; trả 429 `auth.login_throttled` kèm `Retry-After`. Đếm
  cả email không tồn tại, lưu SHA-256 của email. Đăng nhập đúng xoá số đếm. Mỗi bước
  đếm là một `UPDATE` có điều kiện; test 8 lần sai đồng thời đếm đủ 8 trên hai
  database.

Lý do không dùng `tower_governor` 0.8 (STACK.md ghi trước đây): bộ feature mặc định
kéo `tonic` (gRPC); tắt đi vẫn thêm `governor`, `dashmap`, `forwarded-header-value`,
`thiserror` 1 cho việc vài chục dòng làm được (luật "không thêm thư viện khi tự viết
đủ"), và lỗi của nó không ra dạng Problem kèm `code`.

Giới hạn đã chấp nhận: kẻ xấu biết email có thể cố ý gõ sai để khoá người đó 15
phút. Cách chuẩn để giảm (CAPTCHA, khoá theo cặp thiết bị–email) cần giao diện, nằm
ngoài core. Bảng `auth_login_throttle` lớn dần với email lạ — dọn bằng job định kỳ
ở bước 5 (đã thêm vào lộ trình).

Đã cân nhắc: rate limiting trong database — mỗi request thêm một lần ghi; khoá theo
địa chỉ IP khi đăng nhập sai — kẻ tấn công đổi IP dễ, người dùng sau NAT bị khoá
chung; khoá vĩnh viễn tới khi quản trị viên mở — biến lỗi gõ sai thành việc của
quản trị viên.

## D-048 · Lệnh `autoxide user create` — 2026-11-11 — agent quyết

Quyết định:

- Lệnh `autoxide user create --email … --name … [--admin] [--password-stdin]` tạo tài
  khoản, chạy với quyền của người vận hành máy chủ (không đăng nhập, không kiểm
  permission). `--admin` gán role `admin` trong cùng transaction với việc tạo.
- Mật khẩu: không có `--password-stdin` thì sinh 18 byte ngẫu nhiên (24 ký tự
  base64url) và in ra đúng một lần; có thì đọc dòng đầu của standard input. Không
  nhận mật khẩu qua tham số dòng lệnh (lộ trong lịch sử shell và danh sách tiến
  trình) và không hỏi kiểu ẩn ký tự (cần thêm thư viện, và AI agent không gõ được).
- Binary gọi module `auth` qua trait mới `autoxide_auth::api::Accounts` (lấy bằng
  `Modules::api()`), không chạm phần bên trong module. Lệnh từ chối chạy khi còn
  migration chưa áp dụng, như `serve`.

Lý do: người cài cần một cách tạo quản trị viên đầu tiên mà không cần ai đăng nhập
trước; dòng lệnh là cách chuẩn (Django `createsuperuser`, Keycloak bootstrap admin).

## D-049 · Quản lý người dùng qua API và tự đăng ký — 2026-11-11 — agent quyết

Quyết định:

- Permission mới `auth.users.manage`: tạo tài khoản (`POST /api/v1/auth/users`), đổi
  tên, khoá và mở khoá (`PATCH /api/v1/auth/users/{id}`). Xem danh sách và một tài khoản
  cần `auth.users.read`. Gán role vẫn cần `auth.roles.manage` (D-046) — tạo tài khoản
  không kéo theo cấp quyền.
- **Khoá thay vì xoá:** tài khoản bị khoá giữ nguyên dữ liệu (module khác có thể đang
  giữ ID của nó — không foreign key xuyên module, D-025), không đăng nhập được, access
  token mất hiệu lực ngay ở request kế (đã kiểm tài khoản khoá mỗi request), refresh
  token bị thu hồi trong cùng transaction. Không khoá được admin cuối cùng còn hoạt
  động; phép đếm admin (cả khi gỡ role) từ nay chỉ tính tài khoản chưa khoá.
- **Tự đăng ký** `POST /api/v1/auth/register` (không cần đăng nhập) chỉ mở khi
  `AUTOXIDE_AUTH_SELF_REGISTRATION=true` (mặc định tắt — D-028); tắt thì trả 403
  `auth.registration_disabled`. Tài khoản tự đăng ký không có role nào; bắt buộc có
  mật khẩu. Rate limiting chung của kernel chặn tạo hàng loạt.

Giới hạn đã chấp nhận: đăng ký bằng email đã có trả `auth.email_taken`, nên ai đăng
ký được thì biết một email đã có tài khoản. Cách chặn hoàn toàn là xác minh email
trước khi tạo tài khoản — làm ở bước 7 (email).

Đã cân nhắc: xoá hẳn tài khoản — mất dấu vết cho audit log và để lại ID mồ côi ở module
khác; role mặc định cho người tự đăng ký — chưa có nhu cầu, thêm bằng cấu hình khi cần.

## D-050 · API key cho máy và AI agent — 2026-11-11 — agent quyết

Quyết định:

- **Dạng key:** `axk_` + 32 byte ngẫu nhiên base64url. Tiền tố cố định để công cụ quét
  secret nhận ra key lỡ bị commit, và để bộ xác thực phân biệt với access token. Gửi
  như access token: `Authorization: Bearer axk_…`. Database lưu SHA-256 của key (bảng
  `auth_api_key`) và 12 ký tự đầu để người dùng nhận ra key.
- **Quyền:** mỗi key ghi một danh sách permission (bảng `auth_api_key_permission`),
  chỉ chọn được trong số permission người tạo đang có (`not_granted` nếu không). Mỗi
  request, quyền thật = permission của key ∩ permission HIỆN TẠI của chủ key — chủ key
  mất quyền hay bị khoá thì key mất theo ngay. Không có key "mọi permission", kể cả
  khi chủ key là admin: key nên chỉ có đúng thứ việc của nó cần.
- **Không dùng API key để tạo API key** (`auth.api_key_cannot_create_keys`): key bị lộ
  không tự đẻ thêm key. Kernel thêm `Principal::via_api_key` / `api_key_id` để biết
  request đến bằng key.
- Hạn dùng tuỳ chọn (`expires_at`); hết hạn trả `auth.api_key_expired`. Thu hồi =
  xoá dòng, có hiệu lực ngay. Mỗi người tối đa 100 key. `last_used_at` ghi nhiều nhất
  một lần mỗi phút mỗi key (`UPDATE` có điều kiện).
- Endpoint: `GET|POST /api/v1/auth/api-keys`, `DELETE /api/v1/auth/api-keys/{id}` —
  mỗi người chỉ thấy và thu hồi key của mình.

Lý do: tiêu chí "có cách xác thực cho máy, không buộc qua bước đăng nhập tương tác"
(CLAUDE.md). Giao cắt với quyền hiện tại của chủ key giữ đúng nguyên tắc quyền tối
thiểu và không tạo đường vòng quanh việc đổi role hay khoá tài khoản.

Đã cân nhắc: tài khoản máy riêng (service account) — thêm một loại người dùng mà
key của người dùng thật đã đủ; lưu key bằng Argon2 — key đã ngẫu nhiên 256 bit, băm
chậm chỉ làm mỗi request chậm thêm; quản trị viên thu hồi key của người khác — khoá
tài khoản đã làm việc đó, thêm khi có nhu cầu.

## D-051 · TOTP — 2026-11-11 — agent quyết

Quyết định:

- Thư viện `totp-rs` 6.0 (đã chọn trong `STACK.md`), chỉ bật `std` và `otpauth`; nó
  dùng đúng `hmac` 0.13 / `sha2` 0.11 đang có, chỉ thêm `sha1`, `base32`,
  `constant_time_eq`. Tham số mọi ứng dụng authenticator hiểu: SHA-1, 6 chữ số, bước
  30 giây, nhận lệch một bước mỗi phía. Secret 160 bit sinh bằng `getrandom`.
- **Bật hai bước:** `POST /api/v1/auth/me/mfa/totp` trả secret (base32) và đường dẫn
  `otpauth://` để client vẽ QR (core không vẽ QR — D-026: core chỉ có API); `POST
  …/confirm` với một mã mới có hiệu lực. Tắt (`…/disable`) cần một mã. Các việc này
  cần phiên đăng nhập bằng mật khẩu, API key không làm được (`auth.session_required`)
  — key bị lộ không được dùng để chiếm tài khoản.
- **Mỗi mã dùng một lần** (RFC 6238 mục 5.2 — thư viện không tự làm): lưu số thứ tự
  bước của mã vừa nhận (`last_used_step`), chỉ nhận bước lớn hơn bằng một `UPDATE` có
  điều kiện; test sáu request cùng một mã đồng thời: đúng một được nhận, trên hai
  database.
- Quản trị viên (`auth.users.manage`) xoá MFA của người mất thiết bị:
  `DELETE /api/v1/auth/users/{id}/mfa`.
- Tên hiện trong ứng dụng: `AUTOXIDE_AUTH_TOTP_ISSUER` (mặc định `Autoxide`).
- **Secret lưu nguyên văn** trong `auth_totp` (máy chủ phải đọc được nó để kiểm mã).

Lý do không mã hoá secret: khoá mã hoá phải ổn định qua các lần khởi động, mà
`AUTOXIDE_AUTH_TOKEN_SECRET` được phép để trống (khoá ngẫu nhiên mỗi lần chạy) —
dùng nó thì mọi người dùng mất MFA sau khi khởi động lại. Kẻ đọc được database đã
đọc được cả hash mật khẩu và mọi dữ liệu khác; mã hoá riêng secret TOTP thêm một biến
cấu hình bắt buộc mà lợi ích nhỏ. Cân nhắc lại khi có biến khoá mã hoá dữ liệu chung.

Đã cân nhắc: tự viết TOTP — thư viện đã chọn, nhỏ và đúng chuẩn; trả ảnh QR —
kéo thư viện ảnh vào core trong khi client vẽ QR từ URL dễ dàng.

## D-052 · Recovery code — 2026-11-11 — agent quyết

Quyết định:

- Xác nhận TOTP sinh 10 recovery code, trả về một lần duy nhất. Mỗi mã 80 bit ngẫu
  nhiên, viết 16 ký tự base32 chữ thường chia nhóm 4 (`abcd-efgh-ijkl-mnop`) —
  không có ký tự dễ nhầm như `0`/`O`, `1`/`l`. Gõ hoa, thừa khoảng trắng hay thiếu
  `-` vẫn nhận. Database lưu SHA-256 của mã (bảng `auth_recovery_code`): 80 bit
  ngẫu nhiên thì băm nhanh vẫn không đoán ngược được.
- Recovery code thay cho mã TOTP ở chỗ cần "lớp thứ hai" (tắt MFA, và đăng nhập ở
  mục sau); mỗi mã dùng một lần bằng `UPDATE … WHERE used_at IS NULL` (test đồng thời
  trên hai database).
- Sinh lại bộ mới (`POST /api/v1/auth/me/mfa/recovery-codes`) cần mã TOTP, không nhận
  recovery code — ai chỉ cầm một recovery code không tự đẻ thêm mã. Tắt MFA hoặc quản
  trị viên xoá MFA thì xoá luôn các mã còn lại. `GET /me/mfa` cho biết còn bao nhiêu mã.
- Base32 cho đúng 10 byte tự viết (~15 dòng, có vector thử của RFC 4648) thay vì
  thêm thư viện.

Lý do: mất điện thoại không được đồng nghĩa với mất tài khoản khi không có quản trị
viên giúp; recovery code là cách chuẩn (GitHub, Google) và không cần email.

## D-053 · Đăng nhập hai bước — 2026-11-11 — agent quyết

Quyết định:

- `POST /api/v1/auth/login` với tài khoản đã bật MFA trả **202 Accepted** kèm
  `{mfa_token, expires_in, methods}` thay vì token; client gửi `mfa_token` cùng mã
  TOTP hoặc recovery code tới `POST /api/v1/auth/login/mfa` để nhận cặp token. Mã
  trạng thái khác (200 / 202) và hình dạng khác cho hai kết quả: client và AI agent
  rẽ nhánh theo mã trạng thái, không phải đoán theo trường.
- `mfa_token`: 32 byte ngẫu nhiên, database giữ SHA-256 (bảng `auth_mfa_challenge`),
  sống 5 phút, cấp token đúng một lần (xoá dòng có điều kiện — test đồng thời trên hai
  database), cho sai tối đa 5 lần.
- Mỗi mã sai cũng ghi một lần sai vào login throttling của email: kẻ đã biết mật khẩu
  không mở được lượt mới vô hạn để đoán 10⁶ mã. Số lần sai chỉ được xoá khi qua cả hai
  bước.
- Đổi ở code: `SessionService::login` trả `LoginOutcome` (`Tokens` | `MfaRequired`).

Lý do: cách chuẩn của các hệ thống có MFA (đăng nhập theo bước, token tạm của bước
hai). Không gói mã vào chung request đăng nhập để client không phải biết trước tài
khoản có bật MFA hay không.

Đã cân nhắc: `mfa_token` là JWT không lưu database — không đếm được số lần sai và
không bảo đảm dùng một lần; trả 401 với mã `auth.mfa_required` — 401 nghĩa là chưa
xác thực được, trong khi mật khẩu đã đúng.

## D-054 · Event: transactional outbox — 2026-11-11 — agent quyết

Quyết định:

- **Mỗi module phát event có bảng `<module>_events` của chính nó**, tạo bằng
  migration kernel cung cấp (`event::outbox_migration(tên, module)`), module tự đặt vào
  danh sách migration của mình. Kernel đọc/ghi bảng này như đã làm với bảng lịch sử
  `<module>_migrations` — luật sở hữu bảng vẫn giữ (bảng mang tiền tố module, không
  module nào ghi vào bảng của module khác).
- **Ghi:** `Outbox::record(&transaction, actor, subject, &event)` trong CÙNG transaction
  với thay đổi. Event là kiểu `Serialize` cài trait `Event` (`NAME = "<module>.<việc>"`);
  payload lưu JSON dạng chữ (chạy giống nhau trên hai database). Mỗi event mang `actor`
  (người dùng, API key, hoặc hệ thống) và `subject` (`<loại>:<id>`) — đủ cho audit log.
- **Hợp đồng module thêm** `events()` (khai báo, kiểm tên lúc khởi động) và
  `subscriptions()` (nhận một event theo tên, hoặc mọi event với `Subscription::ALL`);
  đăng ký nhận event không ai khai là lỗi khởi động.
- **Giao:** `Dispatcher` của kernel chạy nền trong `serve`, hỏi mỗi giây hoặc ngay khi
  có event mới (chuông `Notify`), theo thứ tự ID trong từng bảng. Nhận việc giao một
  event bằng `UPDATE … WHERE claimed_until IS NULL OR claimed_until < now` — nhiều tiến
  trình chạy cùng lúc không giao trùng (test hai bộ phát đồng thời, 30 event, trên hai
  database), tiến trình chết giữa chừng thì hết hạn giữ (60 giây) và tiến trình khác
  giao lại. Lỗi thì thử lại với backoff 2, 4, 8… giây (tối đa 1 giờ); sau 10 lần đánh
  dấu `failed_at`, ghi log lỗi và số đo `events_failed_total`.
- **Giao ít nhất một lần:** máy chủ tắt giữa lúc giao thì event được giao lại; một bên
  nhận lỗi thì mọi bên nhận của event đó nhận lại. Bên nhận phải bỏ qua event trùng
  theo `id`.

Lý do: "việc phụ đi qua event, ghi cùng transaction" là luật kiến trúc 3 của
`CLAUDE.md`; transactional outbox là cách chuẩn duy nhất bảo đảm không mất và không bịa
event mà không cần hệ thống thứ hai (message broker). Không dùng `SKIP LOCKED` của
Postgres để mọi câu lệnh chạy giống nhau trên SQLite; job queue ở bước 5 sẽ đo lại và
quyết định có cần nhánh riêng không (D-009).

Đã cân nhắc: một bảng event chung của kernel — phá luật mọi bảng thuộc một module;
phát event trong bộ nhớ ngay sau commit — mất event khi tiến trình chết giữa commit và
lúc phát; mỗi bên nhận một dòng theo dõi riêng — chính xác hơn khi giao lại nhưng nhân
số dòng theo số bên nhận, chưa cần.

## D-055 · Audit log là một module nhận mọi event — 2026-11-11 — agent quyết

Quyết định:

- **Module `audit`** (`modules/audit`) đăng ký nhận MỌI event (`Subscription::ALL`) và
  ghi mỗi event thành một dòng `audit_entry`: module, hành động (tên event), người làm
  (người dùng, API key; trống là hệ thống), bản ghi (`subject`), trước / sau, thời điểm
  xảy ra và thời điểm ghi. Không module nào gọi audit trực tiếp; thêm module mới mà
  phát event thì tự có audit, không sửa audit.
- **Khoá chính của `audit_entry` là ID của event** (UUIDv7 sinh lúc xảy ra): event
  giao lại (giao ít nhất một lần, D-054) thì dòng đã có — bỏ qua; sắp theo ID là sắp
  theo thời gian xảy ra.
- **Quy ước payload "trước / sau":** event mang `{"before": …, "after": …}` thì hai phần
  vào cột `before` / `after`, phần còn lại vào `details`. Mọi event của `auth` theo quy
  ước này; event không bao giờ chứa mật khẩu, hash, secret TOTP, token, API key hay
  recovery code (có test kiểm).
- **Chỉ thêm:** code của module không có lệnh sửa hay xoá dòng audit. Không dùng
  trigger chặn ở database vì cú pháp trigger khác nhau giữa SQLite và Postgres (luật
  số một); người có quyền ghi thẳng vào database vẫn sửa được — chuỗi băm chống sửa để
  sau (D-008).
- **Module `auth` phát event cho mọi thay đổi:** tạo / sửa người dùng, đổi mật khẩu,
  đăng nhập / đăng xuất, phát hiện dùng lại refresh token, tạo / sửa / xoá role, đổi
  role của người dùng, tạo / thu hồi API key, bật / tắt MFA, sinh lại recovery code.
  Service nhận thêm tham số `Actor`: handler truyền `Actor::from(&principal)`, lệnh CLI
  và tự đăng ký truyền `Actor::system()`.

Lý do: luật kiến trúc 3 ("việc phụ đi qua event") và yêu cầu audit "ai, làm gì, bản
ghi nào, lúc nào, trước / sau" (lộ trình bước 4). Audit ghi sau khi commit (qua bộ phát)
chứ không cùng transaction, nhưng không thể mất: event đã nằm trong database cùng
transaction với thay đổi.

Đã cân nhắc: mỗi module tự ghi audit — lặp code ở mọi module và dễ quên; audit trong
kernel — phá luật mọi bảng thuộc một module.

## D-056 · Job queue — 2026-11-11 — agent quyết

Quyết định (thực hiện D-008 và D-009 — chủ dự án quyết):

- **Bảng job riêng của từng module** (`<module>_jobs`, tạo bằng
  `job::jobs_migration`), như bảng event (D-054). Module đưa job vào hàng đợi bằng
  `JobQueue::enqueue` (lấy từ `StartContext::jobs()`), trong transaction của thay đổi
  nếu có. Loại job tên `<module>.<việc>`; module khai bộ chạy qua `Module::jobs` và
  lịch qua `Module::schedules` (mặc định rỗng — D-038).
- **Hai cách claim sau trait `Claim`** (D-009): Postgres `UPDATE … WHERE id IN
  (SELECT … FOR UPDATE SKIP LOCKED) RETURNING`; SQLite cùng câu đó không có khoá (SQLite
  chỉ một bên ghi nên câu lệnh đã nguyên tử). Dựng bằng `sea_query`, không viết SQL
  tay. Gate chạy test claim đồng thời (3 worker × 40 job; 5 task gọi thẳng `Claim`) trên
  cả hai database, nên cả hai nhánh đều được test.
- **Chạy:** claim tăng `attempts`, giữ job 5 phút; mỗi module tối đa
  `AUTOXIDE_JOBS_CONCURRENCY` job cùng lúc (mặc định 4); bộ chạy quá 4,5 phút bị coi là
  lỗi. Ghi kết quả chỉ khi job còn là của worker đó. Lỗi thì thử lại sau 2, 4, 8…
  giây (tối đa 1 giờ); quá `max_attempts` (mặc định 10) thì đặt `failed_at` —
  **dead-letter queue** nằm ngay trong bảng, xem bằng `autoxide jobs failed`, chạy lại
  bằng `autoxide jobs retry <module> <id>`. Job xong được xoá sau 7 ngày.
- **Lịch định kỳ:** mỗi lịch có khoảng `every`; mỗi khoảng sinh một job với
  `unique_key = schedule:<tên>:<số thứ tự khoảng>` — index unique bảo đảm nhiều worker
  chạy cùng lúc chỉ sinh một job (có test). Chỉ hỗ trợ "mỗi N giây", chưa có cú pháp
  cron. Lịch đầu tiên: `auth.cleanup` mỗi giờ dọn số đếm đăng nhập sai cũ, refresh
  token hết hạn hay đã thu hồi, lượt MFA hết hạn. Bộ phát event cũng tự xoá event đã
  giao quá 7 ngày.
- **Process role** `AUTOXIDE_PROCESS_ROLE`: `all` (mặc định), `api` (chỉ API), `worker`
  (chỉ việc nền — job, lịch, giao event — kèm health check). Cùng một binary, cùng lệnh
  `serve`.
- Chạy **ít nhất một lần**: worker chết giữa chừng thì job được chạy lại khi hết hạn
  giữ; bộ chạy phải chịu được việc chạy lại.

Lý do: D-008 (job queue trong database, không Redis; retry, lịch, dead-letter queue)
và D-009 (hai cách claim sau một trait, test đồng thời trên hai database). Dead-letter
queue nằm chung bảng thay vì bảng riêng: chuyển job sang bảng khác là thêm một bước có
thể hỏng giữa chừng, mà `failed_at` đã đủ để lọc.

Đã cân nhắc: cú pháp cron cho lịch — cần thêm thư viện hoặc tự viết bộ phân tích,
chưa có nhu cầu (thêm khi có lịch "3 giờ sáng mỗi ngày"); API REST cho dead-letter
queue — core chưa có permission cho người vận hành hệ thống, CLI đủ cho người cài.

## D-057 · i18n: bản dịch theo `code`, dịch ở lớp HTTP — 2026-11-11 — agent quyết

Quyết định:

- **Không dùng `rust-i18n`** (STACK.md ghi trước đây). Nó dịch bằng macro `t!` trong
  từng crate lúc biên dịch; ở đây thông báo cần dịch là lỗi đã tạo xong ở service,
  dịch ở MỘT chỗ (lớp HTTP) theo `code`, với bản dịch do từng module mang theo. Phần đó
  khoảng 150 dòng tự viết (`crates/kernel/src/i18n.rs`).
- **Bản dịch:** mỗi crate có `locales/vi.json` (`{"code": "câu"}`), cùng chỗ điền `{tên}`
  như câu tiếng Anh; module khai qua `Module::translations` (mặc định rỗng — D-038).
  Module chỉ được dịch `code` của mình (`<module>.…`); kernel dịch `code` không tiền tố.
  File sai là lỗi khởi động. Câu tiếng Anh vẫn nằm ở chỗ tạo lỗi (D-039).
- **Chọn ngôn ngữ:** lớp HTTP `localize` đọc `Accept-Language` (có xét `q`), dịch
  `detail` của Problem theo `code` và giá trị chỗ điền của chính lỗi đó, gắn
  `Content-Language`. Không nêu, hay nêu ngôn ngữ chưa có: tiếng Anh (D-027). Bản dịch
  dùng chỗ điền mà lỗi không có giá trị thì rơi về tiếng Anh, không bao giờ lộ `{tên}`.
  Lỗi do thư viện sinh ra (404, 405…) cũng mang `Error` trong extensions nên dịch được.
- **Gate:** `cargo xtask check` đọc mã nguồn kernel và module (bỏ chú thích và phần
  `#[cfg(test)]`), tìm `code` ở mọi chỗ tạo lỗi, báo `code` thiếu bản dịch, bản dịch
  thừa, và `code` của module thiếu tiền tố `<module>.`.

Giới hạn đã chấp nhận ở mục này: `title` (cụm chuẩn của mã HTTP) và thông báo của từng
trường (`errors[].detail`) vẫn tiếng Anh. Thông báo trường được thêm thành mục riêng
của lộ trình; `code` của trường (`length`, `email`…) đã ổn định cho máy.

Đã cân nhắc: dịch ở chỗ tạo lỗi (service biết ngôn ngữ) — mọi service phải nhận thêm
ngôn ngữ, và lỗi trong log đổi theo ngôn ngữ người gọi; file TOML/YAML — thêm thư viện
đọc file trong khi JSON đã có `serde_json`.

## D-058 · Idempotency key — 2026-11-11 — agent quyết

Quyết định:

- Header `Idempotency-Key` theo bản nháp IETF ("The Idempotency-Key HTTP Header
  Field"), áp cho `POST`, `PUT`, `PATCH`, `DELETE` của MỌI endpoint, xử lý ở một lớp
  HTTP của kernel — module không phải làm gì.
- **Phạm vi key:** người gọi (SHA-256 của header `Authorization`) + key. Hai người dùng
  chọn trùng key không đụng nhau. **Dấu vân tay request:** SHA-256 của method, đường
  dẫn và thân request; cùng key mà dấu khác → 422 `idempotency_key_reused`.
- **Lần đầu:** chèn dòng `(scope, key)` — index unique quyết đúng một request chạy;
  request trùng đến khi lần đầu chưa xong → 409 `idempotency_key_in_progress` kèm
  `Retry-After: 1`; đã xong → trả lại đúng status, header (`content-type`, `location`,
  `retry-after`, `www-authenticate`, `content-language`) và thân đã lưu, kèm
  `Idempotent-Replayed: true`. Lỗi 5xx không lưu (bỏ dòng) để client gửi lại được. Lần
  đầu chết giữa chừng thì sau `thời gian tối đa của request + 30 giây` request sau nhận
  quyền chạy (`UPDATE` có điều kiện). Giữ key 24 giờ; job định kỳ dọn mỗi giờ.
- **Lưu trữ sau trait** `kernel::idempotency::IdempotencyStore`, do module mới
  `idempotency` (bảng `idempotency_record`) cung cấp — như `Authenticator` do `auth`
  cung cấp. Không đăng ký module thì kernel bỏ qua header.
- Vị trí trong lớp HTTP: trong rate limiting (request bị chặn vì gửi quá nhiều thì
  không lưu), ngoài lớp ngôn ngữ (lưu đúng response client đã nhận).

Lý do: tiêu chí "lệnh ghi an toàn khi gọi lại, vì agent hay retry" (CLAUDE.md). Làm ở
lớp HTTP để mọi endpoint của mọi module có ngay, không module nào quên.

Đã cân nhắc: idempotency trong từng service — mỗi module tự làm, dễ sót và mỗi nơi
một kiểu; khoá bằng lock trong bộ nhớ — sai khi chạy nhiều máy chủ; bắt buộc key cho
mọi lệnh ghi — phá client đơn giản, bản nháp IETF để tuỳ chọn.

Đã kiểm: sáu request cùng key gửi đồng thời → handler chạy đúng một lần, trên cả hai
database.

## D-059 · Endpoint deprecated — 2026-11-11 — agent quyết

Quyết định:

- Module khai endpoint sắp bỏ qua `Module::deprecations()` (mặc định rỗng — D-038):
  method, đường dẫn trong module, ngày bắt đầu (`since`), ngày bỏ hẳn (`sunset`, tuỳ
  chọn), endpoint thay thế (tuỳ chọn). Ngày sai dạng hay `sunset` không sau `since` là
  lỗi khởi động.
- Kernel đặt `deprecated: true` cho operation trong OpenAPI và thêm câu "Deprecated
  since …; removed on …; use … instead" vào mô tả; response của endpoint đó mang
  `Deprecation: @<unix time>` (RFC 9745), `Sunset: <HTTP-date>` (RFC 8594) và
  `Link: <thay thế>; rel="successor-version"`. Đếm số lần gọi bằng
  `http_deprecated_requests_total{route}` để biết khi nào bỏ được.
- Thay đổi phá vỡ: làm endpoint mới (hoặc `/api/v2`), giữ endpoint cũ chạy y như cũ tới
  ngày sunset (luật ghi trong `CLAUDE.md`).

Lý do: chuẩn IETF cho việc báo endpoint sắp bỏ — client và AI agent đọc được từ
response và từ tài liệu, không cần đọc ghi chú phát hành. Một chỗ khai, kernel lo cả
tài liệu lẫn header, nên hai thứ không lệch nhau.

Đã cân nhắc: dùng `deprecated` của macro utoipa — chỉ đánh dấu tài liệu, không có
ngày, không có header.

## D-060 · Không dịch thông báo của từng trường ở máy chủ — 2026-11-11 — agent quyết

Quyết định: bỏ mục lộ trình "Dịch cả thông báo của từng trường (`errors[].detail`)"
(mục agent tự thêm ở D-057). `detail` của Problem đã dịch theo `Accept-Language`;
`errors[]` giữ `field` và `code` ổn định cho máy, `detail` của từng trường bằng tiếng Anh.

Lý do: giao diện hiển thị lỗi cạnh từng ô nhập thường tự dịch theo cặp
(`field`, `code`) bằng bộ chữ của chính nó; AI agent đọc `code`. Dịch ở máy chủ cần đổi
mọi chỗ tạo `FieldError` (khoảng 25 chỗ, đang ghép câu bằng `format!`) sang câu mẫu có
chỗ điền, kèm cách quét mới trong `cargo xtask check` — công sức lớn so với lợi ích,
trong khi các bước 7–11 của lộ trình còn nguyên.

Mở lại khi: một sản phẩm dùng Autoxide cần thông báo trường tiếng Việt từ máy chủ
(vd client không có bộ chữ riêng). Cách làm khi đó: `FieldError` mang câu mẫu và chỗ
điền như `Error`, khoá dịch `field.<code>` trong `locales/vi.json`.
