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

## D-061 · Gửi mail — 2026-11-11 — agent quyết

Quyết định:

- **Trait `kernel::mail::Mailer`** (`send(Mail)`), như `Authenticator` và
  `IdempotencyStore`: module cần gửi mail `require::<dyn Mailer>()` mà không phụ thuộc
  module gửi. Module mới **`mail`** cung cấp nó và phải đăng ký trước module dùng nó.
- `send` chỉ kiểm địa chỉ rồi đưa mail vào job queue của module `mail` (job
  `mail.send`); gửi chạy nền, lỗi (máy chủ SMTP tạm hỏng) thì thử lại với backoff, quá
  10 lần vào dead-letter queue (D-056). Log chỉ ghi ID của job, không ghi địa chỉ hay
  nội dung. Nội dung mail nằm trong `mail_jobs` tới khi được dọn (7 ngày): không gửi
  mật khẩu qua mail.
- **Đường gửi:** `AUTOXIDE_MAIL_SMTP_URL` (secret; `smtps://…` hay
  `smtp://…?tls=required`) thì gửi bằng `lettre` 0.11; không đặt thì ghi file `.eml`
  vào `AUTOXIDE_MAIL_FILE_DIR` (mặc định `data/mail`) và cảnh báo lúc khởi động — người
  lạ chạy thử được mà không cần máy chủ mail. Người gửi: `AUTOXIDE_MAIL_FROM`.
- `lettre` bỏ bộ feature mặc định (native-tls kéo OpenSSL); TLS dùng `rustls` với
  `ring` — cùng thư viện mật mã với `sqlx`, tránh xung đột hai provider (D-043).
- `deny.toml` cho phép license **0BSD** (của `quoted_printable`, bộ mã hoá nội dung mail
  của `lettre`): thoáng hơn MIT, hợp với MIT OR Apache-2.0 của Autoxide.

Đã kiểm: test gửi qua một máy chủ SMTP giả chạy trong test và qua file, trên hai
database.

Đã cân nhắc: gửi mail ngay trong request — request chậm theo máy chủ mail và mất mail
khi máy chủ mail hỏng; gửi bằng event — mail không phải "việc phụ của một thay đổi" mà
là việc chính được yêu cầu, job queue hợp hơn.

## D-062 · Mẫu mail theo i18n key — 2026-11-11 — agent quyết

Quyết định: `kernel::mail::MailTemplate::new(key, subject, text)` — tiêu đề và nội dung
tiếng Anh nằm trong code (có chỗ điền `{tên}`), như câu của lỗi (D-039); bản dịch nằm
trong `locales/vi.json` của crate với khoá `<key>.subject` và `<key>.text`, khoá đặt
`<module>.mail.<tên>`. `render` / `mail` điền theo ngôn ngữ; thiếu bản dịch, hay bản
dịch dùng chỗ điền không có giá trị, thì dùng tiếng Anh. Ngôn ngữ của mail là ngôn ngữ
của request gây ra mail (extractor `Language` đọc `Accept-Language`); module lấy bản
dịch lúc khởi động qua `StartContext::translations`. `cargo xtask check` thấy
`MailTemplate::new("…")` và đòi đủ hai bản dịch. Mail chỉ có nội dung chữ; HTML thêm khi
có mẫu cần.

Lý do: một cách dịch duy nhất cho mọi thông báo (D-057); người lạ đọc code thấy ngay
câu tiếng Anh, bản dịch nằm cạnh bản dịch lỗi của cùng module.

Đã cân nhắc: lưu ngôn ngữ ưa thích của người dùng — cần thêm cột và API; ngôn ngữ
theo request đủ cho các mail hiện có (đặt lại mật khẩu, xác minh email đều do chính
người nhận yêu cầu).

## D-063 · Đặt lại mật khẩu bằng token một lần qua mail — 2026-11-11 — agent quyết

Quyết định: `POST /auth/password-reset` luôn trả 202, không nói email có tài khoản hay
không; email có thật và chưa bị khoá thì nhận mail chứa token ngẫu nhiên 32 byte
(base64url). Database chỉ giữ SHA-256 của token (bảng `auth_password_reset`), hạn 30
phút, mỗi email tối đa một mail mỗi 60 giây. `POST /auth/password-reset/confirm` kiểm
luật mật khẩu trước (sai luật không đốt token), rồi đánh dấu token đã dùng bằng một câu
`UPDATE … WHERE used_at IS NULL` có điều kiện — nhiều request cùng token thì đúng một
bên thắng trên cả hai database; đặt mật khẩu qua `UserService::set_password` (huỷ mọi
phiên), rồi vô hiệu các token còn lại của người đó. Token sai, đã dùng hay hết hạn
cùng một lỗi `auth.invalid_reset_token`. `AUTOXIDE_AUTH_PASSWORD_RESET_URL` (chứa
`{token}`) cho phép gửi đường dẫn tới giao diện riêng; không đặt thì mail chứa token trần.
Ghi event `auth.password_reset_requested`.

Lý do: chuẩn OWASP cho quên mật khẩu; token lưu dạng hash như refresh token và API key
(lộ database không lộ token); core chỉ có API (luật kiến trúc 9) nên trang nhập mật khẩu
mới thuộc giao diện riêng.

Đã cân nhắc: JWT có hạn thay cho bảng — không thu hồi được và không bảo đảm dùng một
lần; mã số 6 chữ — dễ đoán, cần thêm giới hạn số lần thử.

## D-064 · Tự đổi mật khẩu và mật khẩu đầu tiên do quản trị viên đặt — 2026-11-11 — agent quyết

Quyết định:

- `POST /auth/me/password` (`current_password`, `new_password`): chỉ trong phiên đăng
  nhập bằng mật khẩu, không qua API key. Sai mật khẩu hiện tại trả 400
  `auth.wrong_password` và được tính vào login throttling của email như một lần đăng
  nhập sai. Đổi xong thì mọi phiên kết thúc, response trả cặp token của một phiên mới
  để client đang dùng khỏi đăng nhập lại.
- `PUT /auth/users/{id}/password` (permission `auth.users.manage`): chỉ cho tài khoản
  chưa có mật khẩu; đã có thì 409 `auth.password_already_set`. Điều kiện nằm ngay trong
  câu `UPDATE … WHERE password_hash IS NULL`, nên hai quản trị viên đặt cùng lúc thì
  đúng một người thắng trên cả hai database.

Lý do: người giữ `auth.users.manage` không được chiếm tài khoản của người khác (kể cả
của admin) bằng cách thay mật khẩu; người quên mật khẩu đã có đường đặt lại qua mail
(D-063). Tính lần sai vào throttling để access token bị lộ không thành công cụ dò mật
khẩu.

Đã cân nhắc: quản trị viên đặt lại mật khẩu bất kỳ — mở đường leo thang quyền; giữ
phiên hiện tại khi đổi mật khẩu — phải phân biệt phiên nào là "hiện tại" trong access
token, phức tạp hơn mà không an toàn hơn.

## D-065 · Xác minh email và đổi email qua mail — 2026-11-11 — agent quyết

Quyết định:

- Cột mới `auth_user.email_verified_at` (cho phép NULL; tài khoản có sẵn coi như chưa
  xác minh), hiện trong `User` của `api`, trong response người dùng và `/me`.
- Bảng `auth_email_verification` (user, địa chỉ cần xác minh, SHA-256 của token, hạn
  24 giờ, đã dùng chưa). Mỗi địa chỉ nhận nhiều nhất một mail mỗi 60 giây.
- Tạo tài khoản qua REST (tự đăng ký hay quản trị viên tạo) gửi mail xác minh; gửi hỏng
  chỉ ghi log vì tài khoản đã tạo xong — người dùng xin gửi lại bằng
  `POST /auth/me/email/verification`. Tài khoản tạo bằng CLI không nhận mail.
- Đổi email: `POST /auth/me/email` cần phiên đăng nhập bằng mật khẩu và mật khẩu hiện
  tại (sai thì tính vào login throttling, dùng chung `SessionService::check_password`
  với đổi mật khẩu). Mail đi tới địa chỉ MỚI; email chỉ đổi khi token được xác nhận.
- `POST /auth/email-verification/confirm` không cần đăng nhập. Trong MỘT transaction:
  đánh dấu token đã dùng bằng `UPDATE … WHERE used_at IS NULL` (cùng token đồng thời chỉ
  một bên thắng), ghi email + `email_verified_at`, vô hiệu mọi token xác minh khác của
  người đó, ghi event `auth.email_verified`. Địa chỉ đã bị người khác lấy trong lúc chờ
  thì index unique chặn → 409 `auth.email_taken`, không gì đổi.
- Chưa xác minh **không** chặn đăng nhập: core chỉ ghi nhận trạng thái; module hay giao
  diện tự quyết việc nào cần email đã xác minh.

Lý do: không ai đổi email sang địa chỉ không phải của mình (email là tên đăng nhập và
là nơi nhận mail đặt lại mật khẩu); token một lần, lưu dạng hash như D-063.

Đã cân nhắc: chặn đăng nhập khi chưa xác minh — khoá ngoài người dùng do CLI tạo và
cài đặt chưa có SMTP; gửi thêm mail báo cho địa chỉ cũ khi email đổi — để sau, khi có
nhu cầu.

## D-066 · Lưu file: trait `Storage` của kernel, module `files` trên `object_store` — 2026-11-11 — agent quyết

Quyết định:

- Kernel có trait `storage::Storage` (`put` / `get` / `delete` theo khoá, nội dung là
  luồng byte `ByteStream` — file lớn không nằm trọn trong bộ nhớ), như `Mailer`: module
  `files` công bố `dyn Storage`, module khác lấy bằng `context.require` (đăng ký sau
  `files`).
- Module `files` cài trait bằng `object_store` 0.14: thư mục `AUTOXIDE_FILES_DIR`
  (mặc định `data/files`, tạo khi ghi file đầu tiên, có fsync) hoặc S3 khi đặt
  `AUTOXIDE_FILES_S3_BUCKET`. Thông tin S3 dùng tên chuẩn của AWS (`AWS_REGION`,
  `AWS_ENDPOINT_URL`, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`), đọc qua
  `ConfigReader` như mọi biến khác (không để thư viện tự đọc môi trường). Ghi qua
  `BufWriter` của `object_store`: file nhỏ ghi một lần, file lớn ghi multipart từng
  phần 8 MiB; luồng lỗi giữa chừng thì huỷ multipart, không để lại file dở.
- Một thư viện mật mã: feature `aws-base` + `ring` thay cho `aws` (kéo aws-lc-rs);
  `reqwest` dùng rustls không kèm provider. Vì `reqwest` là một bản dùng chung (exporter
  OTLP cũng dùng), kernel có `tls::install_crypto_provider()` cài `ring` cho cả tiến
  trình; chỗ nào dựng HTTP client (exporter OTLP, kho S3) gọi nó trước (D-043).
- Giới hạn thân request theo route: `Module::body_limits` (mặc định rỗng — D-038) khai
  `BodyLimit { method, path, max_bytes }`; lớp `body_limit` của kernel (chạy sau khi
  chọn route, như lớp deprecated) dùng giới hạn đó thay cho `AUTOXIDE_HTTP_BODY_LIMIT_BYTES`.
  Lớp idempotency đọc thân theo cùng giới hạn. `DefaultBodyLimit` của axum tắt đi vì
  lớp này đã giới hạn mọi route.
- Test của nơi lưu chạy trên ổ đĩa, và trên S3 khi `TEST_S3_ENDPOINT` được đặt
  (testkit đọc biến, `s3_settings()`). Gate dựng máy chủ S3 giả lập `moto`
  (`motoserver/moto`) trong Docker — hoặc dùng `GATE_S3_ENDPOINT` — và đặt biến đó cho
  cả hai lượt test.

Lý do: `object_store` (Apache Arrow) là thư viện chuẩn, một API cho mọi nơi lưu, có
sẵn multipart và huỷ multipart; trait ở kernel giữ module nghiệp vụ không phụ thuộc
thư viện đó. Test S3 trên máy chủ giả lập chạy được ở mọi máy mà không cần secret thật.

Đã cân nhắc: `aws-sdk-s3` — rất nặng, chỉ S3; MinIO để test — bản mã nguồn mở đã ngừng
phát hành (dl.min.io trả 410, ngày 11/11/2026); `LocalFileSystem::new_with_prefix` —
đòi thư mục có sẵn lúc khởi động (test và lệnh CLI sẽ tạo thư mục rác).

## D-067 · API file: thân request là nội dung thô, metadata trong `files_file` — 2026-11-11 — agent quyết

Quyết định:

- `POST /files?name=…` nhận **nguyên nội dung file làm thân request** (kèm
  `Content-Type`), không nhận form multipart: đọc theo luồng thẳng vào nơi lưu, dễ gọi
  từ máy và AI agent (`curl --data-binary`), không cần thêm thư viện. Trả `FileResponse`
  (id, tên, loại, kích thước, SHA-256, chủ, lúc tạo). Gửi lại an toàn qua
  `Idempotency-Key`.
- `GET /files` (file của mình, phân trang), `GET /files/{id}`, `GET /files/{id}/content`
  (luôn `Content-Disposition: attachment`, `X-Content-Type-Options: nosniff`, `ETag` là
  SHA-256), `DELETE /files/{id}`.
- Permission `files.upload` (tải lên, và xem / tải về / xoá file của mình) và
  `files.manage` (với tới file của mọi người). File người khác mà không có quyền thì
  báo 404 như không tồn tại.
- Bảng `files_file` (chủ chỉ giữ ID — luật kiến trúc 4); nội dung ở khoá `files/<id>`.
  Tải lên: ghi nội dung trước, rồi dòng + event `files.file_uploaded` trong một
  transaction, hỏng thì xoá nội dung. Xoá: xoá dòng + event `files.file_deleted` trước
  (hai request xoá cùng lúc thì một bên nhận 404), rồi xoá nội dung; xoá nội dung hỏng
  chỉ để lại file mồ côi ở nơi lưu (ghi log).
- Giới hạn: `AUTOXIDE_FILES_MAX_BYTES` (mặc định 25 MiB) — module khai làm giới hạn
  thân request của route tải lên (D-066), service còn đếm lại trong lúc đọc (cho cả
  đường MCP sau này); `AUTOXIDE_FILES_ALLOWED_TYPES` (danh sách `loại/kiểu` hoặc
  `loại/*`, không đặt: mọi loại) — kiểm trên `Content-Type` đã khai, trước khi đọc nội
  dung; trả 413 `files.too_large` / 415 `files.type_not_allowed`.

Lý do: một cách tải lên duy nhất, chạy theo luồng nên file lớn không tốn bộ nhớ; tải về
luôn là attachment + nosniff nên loại đã khai không làm hại được trình duyệt dù sai.

Đã cân nhắc: multipart/form-data — tiện cho form HTML nhưng cần thư viện phân tích và
khó gọi từ máy hơn (giao diện là dự án riêng, gửi được thân thô); đoán loại theo nội
dung (magic number) — cần thêm thư viện, mà loại khai sai không gây hại nhờ hai header
trên; thêm sau nếu có nhu cầu.

## D-068 · MCP endpoint: `rmcp` ở kernel, tool khai qua hợp đồng module — 2026-11-11 — agent quyết

Quyết định:

- Kernel phục vụ MCP (Streamable HTTP) ở `/mcp` bằng `rmcp` 3.9 (SDK chính thức), chế
  độ **không giữ phiên, trả JSON**: mỗi request độc lập, nhiều máy chủ chạy song song
  như REST. Chỉ có khi một module công bố `Authenticator`.
- Xác thực ở lớp chạy trước `rmcp`, cùng `Authenticator` với REST (access token hay
  API key); thiếu hay sai thì 401 kiểu Problem kèm `WWW-Authenticate: Bearer`. Kiểm
  `Host` của `rmcp` tắt (chỉ có ích cho máy chủ không xác thực); kiểm `Origin` theo
  cùng danh sách `AUTOXIDE_HTTP_CORS_ORIGINS`. CORS cho phép thêm header của MCP.
- `Module::tools()` (mặc định rỗng — D-038) trả `kernel::mcp::Tool`: tên ngắn (kernel
  thêm `<module>.`), mô tả tiếng Anh, đầu vào là kiểu Rust `Deserialize + Validate +
  ToSchema` (JSON Schema sinh từ cùng kiểu, kiểm như `ValidJson`; không cho kiểu lồng vì
  client MCP không đọc `$ref`), permission (`requires`), gợi ý `read_only` /
  `destructive`. `Modules::tools()` kiểm khai báo lúc khởi động (tên, trùng, permission
  có người khai). `PageInput` cho tool liệt kê, cùng ý nghĩa `limit`/`cursor`/`order`.
- `tools/list` chỉ có tool người gọi có permission; gọi tool khác nhận cùng lỗi như tool
  không tồn tại. Kết quả: `structuredContent = {"data": …}` (cùng envelope với REST);
  lỗi của tool là kết quả `isError` với `code`, `message` dịch theo `Accept-Language`
  và `errors` như Problem.
- Tool chỉ gọi service, cùng permission với endpoint REST tương ứng; service ghi event
  với `Actor` của người gọi nên lệnh ghi qua MCP vào audit log như REST. Tool của core
  (module `auth`): `whoami`, `list_permissions`, `list_users`, `update_user`.
- `rustls` cần provider: kernel `tls::install_crypto_provider()` (D-066) cũng dùng cho
  client MCP trong test.

Lý do: SDK chính thức theo kịp đặc tả (phiên bản giao thức, metadata, lỗi); stateless
hợp với triển khai nhiều máy chủ; một đường xác thực và một service layer cho cả REST
lẫn MCP thì không có lỗ hổng riêng của MCP.

Đã cân nhắc: tự viết JSON-RPC — ít thư viện hơn nhưng phải tự theo kịp đặc tả đang đổi
nhanh; macro `#[tool]` của `rmcp` — gắn module vào `rmcp` và schemars, trong khi kiểu
đầu vào đã có `ToSchema` + `Validate` như REST; giữ phiên (session) — cần lưu phiên
chung giữa các máy chủ mà không thêm được gì cho tool hiện có.

## D-069 · Module mẫu `notes` không đăng ký sẵn trong binary — 2026-11-11 — agent quyết

Quyết định: module mẫu của bước 10 là `modules/notes` (ghi chú cá nhân: mỗi người
chỉ thấy ghi chú của mình, tên không trùng trong ghi chú của một người). Nó dùng đủ hợp
đồng module: migration (index unique theo chủ + tên), permission `notes.read` /
`notes.write`, service, REST + OpenAPI, event (`notes.note_created` / `_updated` /
`_deleted`, vào audit log), tool MCP (`list_notes`, `get_note`, `create_note`,
`update_note`, `delete_note`), bản dịch, và trait `api::Notes` cho module khác. Module
**không** được đăng ký trong `crates/app/src/modules.rs`: test của nó tự dựng module
như binary dựng (migration → start → REST + MCP qua lớp HTTP của kernel, client MCP
thật) trên cả hai database, và kiểm tài liệu OpenAPI bằng `http::lint`. Hướng dẫn viết
module: `docs/MODULE_GUIDE.md` + `docs/MODULE_GUIDE.vi.md`.

Lý do: core trung lập với nghiệp vụ và mọi bản cài không nên có bảng `notes_…` không
ai dùng; module đầu tiên do chủ dự án chọn. Ghi chú là ví dụ không thuộc ngành nào, đã
được dùng làm ví dụ xuyên suốt tài liệu và doc test của kernel.

Đã cân nhắc: đăng ký sẵn và bật/tắt bằng cấu hình — thêm một cơ chế chỉ để phục vụ
module mẫu; để module mẫu trong thư mục `examples/` — khi đó gate không kiểm ranh giới
module của nó như module thật.

## D-070 · Chép dữ liệu giữa SQLite và Postgres bằng `autoxide copy` — 2026-11-11 — agent quyết

Quyết định:

- `kernel::transfer::copy_modules(nguồn, đích, module)` chép mọi bảng `<module>_…` (trừ
  `<module>_migrations`) theo thứ tự tạo bảng ở đích (`sqlite_master.rowid`,
  `pg_class.oid`) — tức thứ tự migration, nên foreign key trong module luôn thoả. Kiểu
  của từng cột lấy từ khai báo của mỗi database (`pragma_table_info` với tên kiểu
  sea-query đặt như `uuid_text`; `information_schema.columns`), đọc theo kiểu ở nguồn, ghi
  theo kiểu ở đích (số nguyên SQLite luôn đọc 64 bit, đổi có kiểm sang `integer` 4 byte
  của Postgres). Đọc theo luồng, ghi 100 dòng mỗi câu, tất cả trong MỘT transaction ở
  đích; xong thì đếm lại từng bảng.
- Dữ liệu migration gieo sẵn ở đích (role `admin`) bị xoá trong cùng transaction rồi
  thay bằng dữ liệu nguồn. Vì vậy lệnh `autoxide copy --to <URL>` (hay `--to-stdin`,
  để mật khẩu không hiện trong danh sách tiến trình) chỉ nhận database đích chưa từng có
  migration nào, đòi nguồn đã áp dụng đủ migration, rồi áp dụng migration cho đích.
- Test: kernel chép SQLite → database của gate → SQLite với đủ kiểu cột, NULL, chữ
  Unicode và 250 dòng (vượt một lô); binary chép dữ liệu thật của các module.

Lý do: không cần công cụ ngoài (pgloader…) hay định dạng trung gian; dùng đúng
migration của chương trình nên schema hai bên chắc chắn khớp.

Đã cân nhắc: xuất / nhập JSON — thêm một định dạng phải giữ ổn định; tắt kiểm foreign
key khi chép — Postgres cần quyền superuser.

## D-071 · Bộ đo hiệu năng tự viết, qua toàn bộ request — 2026-11-11 — agent quyết

Quyết định: bộ đo là bench target `harness = false` của module mẫu
(`modules/notes/benches/core.rs`, chạy `cargo bench -p autoxide-notes`), đi qua toàn bộ
lớp HTTP của kernel tới database bằng `tower::oneshot` (không qua socket, để đo code và
database chứ không đo mạng). Đo write, read, list 50 dòng và write từ 16 task; in số
việc / giây và p50 / p95 / p99. Database chọn bằng `TEST_DATABASE_URL` như test. Số gốc
và cách đọc ghi ở `docs/BENCHMARKS.md`. Gate biên dịch bộ đo (`clippy --all-targets`)
nhưng không chạy nó (thời gian và số đo phụ thuộc máy).

Lý do: đo đúng thứ người dùng gặp (một request), không thêm thư viện (`criterion` kéo
nhiều phụ thuộc mà ở đây chỉ cần đếm thời gian và lấy phân vị — vài chục dòng).

Đã cân nhắc: `criterion` — phân tích thống kê tốt hơn nhưng nặng và đo từng hàm nhỏ;
đo qua socket thật — cộng thêm thời gian mạng không thuộc core.

## D-072 · Docker: build multi-stage, chạy Debian slim không root; compose có bước migrate — 2026-11-11 — agent quyết

Quyết định:

- `Dockerfile`: stage build từ `rust:<phiên bản ghim>-slim-trixie` (đặt
  `RUSTUP_TOOLCHAIN` để khỏi tải thành phần chỉ dành cho máy dev), cache registry và
  `target` bằng BuildKit; stage chạy `debian:trixie-slim` + `ca-certificates`, người dùng
  `autoxide` (uid 10001), thư mục làm việc `/var/lib/autoxide`, mọi dữ liệu ghi đĩa trong
  volume `/var/lib/autoxide/data` (SQLite, file tải lên, mail ghi ra file). Mặc định
  nghe `0.0.0.0:8080`, log JSON, lệnh `serve`. Image ~190 MB.
- `compose.yml`: `postgres:16-alpine` (phiên bản tối thiểu được hỗ trợ) có healthcheck;
  service `migrate` chạy `autoxide migrate up` một lần; `app` chỉ khởi động khi migrate
  xong, chỉ mở cổng trên `127.0.0.1` (reverse proxy TLS đặt phía trước). Secret
  (`POSTGRES_PASSWORD`, `AUTOXIDE_AUTH_TOKEN_SECRET`) đọc từ `.env` không commit, thiếu thì
  compose từ chối chạy.
- Đã thử thật: build image, `compose up`, tạo admin bằng `docker compose run app user
  create`, đăng nhập, tải file vào volume.

Lý do: migration không bao giờ tự chạy lúc `serve` (D-036), nên compose cần một
bước riêng; Debian slim thay vì distroless/alpine để có glibc (SQLite đóng gói, ring) và
công cụ gỡ lỗi tối thiểu.

Đã cân nhắc: image `scratch`/distroless — nhỏ hơn nhưng khó gỡ lỗi và cần build musl;
chạy migration trong lệnh khởi động của app — nhiều bản app cùng lúc sẽ tranh nhau
migrate.

## D-073 · Backup là một thư mục trung lập: SQLite + file — 2026-11-11 — agent quyết

Quyết định:

- `autoxide backup <thư mục>` tạo `backup.json` (định dạng 1, phiên bản, lúc tạo, số
  dòng mỗi bảng, số file), `database.sqlite` (dữ liệu mọi module, chép bằng
  `transfer::copy_modules` — D-070) và `files/<khoá>` (mọi file từ nơi lưu, qua
  `Storage::keys` + `get`). Thư mục phải mới hoặc trống.
- Database nguồn được đọc trong `Database::begin_snapshot()` (Postgres: `REPEATABLE READ
  READ ONLY`; SQLite/WAL: transaction đọc) nên nhất quán dù máy chủ đang chạy. File chép
  SAU database: file tải lên trong lúc backup có thể có mặt mà không có dòng (vô hại);
  file bị xoá trong lúc backup thì thiếu — muốn trọn vẹn tuyệt đối thì dừng máy chủ.
- `autoxide restore <thư mục>` chỉ nhận database chưa có migration nào, áp dụng
  migration, áp dụng migration lên một BẢN SAO của `database.sqlite` (backup cũ hơn vẫn
  khôi phục được, bản gốc không bị đổi), chép vào, rồi đưa file vào nơi lưu đang cấu
  hình; kiểm số file khớp `backup.json`.
- Một định dạng cho mọi cách cài: backup từ Postgres khôi phục vào SQLite được, từ ổ đĩa
  sang S3 được, và ngược lại.

Lý do: không phụ thuộc công cụ ngoài (`pg_dump`, `sqlite3`, `aws s3 sync`) có phiên bản
khác nhau trên mỗi máy; người lạ chỉ cần một lệnh. Dùng lại đúng cơ chế chép đã kiểm
kiểu cột trên cả hai database.

Đã cân nhắc: `pg_dump` / sao chép file SQLite — hai cách khác nhau cho hai database và
không mang file theo; nén thành một file `.tar.gz` — người dùng tự nén được, thêm thư
viện chỉ để nén thì không đáng.

## D-074 · CI trên GitHub Actions: gate đầy đủ trên PostgreSQL 16 và 18, quét secret bằng gitleaks — 2026-11-11 — agent quyết

Quyết định: `.github/workflows/ci.yml` chạy khi push lên `main`, khi có pull request và
khi bấm tay.

- Job `gate`: ma trận PostgreSQL `16` (thấp nhất được hỗ trợ) và `18` (mới nhất lúc
  viết), mỗi bản là một service container; thêm service `motoserver/moto` cho test lưu
  file trên S3. Chạy đúng `./gate.sh` với `GATE_POSTGRES_URL` / `GATE_S3_ENDPOINT` — CI và
  máy dev cùng một định nghĩa "xong". Toolchain theo `rust-toolchain.toml`, cache bằng
  `Swatinem/rust-cache`, `cargo-deny` cài bằng `taiki-e/install-action`.
- Job `secrets`: gitleaks (image `ghcr.io/gitleaks/gitleaks`, phiên bản ghim) quét TOÀN
  BỘ lịch sử git với `.gitleaks.toml`: bộ luật mặc định cộng một ngoại lệ hẹp cho chủ thể
  event mẫu `<module>.<loại>:<UUID>` (bị luật `generic-api-key` báo nhầm). Chạy image
  thay vì `gitleaks-action` vì action đó đòi license cho tài khoản tổ chức.
- Workflow chỉ có quyền đọc (`permissions: contents: read`); đã kiểm bằng actionlint.

Lý do: gate là định nghĩa "xong"; CI chạy lại đúng nó trên hai phiên bản PostgreSQL để
bắt chỗ khác nhau giữa các phiên bản. Secret không bao giờ vào git — quét lịch sử chặn
cả secret đã commit rồi xoá.

Đã cân nhắc: chỉ PostgreSQL mới nhất — bỏ sót lỗi trên bản tối thiểu được hỗ trợ;
`gitleaks-action` — cần license cho tổ chức.

## D-075 · Thử gỡ ngoại lệ RUSTSEC-2026-0459: chưa gỡ được — 2026-11-11 — agent quyết

Quyết định: giữ ngoại lệ của D-032. Đã thử ngày 11/11/2026: `sqlx-sqlite` mới nhất vẫn
là 0.9.0 và vẫn đòi `libsqlite3-sys >= 0.30.1, < 0.38`; bản vá cần ≥ 0.39 (mới nhất hiện
có 0.40). `cargo update -p libsqlite3-sys` không đổi gì. Cơ chế tự hết hạn của D-032 vẫn
còn: gate chạy `cargo deny check -D advisory-not-detected`.

Điều kiện an toàn của D-032 với các lệnh mới của bước 11:

- `autoxide copy`: nguồn là database của chính máy chủ; đích là database mới do lệnh
  tạo.
- `autoxide backup`: tạo file SQLite mới, đóng hẳn trước khi xong (không để lại `-wal`).
- `autoxide restore`: chỉ chép đúng file `database.sqlite` của bản backup sang thư mục
  tạm rồi mở bản sao — không bao giờ mở file `-wal` / `-shm` nằm cạnh, là đường khai thác
  của lỗ hổng. README dặn chỉ khôi phục bản backup do chính mình tạo.

Lần thử kế: khi nâng `sea-orm` hay `sqlx`, hoặc khi gate báo ngoại lệ không còn cần.

## D-076 · Đo tải qua API thật; bật TCP_NODELAY; một câu query lấy quyền — 2026-11-11 — agent quyết

Quyết định:

- `scripts/loadtest.sh` đo hiệu năng và chịu tải qua API thật: binary release,
  `autoxide serve`, TCP, đăng nhập thật, bằng `oha` (công cụ ngoài, cài bằng
  `cargo install oha`, không phải phụ thuộc của dự án). Chạy được trên SQLite và trên
  một database PostgreSQL trống (`LOADTEST_DATABASE_URL`). Kết quả ghi ở
  `docs/BENCHMARKS.md`. Gate không chạy nó (tốn phút và số đo phụ thuộc máy).
- Kernel bật `TCP_NODELAY` trên mọi kết nối (`ListenerExt::tap_io` của axum): đo thấy
  câu trả lời gửi thành nhiều mảnh (tải file, MCP) chờ ~40 ms vì Nagle + delayed ACK.
- `RoleService::principal` (chạy ở MỌI request đã đăng nhập) lấy quyền bằng một câu
  JOIN `auth_user_role` – `auth_role` – `auth_role_permission` thay vì hai câu; trên
  PostgreSQL mỗi câu là một vòng mạng. Join chỉ trong bảng của chính module `auth`.
- Lớp log của kernel bỏ đúng thông điệp
  `rmcp::service: failed to send pending response during drain`: thư viện ghi nó ở mức
  ERROR khi client ngắt kết nối giữa lời gọi MCP — việc bình thường. Đã kiểm: chạy trọn
  vẹn 20.000 lời gọi thì không có dòng này; chỉ có khi client bỏ đi.

Lý do: đo trước rồi mới tối ưu (CLAUDE.md, tiêu chí "Nhanh"/"Tối ưu"); cả ba chỗ đều
tìm ra từ số đo, sửa không đổi hành vi (test cũ vẫn xanh trên hai database).

Đã cân nhắc: cache quyền trong bộ nhớ — nhanh hơn nữa nhưng quyền đổi (gỡ role, khoá tài
khoản) phải có hiệu lực ngay trên mọi máy chủ; một câu join đã đủ.

## D-077 · Passkey (WebAuthn) bằng `webauthn_rp`; hai ngoại lệ `cargo deny` — 2026-11-11 — chủ dự án mở, agent quyết cách làm

Chủ dự án mở mục "Passkey" của "Sau core" (bước 12 của lộ trình). Cách làm:

- Thư viện `webauthn_rp` 0.3 (MIT OR Apache-2.0): Relying Party theo W3C WebAuthn Level 3,
  thuần Rust trên RustCrypto, không OpenSSL (D-043). `webauthn-rs` phổ biến hơn nhưng cần
  OpenSSL. Feature `serde_relaxed` (nhận JSON `credential.toJSON()` của mọi trình duyệt)
  và `serializable_server_state` (trạng thái nghi thức lưu được vào database).
- Mọi kiểu của thư viện ở trong `modules/auth/src/passkey/webauthn.rs`; phần còn lại chỉ
  thấy byte và JSON.
- Bảng `auth_passkey` (credential ID duy nhất, user handle, khoá công khai và trạng thái
  động do thư viện mã hoá; không secret) và `auth_passkey_ceremony` (trạng thái nghi thức,
  hạn 5 phút, job dọn dẹp xoá). Mỗi nghi thức dùng một lần nhờ `DELETE` có điều kiện; nghi
  thức đăng ký gắn với người bắt đầu. Máy chủ nào hoàn tất cũng được.
- User handle: 64 byte ngẫu nhiên (khuyến nghị của đặc tả), chung cho mọi passkey của một
  người; tối đa 20 passkey mỗi người; trình duyệt nhận danh sách passkey đã có để không tạo
  trùng.
- Đăng nhập là discoverable (không cần email); passkey đã xác minh người dùng (UV bắt buộc)
  nên không hỏi thêm TOTP; cấp cùng cặp token như đăng nhập mật khẩu (event `signed_in`,
  method `passkey`). Mọi lỗi đăng nhập cùng một mã `auth.passkey_sign_in_failed` (không lộ
  passkey có tồn tại không; lý do ghi log mức info).
- Thêm / xoá passkey chỉ trong phiên đăng nhập bằng mật khẩu (không qua API key), như cài
  đặt MFA. Event `auth.passkey_added` / `auth.passkey_removed`.
- Cấu hình `AUTOXIDE_AUTH_PASSKEY_RP_ID` (không đặt: tính năng tắt, 403
  `auth.passkeys_disabled`) và `AUTOXIDE_AUTH_PASSKEY_ORIGINS` (mặc định
  `https://<rp id>`; mỗi origin phải thuộc RP ID — kiểm lúc khởi động; `http://` chỉ cho
  localhost).
- Test: authenticator phần mềm (`passkey/testing.rs`: khoá P-256 bằng `p256`, CBOR theo
  CTAP2, chữ ký theo đặc tả) — đăng ký, đăng nhập, gửi lại, sai origin (chống phishing),
  credential lạ, user handle sai, xoá, tài khoản khoá, tắt tính năng, và 5 request đồng
  thời trên một nghi thức (đúng một thành công) — trên cả hai database; cộng luồng REST.

Ngoại lệ `cargo deny` (theo điều khoản ngoại lệ của luật gate): RUSTSEC-2023-0071 (`rsa`,
Marvin) và RUSTSEC-2026-0387 (`crypto-bigint` < 0.7.6). Cả hai là rò rỉ thời gian khi xử
lý dữ liệu BÍ MẬT (khoá riêng); chỉ đến từ `webauthn_rp`, nơi chúng chỉ kiểm chữ ký bằng
khoá CÔNG KHAI — không có bí mật để rò. Điều kiện: không dùng `rsa`, `p256`, `p384` cho
khoá riêng ở đâu khác. Tự hết hạn: gate chạy `-D advisory-not-detected`.

Đã cân nhắc: tự viết phần kiểm WebAuthn trên `ring` — vài trăm dòng mã bảo mật, trái
tiêu chí "theo chuẩn ngành thay vì tự chế"; lưu nghi thức trong bộ nhớ — hỏng khi chạy
nhiều máy chủ; quản trị viên xoá passkey của người khác — để sau, khi có nhu cầu (tài
khoản bị khoá thì passkey cũng không đăng nhập được).

## D-078 · Phát hành: SemVer một số cho cả workspace, CHANGELOG, image trên GHCR, Dependabot — 2026-11-11 — agent quyết

Quyết định:

- Một số phiên bản cho cả workspace (`[workspace.package] version`), theo Semantic
  Versioning; `0.x` cho tới khi chủ dự án tuyên bố ổn định. API HTTP `/api/v1` không phá
  vỡ trong mọi trường hợp (deprecation, D-059); phiên bản nói về cấu hình, CLI, hợp đồng
  module.
- `CHANGELOG.md` (tiếng Anh, chuẩn Keep a Changelog): mỗi thay đổi người dùng thấy thêm
  một dòng vào `[Unreleased]`. Quy trình phát hành ở `docs/RELEASING.md`.
- Workflow `Release` chạy khi đẩy tag `vX.Y.Z`: kiểm tag khớp `Cargo.toml` và có mục
  trong CHANGELOG, chạy lại gate, build image bằng `Dockerfile` và đăng
  `ghcr.io/<owner>/autoxide:X.Y.Z` + `:X.Y` lên GitHub Container Registry, tạo bản phát hành
  GitHub với ghi chú từ CHANGELOG. Chỉ `linux/amd64`: build arm64 qua giả lập mất hàng giờ;
  thêm khi có người cần (runner arm64 thật).
- `compose.yml` nhận `AUTOXIDE_IMAGE` để dùng image đã phát hành thay vì tự build.
- Dependabot mỗi tuần cho thư viện Rust (gom minor/patch thành một PR), GitHub Actions và
  image Docker; workflow `Security audit` chạy `cargo deny check advisories` mỗi ngày.

Lý do: người lạ cài được mà không cần toolchain Rust; nâng cấp có nhịp đều, mỗi đề xuất
qua đúng gate; lỗ hổng mới công bố được biết ngay cả khi repo không có commit.

Đã cân nhắc: `cargo-release` / `release-plz` — tự động hơn nhưng thêm công cụ và quy ước
cho một việc làm vài lần mỗi năm; phát hành binary cho từng hệ điều hành — Docker và
`cargo build` đã đủ cho máy chủ.

## D-079 · Rà soát bảo mật bước 13: đếm lượt trước khi kiểm, xác nhận lại mật khẩu, không tác động lên người quyền cao hơn — 2026-11-12 — agent quyết

Hai agent đọc độc lập toàn repo (kernel/CLI và module `auth`); mọi lỗi tìm thấy đã sửa:

- **Login throttling đếm trước rồi mới kiểm** (`LoginThrottle::reserve`): nhiều request bắn
  cùng lúc không vượt được `max_failures`. Mật khẩu đúng mà còn chờ mã MFA thì trả lại lượt
  (`refund`), đúng hẳn thì xoá (`clear`). Mã MFA của một lần đăng nhập được claim bằng một
  câu `UPDATE` có điều kiện. Tắt TOTP và tạo lại recovery code cũng đếm mã sai, theo
  người dùng (`mfa:<user_id>`).
- **Refresh token** khoá dòng người dùng trước (`UPDATE … WHERE disabled_at IS NULL`) trong
  cùng transaction: refresh không lọt qua lúc tài khoản đang bị khoá.
- **Xác nhận lại mật khẩu** khi tạo API key và khi bắt đầu thêm passkey; đặt lại mật khẩu
  qua email xoá mọi passkey và API key — kẻ chiếm phiên không cài được cửa sau.
- **Người có `auth.users.manage` không tác động lên người có permission mà mình không có**
  (gỡ MFA, khoá, đặt mật khẩu đầu): `RoleService::require_covers`, lỗi
  `auth.target_has_more_permissions`.
- **Idempotency** xác thực trước: request chưa đăng nhập không được lưu; key thuộc phạm vi
  (người dùng, API key); dấu vân tay gồm method, đường dẫn kèm query, content-type và thân;
  đọc thân có giới hạn thời gian `request_timeout`.
- **Rate limiting** chỉ đọc giá trị cuối của header IP (giá trị proxy tin cậy thêm vào), nhận
  cả dạng `ip:port`.
- **Job xong thì xoá payload** (ghi `null`): link chứa token không nằm lại trong bảng job.
  Job trong dead-letter queue giữ payload để chạy lại được; token trong đó đã có hạn.
- **`autoxide restore`** từ chối bản backup chứa symbolic link.

Lý do: mỗi điểm là một cách chiếm hoặc giữ tài khoản, hay vượt giới hạn đã hứa trong tài
liệu. Sửa đều chạy trên cả hai database và có test (kể cả test đồng thời).

Đã cân nhắc: khoá `SELECT … FOR UPDATE` cho throttle — SQLite không có; mã hoá payload job
thay vì xoá — thêm key cần quản lý mà không ai cần đọc lại payload của job đã xong.

## D-080 · Tài khoản: quản lý phiên, xoá tài khoản + xuất dữ liệu xuyên module, tìm người dùng — 2026-11-12 — agent quyết

Quyết định:

- **Phiên = một họ refresh token.** Refresh token mang thêm `user_agent` và
  `session_started_at` (m0015), chép sang token mới mỗi lần refresh. Response token có
  `session_id` (id của họ) để client nhận ra phiên của mình. Người dùng xem
  (`GET /me/sessions`), đăng xuất một phiên (`DELETE /me/sessions/{id}`) hoặc mọi phiên trừ
  một (`DELETE /me/sessions?keep=…`); quản trị viên đăng xuất mọi phiên của một người
  (`DELETE /users/{id}/sessions`). Access token đã cấp vẫn chạy tới hết hạn (15 phút) —
  muốn chặn ngay thì khoá tài khoản.
- **Xoá tài khoản là xoá hẳn** (`DELETE /me` kèm mật khẩu, `DELETE /users/{id}`): dữ liệu
  của `auth` đi theo foreign key trong module; module khác nhận event
  `auth.user_deleted` (tên là hằng `privacy::USER_DELETED` của kernel, payload chỉ có id)
  và tự xoá phần của mình. Module dọn dữ liệu đăng ký nhận event này được kể cả khi không
  cài module tài khoản (dispatcher coi nó là hợp đồng của kernel). Audit log giữ event
  (chỉ id, không email). Không xoá được admin cuối cùng.
- **Xuất dữ liệu cá nhân xuyên module:** kernel có trait `privacy::PersonalData`; module
  đăng ký bằng `StartContext::personal_data`, `auth` gom lại ở `GET /me/export`
  (`{"data": {"<module>": …}}`). Sổ đăng ký dùng chung nên module khởi động sau `auth` vẫn
  có mặt. Không xuất secret.
- **Tìm người dùng** bằng `search` ở `GET /users` (và tool MCP `list_users`): chứa chuỗi,
  không phân biệt hoa thường. Cột `auth_user.search_text` (m0016) giữ email + tên viết
  thường **tính trong Rust**: `LOWER` của SQLite chỉ đổi chữ ASCII, của Postgres đổi mọi
  chữ — dùng `LOWER` trong SQL sẽ cho hai kết quả khác nhau (vd tên tiếng Việt). `LIKE` có
  `ESCAPE` để `%`/`_` người dùng gõ là chữ thường. Tìm "chứa" quét bảng người dùng — chấp
  nhận được với kích thước bảng này; cần nhanh hơn thì thêm full-text sau, khi đo thấy chậm.

Lý do: ba việc của bước 13 nhóm 2; chạy như nhau trên hai database, không module nào
phụ thuộc module `auth` (D-046).

Đã cân nhắc: xoá mềm (đánh dấu) — giữ dữ liệu cá nhân trái mục đích của việc xoá; job
xuất dữ liệu ra file — thêm hạ tầng lưu file tạm cho một việc hiếm, JSON trả thẳng là đủ
ở cỡ dữ liệu của core; ghi `sid` vào access token để đánh dấu phiên hiện tại — client đã
có `session_id` từ response token.

## D-081 · Webhook: module `webhooks`, chuẩn Standard Webhooks, gửi qua job queue — 2026-11-12 — agent quyết

Quyết định:

- Module riêng `webhooks` (đăng ký trong binary), nhận mọi event bằng đăng ký `*` như
  audit. Quản trị viên (permission `webhooks.manage`) khai endpoint: địa chỉ `https://`
  (`http://` chỉ khi `AUTOXIDE_WEBHOOKS_ALLOW_HTTP=true`), danh sách tên event (kiểm theo
  danh mục event) hoặc `*`.
- Chữ ký theo **Standard Webhooks** (header `webhook-id`, `webhook-timestamp`,
  `webhook-signature: v1,<HMAC-SHA256 base64>`, secret `whsec_…`) — bên nhận dùng thư viện
  có sẵn của chuẩn. Test khớp ví dụ chính thức của chuẩn. Secret lưu nguyên (cần để ký),
  chỉ hiện một lần lúc tạo hoặc thay.
- Mỗi (event, endpoint) một dòng `webhooks_delivery` (unique — event giao lại không gửi
  trùng) và một job, cùng transaction. Job thử tối đa 12 lần với backoff của job queue
  (khoảng hai giờ); `webhook-id` giữ nguyên qua các lần thử để bên nhận bỏ bản trùng.
  Không theo chuyển hướng; thời gian chờ `AUTOXIDE_WEBHOOKS_TIMEOUT_SECONDS` (10 giây).
- Lịch sử gửi xem ở `GET /webhooks/{id}/deliveries`, giữ 30 ngày (job dọn hằng ngày); số
  đo `webhooks_delivered_total`, `webhooks_failed_total`.

Lý do: tích hợp với hệ thống khác mà không phải sửa core; chuẩn ngành thay vì tự chế;
dùng lại job queue (retry, dead-letter queue) đã có trên cả hai database.

Đã cân nhắc: webhook theo từng người dùng — cần lọc event theo quyền xem dữ liệu của từng
người, để sau khi có nhu cầu; chặn địa chỉ mạng nội bộ (SSRF) — chỉ quản trị viên tạo được
endpoint và không theo chuyển hướng; thêm khi mở cho người dùng thường.

## D-082 · Gate kiểm tài liệu OpenAPI hợp lệ và mô tả gửi ra ngoài là tiếng Anh — 2026-11-12 — agent quyết

Bối cảnh: chủ dự án kiểm `docs/openapi.json` bằng trình kiểm chuẩn và thấy file không hợp
lệ — tham số `order` của mọi endpoint danh sách trỏ tới `#/components/schemas/Order` không
có trong file (trình kiểm chuẩn và công cụ sinh SDK từ chối cả file). Tool MCP mang mô tả
tiếng Việt (`///` trên `NoInput`, `PageInput`). Gate cũ chỉ kiểm "có mô tả hay không".

Quyết định:

- `Order` khai `inline` trong `PageQuery`.
- `http::lint` (chạy khi sinh `docs/openapi.json` và trong test của gate) kiểm thêm: mọi
  `$ref` trỏ tới một mục có thật; không `summary` / `description` / `title` nào có chữ tiếng
  Việt. Kiểm tiếng Việt cũng chạy cho mô tả và schema đầu vào của mọi tool MCP lúc khởi
  động.
- Đã xác nhận bằng `openapi-spec-validator`: file cũ bị từ chối, file mới hợp lệ.

Lý do: OpenAPI là hợp đồng để dùng Autoxide từ ngôn ngữ khác; một `$ref` treo làm hỏng cả
hợp đồng. Kiểm trong Rust, không thêm công cụ ngoài vào gate (người lạ không cần Python
hay Node để chạy gate).

Đã cân nhắc: chạy trình kiểm chuẩn đầy đủ (Python / Node) trong gate — thêm phụ thuộc
ngoài cho mọi người phát triển; kiểm theo JSON Schema chính thức của OpenAPI 3.1 bằng thư
viện Rust — thêm thư viện lớn trong khi lỗi thực tế gặp là `$ref` treo, đã kiểm trực tiếp.

## D-083 · Bước 14: kiểm đầu vào ở service và báo mọi lỗi một lần; `Created`; header chung trong OpenAPI; MCP cùng envelope với REST — 2026-11-12 — agent quyết

Quyết định:

- **Mỗi trường được kiểm ở đúng một chỗ.** Trường service kiểm thì không gắn `#[validate]`
  trên kiểu nhận JSON; service gom mọi lỗi của đầu vào (kể cả "thiếu mật khẩu hiện tại" —
  `FieldError::blank`) thành một `Error::validation`. Trước đây `ValidJson` dừng ở lỗi độ
  dài nên lỗi định dạng email chỉ hiện ở lần gửi sau. `#[validate]` chỉ còn ở trường không
  service nào kiểm (token, mã đăng nhập…). Lợi thêm: REST và MCP kiểm như nhau. Webhook báo
  địa chỉ sai và tên event lạ bằng lỗi theo trường (`url`, `events[i]`) như role và API key,
  thay cho mã riêng `webhooks.insecure_url` / `webhooks.unknown_event` (chưa phát hành).
- **`response::Created`**: mọi endpoint tạo mới trả 201 + `Location` (đường dẫn tới bản ghi);
  idempotency đã giữ header này khi trả lại. OpenAPI tự khai `Location` cho mọi response 201.
- **OpenAPI khai header chung trên từng operation**: `Accept-Language` ở mọi nơi;
  `Idempotency-Key` ở lệnh ghi cần đăng nhập khi có nơi lưu key. Có mục `servers` (`/`, địa
  chỉ tương đối — bản trong repo không biết tên miền người triển khai).
- **MCP**: kết quả dạng trang giữ nguyên `{"data", "pagination"}` như REST, giá trị khác bọc
  `{"data": …}`. Thêm tool cho role, API key (auth), audit log và file (nội dung base64, tới
  1 MiB; file lớn dùng REST — mã `files.too_large_for_tool`).

Lý do: các điểm chủ dự án tìm thấy khi kiểm hợp đồng API (bước 14); giữ "học một endpoint là
biết mọi endpoint" cho cả người lẫn agent.

Đã cân nhắc: để `ValidJson` gom lỗi rồi cho handler chạy tiếp — handler phải xử lý dữ liệu
chưa hợp lệ, dễ sai; gắn `#[validate(custom)]` gọi lại luật của service — luật cần ngữ cảnh
(danh mục permission, cấu hình) không chạy được ở đó, vẫn thành hai đợt.

## D-084 · Chống ghi đè: `ETag` từ `updated_at`, `If-Match` kiểm trong câu `UPDATE` — 2026-11-12 — agent quyết

Quyết định:

- Phiên bản của bản ghi là cột `updated_at` sẵn có (cắt tới micro giây, hai database lưu như
  nhau); `ETag` = `"<số micro giây>"`. Không thêm cột `version`, không migration.
- `GET` một bản ghi sửa được và lệnh sửa của nó trả `ETag` (`version::Tagged`). Lệnh sửa nhận
  `If-Match` (`version::IfMatch`): service thêm `updated_at = <phiên bản>` vào chính câu
  `UPDATE`, không dòng nào khớp thì 412 `precondition_failed` (`ErrorKind::PreconditionFailed`).
  Trên Postgres, câu `UPDATE` chờ khoá dòng rồi kiểm lại điều kiện, nên hai người cùng sửa thì
  chỉ một người thắng — có test đồng thời trên cả hai database.
- Không gửi `If-Match` thì sửa như cũ (client đã có không hỏng). `*` coi như không kiểm; `ETag`
  yếu (`W/…`), sai dạng hay nhiều giá trị thì không bao giờ khớp.
- Áp cho người dùng (`PATCH /auth/users/{id}`), role (`PUT /auth/roles/{id}`), ghi chú và
  endpoint webhook. Tool MCP sửa không kiểm phiên bản (như REST không gửi `If-Match`).

Lý do: mục cuối của bước 14; theo chuẩn HTTP (RFC 9110) thay vì tự chế cơ chế khoá.

Đã cân nhắc: cột `version` số nguyên tăng dần — chắc hơn khi hai lần sửa rơi vào cùng một micro
giây, nhưng phải thêm migration cho mọi bảng; khả năng đó gần như không xảy ra với một bản ghi.
Bắt buộc `If-Match` (428) — phá client đang có.
