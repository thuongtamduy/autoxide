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

## D-085 · Nền chung cho danh sách sắp theo cột, endpoint công khai và nội dung tải về — 2026-11-12 — agent quyết

Bối cảnh: chủ dự án giao module thư viện media (`media`). Lời giao nêu "bước 13: nền chung"
(lọc / sắp xếp danh sách, tìm theo chữ, endpoint công khai, `ETag` / `If-None-Match`, `If-Match`
→ 412) làm điều kiện trước, và gọi việc là "bước 14" — nhưng trong lộ trình của repo bước 13 và
14 đã là việc khác, đã xong. Ghi việc mới thành bước 15 của lộ trình. `If-Match` → 412 đã có
(D-084); phần còn thiếu thêm vào kernel, trung lập với nghiệp vụ, có mặc định (D-038):

- **Sắp theo cột khác ID** (`pagination::fetch_page_by`): keyset pagination trên `(cột, id)`.
  Con trỏ là base64url của JSON `{c: tên cột, v: giá trị, id}` — client vẫn coi là chuỗi mờ;
  `PageRequest` nhận cả con trỏ cũ (UUID) lẫn con trỏ mới. Con trỏ của cách sắp này dùng cho
  cách sắp khác (hay cho `fetch_page`) thì 422 trên `cursor`, không đoán. Endpoint tự khai tham
  số `sort` và bộ lọc của mình. Sắp theo thời gian tạo vẫn là sắp theo ID (UUIDv7).
- **Tìm theo chữ** (`search::normalize`, `search::contains`): cột "khoá tìm" viết thường bằng
  Rust (không dùng `LOWER()` — SQLite chỉ biết ASCII), lọc `LIKE '%…%' ESCAPE '\'`. `auth` chuyển
  sang dùng chung hàm này (cùng kết quả). Cột chữ dùng để sắp thì so sánh theo byte trên cả hai
  database (Postgres: `COLLATE "C"` khai trong migration của module) để thứ tự như nhau.
- **Endpoint công khai**: handler nhận `Option<Principal>`. Không gửi `Authorization` → `None`;
  gửi mà sai → vẫn 401 (không lặng lẽ coi như khách). OpenAPI khai `security((), ("bearer" = []))`.
- **`If-None-Match` → 304** cho mọi `GET` / `HEAD` trả 200 có `ETag`: một lớp của kernel, handler
  không phải viết gì; so sánh yếu, nhận danh sách và `*` (RFC 9110). Hàm `version::if_none_match`
  cho handler đọc dữ liệu đắt tự kiểm trước.
- **`If-Match` bắt buộc ở endpoint chọn** (`IfMatch::required`): thiếu header → 428
  `precondition_required` (RFC 6585). `IfMatch` có thêm `Missing` để phân biệt với `*`. Endpoint
  cũ vẫn không bắt buộc (D-084).
- **`ErrorKind`** thêm `RangeNotSatisfiable` (416) và `PreconditionRequired` (428).
- **`Storage`** thêm `get_range(key, start..end)` và `keys_with_prefix(prefix)`, đều có cài đặt
  mặc định (đọc từ đầu rồi cắt; lọc từ `keys`). `files` cài lại bằng khả năng đọc theo khoảng và
  liệt kê theo thư mục của `object_store` — có test trên ổ đĩa và S3.
- **`http::content_disposition(Disposition, name)`** (RFC 6266 + 8187) chuyển từ `files` lên
  kernel để module khác dùng chung; `files` gọi nó, kết quả không đổi.
- CORS cho trình duyệt gửi `If-None-Match`, `If-Range`, `Range` và đọc `Content-Disposition`,
  `Content-Range`, `Accept-Ranges`.

Lý do: đây là thứ mọi module có danh sách lớn hay nội dung tải về sẽ cần; làm một lần ở kernel
thì mỗi module chỉ khai, không tự chế lại.

Đã cân nhắc: con trỏ chỉ chứa ID rồi đọc lại giá trị cột của dòng đó — gọn hơn nhưng dòng đó bị
xoá giữa hai trang thì mất chỗ. Bộ lọc chung kiểu `filter[field]=…` ở kernel — mỗi module lọc
khác nhau, khai riêng rõ hơn và OpenAPI mô tả được từng tham số.

## D-086 · Module `media`: thư viện media tách khỏi `files` — 2026-11-12 — chủ dự án giao, agent quyết cách làm

Quyết định:

- **Module riêng** `media` (crate `autoxide-media`), không mở rộng `files`: `files` là chỗ lưu
  file của từng người (chủ file, `files.upload`), còn thư viện media là tài sản chung của một
  website (thư mục, công khai, thùng rác, dùng lại trong nội dung). Hai mô hình quyền khác
  nhau. `media` không đọc, ghi hay tham chiếu bảng của `files`; nội dung đi qua `dyn Storage`
  (đăng ký sau `files`), khoá `media/<item_id>/<version_id>`, không bao giờ đụng khoá khác.
- **Permission**: `media.read` (mọi item kể cả riêng tư, thư mục, thùng rác), `media.write`
  (tải lên, sửa, thay nội dung, chuyển, công khai, thùng rác, thư mục), `media.purge` (xoá
  hẳn — việc không lấy lại được nên tách riêng). Quyền theo thư viện, không theo người tải
  lên.
- **Dữ liệu**: `media_item`, `media_folder`, `media_lock`. Thư mục gốc lưu là UUID toàn số 0
  chứ không `NULL` — unique index `(parent_id, name_key)` của SQLite lẫn Postgres coi hai
  `NULL` là khác nhau, nên hai thư mục gốc cùng tên sẽ lọt; có test tạo đồng thời ở gốc. API
  vẫn trả `null` cho gốc. `name_key` (tên viết thường) để tìm, sắp và chặn trùng không phân
  biệt hoa thường; trên Postgres cột này `COLLATE "C"` để thứ tự giống SQLite.
- **Ghi đồng thời đúng trên cả hai database**: thêm item / thư mục con vào một thư mục thì
  "chạm" dòng thư mục đó trong transaction; xoá thư mục thì xoá dòng trước rồi đếm con — hai
  bên chờ nhau, không còn item mồ côi trong thư mục đã xoá. Chuyển thư mục cập nhật dòng
  `media_lock.folder_tree` trước, nên các lệnh chuyển chạy lần lượt và hai lệnh chéo nhau
  (A vào B, B vào A) không tạo vòng — có test đồng thời.
- **Thay nội dung giữ ID**: nội dung mới ghi dưới `version_id` mới, rồi cập nhật dòng + event
  trong một transaction với điều kiện phiên bản trong câu `UPDATE`, rồi xoá nội dung cũ. Hai
  lệnh thay cùng `If-Match`: đúng một thắng (test đồng thời).
- **`If-Match` bắt buộc** (`IfMatch::required`, 428) cho `PATCH` item / thư mục và
  `PUT …/content`, như lời giao. Tool MCP `media.update` nhận `updated_at` đã đọc thay cho
  `If-Match`.
- **Phục vụ nội dung**: item công khai ngoài thùng rác tải được không cần đăng nhập; còn lại
  cần `media.read`, không có thì 404 (không lộ item có tồn tại). `inline` chỉ cho ảnh raster;
  SVG, HTML, PDF… là `attachment`; luôn `nosniff` và `Content-Security-Policy: default-src
  'none'; sandbox`. `ETag` là SHA-256 nội dung. URL kèm `?v=<version_id>` đúng phiên bản hiện
  tại thì cache một năm `immutable`; không kèm hay phiên bản cũ thì 60 giây (công khai) hoặc
  `no-cache` (riêng tư). Một khoảng `Range` → 206, ngoài nội dung → 416, nhiều khoảng → trả
  cả nội dung (RFC 9110 cho phép).
- **Tài khoản bị xoá**: item KHÔNG bị xoá (của thư viện, không của người tải); `uploaded_by`
  thành `NULL`. Bản xuất dữ liệu cá nhân liệt kê item người đó tải lên.
- **Thùng rác**: giữ `AUTOXIDE_MEDIA_TRASH_DAYS` ngày (mặc định 30) rồi job hằng ngày xoá
  hẳn. Job hằng ngày khác xoá nội dung `media/` không item nào trỏ tới, chỉ khoá cũ hơn 24 giờ
  — tuổi đọc từ `version_id` (UUIDv7), không cần nơi lưu cho biết thời điểm ghi.
- **`api::Media`** cho module khác (sắp tới là module nội dung): `find_many(&ids)` một câu
  query, tối đa 500 ID, bỏ item trong thùng rác, giữ thứ tự hỏi.

Lý do: chủ dự án giao thư viện media làm nền cho website. Mỗi chi tiết chọn cách đúng trên
cả hai database mà không cần tính năng riêng của bên nào (trừ `COLLATE "C"` khai trong
migration, chỉ để Postgres so sánh giống SQLite).

Đã cân nhắc: thêm thư mục, công khai, thùng rác vào `files` — đổi hành vi của module đang
chạy và trộn hai mô hình quyền. Khoá thứ tự bằng `SELECT … FOR UPDATE` — SQLite không có cú
pháp đó; dòng khoá dùng `UPDATE` thường chạy được cả hai. Đường dẫn vật chất hoá (`path`)
cho cây thư mục — vẫn cần khoá khi chuyển, mà đổi tên một thư mục phải viết lại cả nhánh.

## D-087 · Không sinh biến thể ảnh (thumbnail, đổi cỡ, WebP/AVIF) trong core — 2026-11-12 — agent quyết

Quyết định: `media` lưu bản gốc kèm chiều rộng / cao; không tạo ảnh thu nhỏ hay chuyển định
dạng. Website (vd component ảnh của Next.js) hoặc CDN / image proxy đứng trước tự tối ưu,
dùng `url` có phiên bản (cache được lâu) làm nguồn.

Lý do: giải mã và mã hoá ảnh cần thư viện lớn (và thường có mã C cho AVIF), tốn CPU / RAM
trong tiến trình API, mở thêm bề mặt tấn công (ảnh độc hại nhắm vào bộ giải mã), và mỗi
website cần bộ cỡ khác nhau. Các framework web hiện đại đã làm việc này tốt. Khi thật sự cần
trong core, thêm sau như job nền mà không phá API (thêm trường `variants`).

Đã cân nhắc: crate `image` + job queue tạo vài cỡ cố định — làm sau nếu chủ dự án cần.

## D-088 · Thư viện media dùng thư mục lồng nhau, không dùng nhãn — 2026-11-12 — agent quyết

Quyết định: mỗi item nằm trong đúng một thư mục (hay gốc); thư mục lồng nhau không giới hạn
độ sâu (chặn vòng lặp). Không có nhãn (tag) ở bước này.

Lý do: người quản trị website quen tổ chức theo thư mục (như máy tính, như các CMS phổ biến);
một item một chỗ thì quyền xoá thư mục ("chỉ khi trống") và di chuyển rõ ràng. Tìm theo tên,
lọc theo loại bù phần lớn nhu cầu của nhãn. Nhãn thêm sau được (bảng riêng) mà không đổi API
hiện có.

Đã cân nhắc: chỉ nhãn (một item nhiều nhãn) — linh hoạt hơn nhưng khó duyệt như cây thư mục;
cả hai cùng lúc — gấp đôi việc khi chưa ai cần.

## D-089 · Nhận loại thật tự viết; kích thước ảnh bằng `imagesize` — 2026-11-12 — agent quyết

Quyết định:

- **Loại thật** theo chữ ký đầu file (`sniff.rs`, vài chục dòng tự viết): JPEG, PNG, GIF,
  WebP, AVIF / HEIC (hộp `ftyp`), SVG / HTML / XML (đọc phần đầu dạng chữ), MP4 / MOV, WebM /
  Matroska, Ogg, FLAC, MP3, AAC, WAV, AVI, PDF, zip. Loại khai phải khớp loại thật (có bảng tên
  đồng nghĩa và định dạng chứa dùng chung cho hình / tiếng); loại có chữ ký mà nội dung không
  mang chữ ký đó thì từ chối; loại không có chữ ký (vd `text/plain` nếu quản trị viên cho
  phép) thì nhận theo lời khai — vẫn luôn được tải về dạng `attachment`. Không khai (hay
  `application/octet-stream`) thì dùng loại nhận ra. Kiểm ngay khi luồng đủ 1 KiB: khai sai
  thì dừng tải lên giữa chừng, không chờ hết file.
- **Kích thước ảnh** bằng thư viện `imagesize` 0.15 (MIT, không phụ thuộc gì, chỉ đọc header,
  không giải mã ảnh), chỉ bật JPEG, PNG, GIF, WebP và HEIF (AVIF). Đọc từ 1 MiB đầu giữ lại
  trên đường tải lên; ảnh raster không đọc được kích thước thì từ chối (`media.invalid_image`).

Lý do: nhận loại là so vài byte — tự viết gọn và kiểm soát được (luật "không thêm thư viện
khi vài chục dòng là đủ"). Đọc kích thước đúng cho JPEG (nhiều đoạn metadata) và nhất là AVIF
(cây hộp ISO-BMFF, `ispe`, `irot`) thì không còn là vài chục dòng; `imagesize` nhỏ, giấy phép
hợp `cargo deny`.

Đã cân nhắc: crate `infer` cho nhận loại — nhiều loại hơn cần, vẫn phải tự viết phần so lời
khai; crate `image` — giải mã cả ảnh, quá nặng cho việc đọc header.

## D-090 · Không tải lên media qua MCP ở bước này — 2026-11-12 — agent quyết

Quyết định: tool MCP của `media` là `media.list`, `media.get`, `media.update` (thông tin) và
`media.trash`. Tải lên và thay nội dung chỉ qua REST.

Lý do: tool MCP đi qua JSON-RPC, nội dung nhị phân phải mã hoá base64 (to thêm một phần ba,
nằm trọn trong bộ nhớ), trong khi media thường là ảnh / video lớn — giới hạn 1 MiB như tool
của `files` (D-083) gần như vô dụng ở đây. AI agent có REST đầy đủ, cùng permission, kèm
`Idempotency-Key`. Thêm tool tải lên từ URL hay base64 nhỏ sau được mà không phá gì.

## D-091 · Module đầu tiên là `media` — 2026-11-12 — chủ dự án quyết

Quyết định: module nghiệp vụ đầu tiên của Autoxide là thư viện media (`media`, D-086). Bổ
sung D-016 (lúc đó "module đầu tiên chưa quyết"); phần còn lại của D-016 giữ nguyên: core
trung lập với nghiệp vụ, ai cũng dùng được.

Hệ quả: `CLAUDE.md` và `STACK.md` ghi `media` là module đầu tiên. Agent vẫn không tự chọn
module nghiệp vụ tiếp theo — module nội dung (`content`) dùng `media` chỉ dựng khi chủ dự án
giao.

## D-092 · Dependabot bỏ qua `p256` ≥ 0.14 cho tới khi `webauthn_rp` nâng theo — 2026-11-12 — chủ dự án quyết

Quyết định: đóng PR nâng `p256` 0.13 → 0.14 (thuongtamduy/autoxide#2) và thêm luật `ignore`
trong `.github/dependabot.yml` cho `p256` từ 0.14 trở lên. Merge PR nâng 5 action của GitHub
Actions (thuongtamduy/autoxide#1).

Lý do: `p256` chỉ là phụ thuộc test (authenticator phần mềm cho test passkey). Bản 0.14 đổi
API (`to_encoded_point` không còn) nên PR không biên dịch; `webauthn_rp` 0.3 vẫn dùng 0.13 —
nâng riêng phần test thì cây thư viện có hai bản `p256`. Nâng cùng lúc khi `webauthn_rp` lên
bản mới, rồi bỏ luật `ignore`. PR action: thay đổi phá vỡ của các bản mới chỉ chạm
`pull_request_target` / `workflow_run` và vài input cũ của action Docker — repo không dùng.

## D-093 · `media` làm ảnh thu nhỏ bằng job nền, thư viện `image` — 2026-11-12 — chủ dự án quyết, agent quyết cách làm

Thay D-087 (không làm ảnh thu nhỏ trong core): chủ dự án yêu cầu làm.

Quyết định:

- Sau khi tải lên / thay nội dung, cùng transaction đưa job `media.make_variants` vào hàng
  đợi của module. Job làm bản nhỏ hơn của ảnh JPEG, PNG, GIF, WebP theo các chiều rộng
  `AUTOXIDE_MEDIA_IMAGE_WIDTHS` (mặc định `320,640,1280,1920`; chỉ cỡ nhỏ hơn ảnh; `off` để
  tắt), xoay theo EXIF, giữ tỉ lệ, lọc `CatmullRom`; ảnh không trong suốt → JPEG chất lượng
  82, có kênh trong suốt → PNG. Không làm WebP / AVIF đầu ra: bộ mã hoá WebP thuần Rust chỉ
  có dạng không nén mất dữ liệu (to hơn JPEG), AVIF cần `rav1e` rất nặng.
- Thư viện `image` 0.25 (MIT / Apache-2.0, thuần Rust), chỉ bật `jpeg`, `png`, `gif`, `webp`,
  không `rayon`. Không giải mã AVIF (bộ giải mã cần `dav1d` viết bằng C) — AVIF không có bản
  thu nhỏ; SVG cũng không (vốn co giãn được).
- An toàn: ảnh quá 40 triệu pixel không được giải mã (kiểm từ kích thước đã lưu, rồi kiểm lại
  bằng `Limits` của `image`); giải mã chạy trong `spawn_blocking`; ảnh không giải mã được thì
  ghi log và thôi (không thử lại mãi).
- Bảng `media_variant (item_id, version_id, width)` duy nhất; khoá nơi lưu
  `media/<id>/<version_id>_w<width>`. Lưu dòng chỉ khi item còn đúng phiên bản (chạm dòng item
  trong transaction); thay nội dung xoá dòng + nội dung bản cũ; xoá hẳn xoá hết; job dọn mồ
  côi hiểu khoá có `_w<width>`.
- API: item có `variants` (rộng, cao, loại, kích thước, `url` = `url` của item + `&w=`).
  `GET …/content?w=<rộng>` trả bản thu nhỏ (`ETag` theo phiên bản + rộng, cùng luật cache /
  công khai / header an toàn); cỡ chưa làm xong thì trả bản gốc với cache ngắn, cỡ không tồn
  tại thì bản gốc. `api::MediaRef` có `variants`.

Lý do: website dùng ngay `srcset` mà không cần dịch vụ đổi cỡ riêng. Làm trong job nền nên
request tải lên không chậm thêm, và làm một lần thay vì mỗi lần xem.

Đã cân nhắc: đổi cỡ theo yêu cầu khi xem (`?w=` bất kỳ) — linh hoạt hơn nhưng mỗi cỡ lạ tốn CPU
ngay trong request và mở đường cho người lạ ép máy chủ làm việc nặng; dịch vụ ảnh riêng
(imgproxy…) — thêm một thứ phải vận hành.

## D-094 · `media` có nhãn, song song với thư mục — 2026-11-12 — chủ dự án quyết, agent quyết cách làm

Bổ sung D-088 (thư mục, chưa có nhãn): chủ dự án yêu cầu làm nhãn. Thư mục giữ nguyên.

Quyết định: bảng `media_tag` (tên duy nhất không phân biệt hoa thường, `name_key` `COLLATE "C"`
trên Postgres) và `media_item_tag`. Item mang tối đa 20 nhãn, mỗi nhãn 1–50 ký tự, không có
`,`. Nhãn tạo ngầm khi lần đầu gắn (`tags=a,b` khi tải lên, `tags: [...]` khi `PATCH` — thay cả
bộ); lọc `GET /media?tag=`; `/media/tags` liệt kê (kèm số item ngoài thùng rác), đổi tên
(`If-Match` bắt buộc), xoá (gỡ khỏi mọi item). Event `media.tag_updated`, `media.tag_deleted`;
thay đổi nhãn của item nằm trong `before` / `after` của `media.item_updated`.

Đồng thời đúng trên cả hai database: tạo nhãn bằng `INSERT … ON CONFLICT DO NOTHING` rồi đọc
lại (Postgres không hỏng transaction vì trùng), chạm dòng nhãn trước khi gắn; xoá nhãn xoá dòng
trước rồi gỡ liên kết — có test nhiều lần tải lên cùng lúc cùng tạo một nhãn mới.

Đổi tên nhãn không đổi `ETag` của item mang nó (bản ghi item không đổi).

## D-095 · Tải lên media qua MCP bằng base64, tối đa 1 MiB — 2026-11-12 — chủ dự án quyết, agent quyết cách làm

Thay D-090 (không tải lên qua MCP): chủ dự án yêu cầu làm.

Quyết định: tool `media.upload` (permission `media.write`) nhận `content_base64`, tên, loại (tuỳ
chọn), thư mục, nhãn; gọi đúng service tải lên của REST (kiểm loại thật, kích thước ảnh, báo
trùng, event). Tối đa 1 MiB sau giải mã, như tool của `files`; cả lời gọi còn phải lọt giới
hạn thân request chung (`AUTOXIDE_HTTP_BODY_LIMIT_BYTES`, mặc định 1 MiB). Thêm tool
`media.list_tags`.

Không làm "tải từ URL" (máy chủ tự tải file về): phải chặn địa chỉ mạng nội bộ (SSRF) cẩn thận
mới an toàn — để khi có nhu cầu thật.

<!-- Các mục D-096 … D-102 được ghi trên nhánh `ccr-ef1f8f71-civ2ra` dưới số D-085 … D-091 (cùng
ngày, song song với các mục D-085 … D-095 trên `main`). Khi gộp nhánh vào `main` (D-103), chúng
được đánh số lại theo thứ tự, nội dung giữ nguyên trừ số tham chiếu chéo. -->

## D-096 · Module đầu tiên là `cms` (nội dung); giao diện là dự án riêng, công khai; chưa làm app — 2026-11-12 — chủ dự án quyết

Quyết định:

- Module nghiệp vụ đầu tiên là `cms`: **chỉ nội dung và những gì liên quan đến nội dung**
  (bài viết, trang, danh mục, thẻ, menu, phiên bản, đa ngôn ngữ, SEO). Không có thương mại
  điện tử (sản phẩm, giỏ hàng, đơn hàng, thanh toán).
- Giao diện — trang quản trị nội dung và website công khai hiển thị nội dung — là một dự án
  riêng, **công khai toàn bộ** như core, chỉ dùng API của core (D-026).
- Chưa làm app di động.

Lý do: chủ dự án chọn.

Hệ quả: các khái niệm của `cms` (bài viết, trang, slug…) chỉ nằm trong `modules/cms`; thứ
`cms` cần mà module khác cũng dùng được (lọc và sắp xếp danh sách, tìm kiếm văn bản, đọc
công khai có cache) thì thêm vào core ở dạng trung lập với nghiệp vụ. Việc cụ thể ở bước 15
của `docs/ROADMAP.md`.

Thay: phần "Module đầu tiên chưa quyết" của D-016.

## D-097 · Stack của dự án giao diện — 2026-11-12 — agent quyết

Quyết định: dự án giao diện (repo riêng, gợi ý tên `autoxide-web`) dùng:

- TypeScript strict; một monorepo pnpm workspaces + Turborepo; Biome để lint và format.
- SDK sinh từ `docs/openapi.json` của core bằng `openapi-typescript` + `openapi-fetch`, có
  lớp chặn tự gắn `Idempotency-Key` cho lệnh ghi và `Accept-Language`.
- Next.js 16 (App Router, React 19) cho **cả** trang quản trị và website công khai. Máy chủ
  Next.js đứng giữa (BFF) và giữ refresh token trong cookie `httpOnly`; trình duyệt không
  bao giờ cầm refresh token.
- Tailwind CSS 4 + shadcn/ui; TanStack Query và TanStack Table; React Hook Form + Zod;
  Tiptap làm trình soạn thảo (nội dung lưu dạng JSON); next-intl cho giao diện vi và en;
  thông báo lỗi tra theo `code` của core.
- Vitest, Playwright, MSW để test; build `output: standalone` chạy trong Docker cạnh core.

Lý do: một ngôn ngữ và một framework cho cả hai ứng dụng (mỗi việc một cách làm); website
công khai cần render phía máy chủ cho SEO; core trả refresh token trong thân JSON nên cần
BFF để không lộ token khi có lỗi XSS; kiểu dữ liệu sinh từ OpenAPI nên API đổi thì trình
biên dịch báo ngay; các thư viện trên phổ biến nhất trong hệ React, AI agent viết tốt.

Đã cân nhắc: Vite SPA cho trang quản trị — phải tự giải quyết chỗ cất refresh token và có
hai cách làm web; Nuxt, SvelteKit — hệ sinh thái nhỏ hơn; Astro cho website — tốt cho nội
dung tĩnh nhưng thêm một framework thứ hai; `@hey-api/openapi-ts` — sinh SDK đầy đủ hơn
nhưng chưa tới 1.0; Flutter / Expo cho app — chưa làm app (D-096).

Ghi chú: phiên bản cụ thể ghim trong repo giao diện khi tạo; mục này chỉ chốt lựa chọn.

## D-098 · Module nội dung tên là `content`, không phải `cms` — 2026-11-12 — chủ dự án quyết

Quyết định: module nội dung của D-096 tên là `content` (thư mục `modules/content`, crate
`autoxide-content`; bảng `content_…`; route `/api/v1/content/…`; permission, mã lỗi và event
mang tiền tố `content.`; cấu hình `AUTOXIDE_CONTENT_…`). Phạm vi giữ nguyên như D-096.

Lý do: chủ dự án thấy `cms` quá chung chung. "CMS" là tên một ứng dụng (màn hình quản trị +
nội dung), còn module được đặt theo dữ liệu nó sở hữu. Nội dung còn được nơi khác dùng —
website, storefront nếu sau này có (trang giới thiệu, blog, banner), AI agent qua MCP — nên
tên phải nói về dữ liệu, không về một màn hình. Sản phẩm, đơn hàng nếu có sau này là module
riêng; `content` không biết về chúng.

Đã cân nhắc: `posts` — hẹp, nội dung còn có trang và menu; `publishing` — dài, mơ hồ.

Thay: tên `cms` trong D-096.

## D-099 · Nâng `files` thành thư viện media trước khi làm `content` — 2026-11-12 — chủ dự án quyết

Quyết định: thứ tự là bước 15 (nền chung trong core: lọc và sắp xếp, tìm kiếm, đọc công khai,
cache, `If-Match`) → bước 16 (module `files` thành thư viện media dùng chung: file công khai,
hiển thị ảnh inline an toàn, metadata ảnh, thư mục hoặc nhãn, thùng rác, `api` cho module
khác, tool MCP) → bước 17 (module `content`). Tên module vẫn là `files`.

Lý do: chủ dự án chọn. `content` dựa vào ảnh (ảnh đại diện, ảnh trong bài, ảnh chia sẻ), và
website phải hiển thị được ảnh cho người đọc không đăng nhập — hiện `files` chỉ là kho file
riêng, luôn tải về dạng attachment (D-067). Làm `files` trước thì `content` chỉ việc giữ ID
file và gọi `api` của `files`.

Đã cân nhắc: đổi tên `files` thành `media` — tên module là hợp đồng (bảng, route, permission,
mã lỗi), đổi làm vỡ client đang dùng mà không thêm giá trị; dữ liệu vẫn là file.

Thay: thứ tự "module `content` ngay sau bước 12" của D-096.

## D-100 · `files` giữ nguyên; thư viện media là module riêng `media` — 2026-11-12 — chủ dự án quyết

Quyết định: không nâng `files` thành thư viện media như D-099 viết. `files` giữ nguyên vai kho
file thô, riêng tư theo người sở hữu, dùng cho mọi module. Thư viện media cho xuất bản là
module mới `media` (bước 16 của lộ trình): lưu nội dung qua trait `Storage` của kernel (do
`files` cung cấp) dưới khoá `media/<id>`, metadata ở bảng `media_…`, không tham chiếu bảng của
`files`. `content` giữ ID của `media`. Thứ tự giữ như D-099: nền chung → `media` → `content`.

Lý do: chủ dự án đề xuất. Hai việc khác nhau: cất file (đính kèm, xuất báo cáo…) cần đơn giản
và riêng tư; xuất bản cần công khai, hiển thị inline, alt text, thư mục, thùng rác. Tách ra thì
`files` không phải mang khái niệm của xuất bản, và file riêng tư không thể lỡ bị công khai vì
không cùng bảng với item công khai. Trait `Storage` ở kernel có sẵn cho đúng việc này (D-066).

Đã cân nhắc: `media` chỉ trỏ tới dòng của `files` rồi thêm metadata — tải lên hai bước, xoá
phải phối hợp hai module, dữ liệu một tấm ảnh nằm ở hai nơi; nâng `files` (D-099) — trộn hai
vai vào một module.

Thay: phần "nâng `files` thành thư viện media" của D-099.

## D-101 · Trang có cấp cha con ngay trong bước 17 — 2026-11-12 — chủ dự án quyết

Quyết định: trang (`kind = page`) của `content` có trang cha (để trống được), đường dẫn lồng
nhau (`/gioi-thieu/doi-ngu`), làm ngay trong bước 17 — không tách thành bước sau. Bài viết
(`kind = post`) vẫn phẳng, gom nhóm bằng danh mục và thẻ. Trang không có cha thì là trang phẳng
như thường.

Lý do: chủ dự án chọn. Website công ty, trường học, tổ chức có vài chục trang thường cần đường
dẫn lồng nhau; làm luôn thì không phải mở thêm một bước và website không phải sửa lần hai.

Đã cân nhắc: làm phẳng trước, thêm cây sau (gọn hơn khoảng 15–20% khối lượng bước 17, cây
thêm sau được mà không phá API); cho slug chứa `/` gõ tay (không phải cây thật, đổi "cha"
không tự cập nhật trang con).

## D-102 · Thiết kế module `content` — 2026-11-12 — agent quyết

Dựa trên khảo sát Ghost, WordPress, Contentful, Sanity, Wagtail, Strapi 5, Payload và các
chuẩn ngành ở `docs/research/content.md`. Chi tiết đủ để làm ở `docs/prompts/step-17-content.md`.

Quyết định:

- **Nháp tách khỏi bản công khai.** `content_entries` giữ bản nháp làm việc (luôn có);
  `content_published` giữ bản đang công khai (chỉ có khi đang công khai). Publish chép nháp
  sang trong một transaction. Endpoint công khai chỉ đọc `content_published`, nên không thể lộ
  nháp. Trạng thái: `draft` / `published` / `archived` + cờ `has_unpublished_changes`; hẹn giờ
  là tài nguyên riêng (`content_schedules`), không phải một trạng thái.
- **Mỗi bản dịch là một bản ghi** với slug và trạng thái publish riêng, nối bằng
  `translation_group_id`. Bài và trang **không fallback** sang ngôn ngữ khác (thiếu thì 404),
  vì fallback làm hỏng canonical và hreflang; tên danh mục, thẻ, menu thì fallback về ngôn ngữ
  mặc định và response ghi ngôn ngữ thật đã trả.
- **Backend tính đường dẫn** (`path`) của mỗi mục: trang = slug của các trang tổ tiên + slug;
  bài = tiền tố cấu hình (mặc định `blog`) + slug. Website chỉ ghép tên miền và tiền tố ngôn
  ngữ. Website tra nội dung bằng một endpoint `resolve` theo (ngôn ngữ, đường dẫn), trả về nội
  dung, chuyển hướng, hoặc "đã gỡ" (410).
- **Đường dẫn công khai đổi thì tự tạo chuyển hướng** (301) cho mục đó và mọi trang con, trỏ
  tới ID (không thành chuỗi chuyển hướng). Gỡ hẳn (xoá, lưu trữ) bản đang công khai thì ghi
  "đã gỡ" (410). Chuyển hướng làm tay được 301/302/307/308/410.
- **Đổi trang cha là một thao tác riêng** (`move`), có hiệu lực ngay cho cả nháp và bản công
  khai; mọi thay đổi cây trong một ngôn ngữ đi qua một dòng khoá chung để hai lệnh `move` đồng
  thời không tạo vòng lặp trên Postgres.
- **Revision là ảnh chụp đầy đủ**, tạo ở service layer (REST và MCP như nhau); khôi phục tạo
  nháp mới, không tự đăng. Hẹn giờ ghim một revision; lúc chạy, nếu bản công khai đã mới hơn
  lúc đặt lịch thì lịch thất bại với mã riêng, không ghi đè.
- **Bắt buộc `If-Match`** cho lệnh sửa, xoá và mọi thao tác đổi trạng thái của `content`;
  thiếu thì 428 `precondition_required`. Tool MCP tương ứng nhận tham số `version`. Đây là
  ngoại lệ có chủ đích so với D-084 (ở đó không bắt buộc để không phá client cũ): `content` là
  module mới, chưa có client cũ, và AI agent hay sửa song song.
- **Máy chủ kiểm JSON thân bài** theo danh sách node, mark, thuộc tính được phép (từ chối mặc
  định), chặn scheme link ngoài `http`/`https`/`mailto` và đường dẫn tương đối, ảnh chỉ giữ ID
  của `media`, giới hạn kích thước và độ sâu, báo mọi lỗi một lần kèm vị trí. Không lưu HTML.
- **Tác giả là hồ sơ của `content`** (`content_authors`: tên hiển thị, slug, giới thiệu, ảnh),
  giữ `user_id` không foreign key; một bài có nhiều tác giả có thứ tự. `content` không phụ
  thuộc module `auth` (D-046).
- **Preview token** ký HMAC, chứa ID mục và hạn dùng, không lưu bảng; chưa cấu hình secret thì
  tính năng tắt và báo lỗi rõ.
- **Slug** sinh bằng crate `slug` (chạy thử: "Đường đến Đà Lạt" → `duong-den-da-lat`; tự bỏ
  dấu và đổi `đ` → `d`); thêm vào `STACK.md` khi làm.

Lý do: đặt đúng ranh giới (nháp / công khai, giữa các bản dịch, backend / website) ngay từ
migration đầu thì redirect, sitemap có hreflang, feed, preview, webhook làm mới cache chỉ còn là
truy vấn trên bảng thường, chạy như nhau trên SQLite và Postgres. Các CMS đặt sai ranh giới phải
vá về sau (khảo sát ở `docs/research/content.md`).

Đã cân nhắc: mẫu Wagtail và Payload (bảng chính là bản công khai, nháp nằm trong revision) —
trang quản trị phải đọc nháp từ JSON, khó lọc và sắp xếp; dịch theo trường bằng cột JSON — khó
đánh index trên SQLite và phải vá publish theo ngôn ngữ như Contentful; khoá bài khi đang sửa
(Payload, WordPress) — `If-Match` theo chuẩn HTTP và không giữ trạng thái; mã chuyển hướng 308
— Google coi như 301, còn 301 được mọi công cụ hiểu.

## D-103 · Gộp nhánh đề bài `content` vào `main`: đánh số lại, bước 15–16 đã xong — 2026-11-12 — agent quyết

Bối cảnh: đề bài bước 17 (`content`) được soạn trên nhánh `ccr-ef1f8f71-civ2ra`, tách ra từ
`main` trước khi `media` được làm. Nhánh đó ghi quyết định D-085 … D-091, trùng số với D-085 …
D-095 trên `main`, và lộ trình của nó tách "nền chung" (bước 15) khỏi "`media`" (bước 16).

Quyết định:

- Quyết định của nhánh được đánh số lại D-096 … D-102 (thứ tự giữ nguyên, nội dung giữ nguyên
  trừ số tham chiếu chéo); đề bài và bản khảo sát được sửa số theo.
- Module đầu tiên: chủ dự án chọn cả `media` (D-091) lẫn `content` (D-096, D-098) trong cùng
  ngày, ở hai phiên khác nhau. Không đảo cái nào: `media` đã làm trước, `content` là bước kế.
  `CLAUDE.md`, `STACK.md` ghi cả hai.
- Lộ trình: bước "15. Thư viện media" của `main` tách lại thành bước 15 (nền chung) và 16
  (`media`) cho khớp đề bài; bước 17 lấy từ nhánh.
- So với yêu cầu bước 15–16 của nhánh, phần đã làm trên `main` khác ở vài chỗ, giữ nguyên:
  - endpoint công khai khai bằng `Option<Principal>` trong handler + `security((), …)` trong
    OpenAPI, không qua một hàm mới của hợp đồng module — một cách làm, đã có test;
  - tìm theo chữ là một hàm của kernel trên cột "khoá tìm" (`search::normalize` +
    `search::contains`), không sau một trait: một câu `LIKE` chạy như nhau trên hai database,
    không có nhánh rẽ nào để giấu (CLAUDE.md chỉ yêu cầu trait khi phải rẽ nhánh);
  - `media` có cả thư mục lẫn nhãn (chủ dự án yêu cầu, D-094), có ảnh thu nhỏ (D-093).
- Mục còn thiếu của bước 15 — tìm được tiếng Việt không dấu — làm trước khi vào bước 17.

## D-104 · Tìm theo chữ không phân biệt dấu: `search::fold` — 2026-11-12 — agent quyết

Quyết định: kernel thêm `search::fold` = viết thường, tách chữ khỏi dấu (Unicode NFD), bỏ dấu,
`đ` → `d` ("Đường đến Đà Lạt" → "duong den da lat"). `search::contains` so trên dạng đó, nên mọi
cột "khoá tìm" lưu dạng `fold`: `auth_user.search_text` (tính lại ở m0017), `media_item.name_key`
và cột mới `media_tag.search_key` (m0005). `search::normalize` (chỉ viết thường) vẫn là khoá
chặn trùng và để sắp khi cần phân biệt dấu: "Anh" và "Ảnh" là hai nhãn, hai thư mục khác nhau.
Thư viện `unicode-normalization` (đã có sẵn trong cây phụ thuộc).

Lý do: mục cuối của bước 15 (đề bài D-099): người Việt hay gõ không dấu. Làm trong Rust thì hai
database cho cùng kết quả; `unaccent` chỉ Postgres có, `LOWER` của SQLite chỉ biết ASCII.

Đã cân nhắc: full-text search riêng của từng database (FTS5, `tsvector`) — mạnh hơn nhưng phải rẽ
nhánh theo database, và cấu hình tiếng Việt của Postgres không có sẵn; để khi bảng lớn tới mức
`LIKE '%…%'` chậm thật (đo trước). Bỏ dấu cả ở khoá chặn trùng — gộp nhầm hai từ khác nghĩa.

## D-105 · Module `content`: những chỗ đề bài chưa nói hoặc làm khác — 2026-11-12 — agent quyết

Bối cảnh: làm bước 17 theo `docs/prompts/step-17-content.md` và D-102. Các chỗ dưới đây đề bài
không nói rõ, hoặc làm khác một chút; ghi lại để phiên sau không đảo nhầm.

Quyết định:

- **Không foreign key cả bên trong module.** Service tự xoá dòng con (liên kết term, tác giả,
  revision, lịch, mục menu) trong cùng transaction. Lý do: hành vi xoá giống hệt trên hai
  database (SQLite chỉ thi hành foreign key khi bật `PRAGMA`), và luật "chỉ giữ ID" đã áp cho
  ID của `media` và người dùng — một cách làm cho mọi ID.
- **Endpoint công khai nằm dưới `/api/v1/content/public/…`** (`resolve`, `posts`, `menus`,
  `terms`, `authors`, `sitemap`, `feed`, `preview`): một tiền tố cho mọi thứ không cần đăng nhập,
  dễ đặt cache và rate limiting ở proxy. Mọi endpoint này trả `Cache-Control: public, no-cache`
  (cache được, hỏi lại rẻ nhờ `ETag` + 304); `preview` trả `no-store`.
- **Phạm vi `If-Match` bắt buộc:** sửa, xoá và đổi trạng thái mục (đăng, gỡ, lưu trữ, `move`,
  khôi phục, hẹn giờ); sửa chuyển hướng, term, tác giả; thay menu (`*` để tạo menu mới). Xoá
  term, tác giả, chuyển hướng không cần: không có nội dung nào bị ghi đè.
- **Gắn term và tác giả bằng endpoint riêng** (`PUT /entries/{id}/terms`, `/authors`), có hiệu
  lực ngay cho cả nháp lẫn bản công khai, không nằm trong revision: đó là phân loại, không phải
  nội dung; website lọc theo danh mục phải thấy ngay.
- **Lỗi của một trường là `errors[].code`, không phải mã riêng của response:**
  `locale_not_supported`, `parent_not_found`, `tree_too_deep`, `not_allowed`… nằm trong
  `validation_failed` để báo mọi lỗi một lần (D-083). Mã riêng `content.invalid_body`,
  `content.media_not_found`, `content.media_not_public` chỉ dùng khi mọi lỗi cùng một loại.
- **Lịch thất bại không retry.** Lỗi nghiệp vụ lúc chạy (bản công khai đã mới hơn lúc đặt lịch,
  trang cha chưa đăng, slug trùng…) đánh dấu lịch `failed` kèm mã lỗi và ghi event
  `content.schedule_failed`; chạy lại không đổi kết quả. Chỉ lỗi hạ tầng (database) mới để job
  queue retry.
- **Preview token:** `base64url(ID 16 byte + hạn dùng 8 byte) . base64url(HMAC-SHA256)`, ký
  bằng `AUTOXIDE_CONTENT_PREVIEW_SECRET` (ít nhất 32 ký tự). Token luôn cho xem bản nháp mới
  nhất; mục bị xoá thì token hết dùng được. Đổi secret là thu hồi mọi token.
- **Tool MCP:** 9 tool (`list`, `get`, `create_draft`, `update`, `publish`, `unpublish`,
  `list_revisions`, `restore_revision`, `resolve`). Kết quả có mục kèm `version` (giá trị
  `ETag`); tool sửa và đổi trạng thái bắt buộc `version`, đọc y như `If-Match` qua hàm mới
  `IfMatch::from_value` của kernel. Đầu vào là kiểu riêng của MCP, không `$ref` (thân bài khai
  là object chung, máy chủ vẫn kiểm đủ).
- **Xoá tài khoản** xoá hồ sơ tác giả gắn với người đó và liên kết của hồ sơ với bài, huỷ lịch
  người đó đặt, bỏ trống `created_by` ở mục, revision, lịch, chuyển hướng; bài vẫn giữ.
- **Kernel:** `pagination::SortValue` thêm loại `Time` để danh sách bài công khai sắp theo
  ngày đăng (`published_at`) với con trỏ ổn định.

Lý do: mỗi điểm chọn cách đơn giản nhất chạy như nhau trên SQLite và Postgres và giữ "một cách
làm cho mỗi việc".

Đã cân nhắc: foreign key `ON DELETE CASCADE` trong module — hành vi phụ thuộc cấu hình kết nối
SQLite; lưu preview token vào bảng để thu hồi từng cái — thêm bảng và dọn dẹp cho một tính năng
token sống 10 phút; retry lịch thất bại — lỗi nghiệp vụ không tự hết, retry chỉ làm ồn log.

## D-106 · Sửa lỗi sau lượt rà soát độc lập module `content` — 2026-11-12 — agent quyết

Một lượt rà soát độc lập (đọc code, ba phần song song) tìm ra các lỗi dưới đây; mỗi lỗi có test
riêng chạy trên cả SQLite và Postgres (`tests::ra_soat`, `http::tests::etag_resolve_…`, test của
`body` và `paths`).

Quyết định:

- **Chuyển trang tính đường dẫn công khai từ slug đã đăng**, không từ slug của nháp (trước đây
  `move` làm lộ slug nháp chưa duyệt lên địa chỉ công khai).
- **Đường dẫn mới của trang con trùng** một bản công khai khác (khi đổi slug trang cha hay chuyển
  trang) báo `content.path_taken` như với chính trang được đổi, thay vì lỗi 500; lịch hẹn giờ gặp
  lỗi này thì thất bại thay vì thử lại mãi.
- **Từ chối `\`** trong URL nhúng, link "trong site" và mọi đường dẫn chuẩn hoá: trình duyệt coi
  `\` như `/` (chuẩn WHATWG URL), nên `https://evil.com\.youtube.com/x` lọt danh sách host được
  phép và `/\evil.com` là link ra ngoài.
- **`ETag` của `resolve` là dấu vân tay nội dung** (SHA-256 của câu trả lời JSON, 16 byte đầu,
  base64url) thay vì `updated_at` của bản công khai: câu trả lời gộp danh mục, thẻ, tác giả, bản
  dịch và ảnh, mà đổi những thứ đó không chạm `updated_at` — website từng nhận 304 với nội dung cũ.
- **Gắn danh mục / tác giả cho mục đang công khai cần thêm `content.publish`**: thay đổi có hiệu
  lực ngay với người đọc, như mọi thay đổi bản công khai khác.
- **Khoá thêm:** tạo trang con (kể cả bản dịch) lấy khoá cây của ngôn ngữ rồi kiểm lại trang cha
  và độ sâu trong transaction (không còn trang con mồ côi khi xoá cha cùng lúc; bản dịch không
  vượt 5 cấp); tạo / sửa chuyển hướng lấy cùng khoá trước khi kiểm vòng lặp; tạo / đổi cha / xoá
  danh mục lấy khoá cây danh mục (dòng `#categories` của bảng khoá cây — không trùng được mã ngôn
  ngữ BCP 47) và kiểm lại cha trong transaction.
- **Chuyển hướng làm tay chỉ trỏ tới mục cùng ngôn ngữ** (`to_entry_id`, lỗi `other_locale`).
  Chuyển hướng trỏ tới mục đã lưu trữ trả 410 (không phải 404), khớp tài liệu.
- **Lịch:** đăng theo lịch lấy vị trí trong cây từ bản đọc sau khi khoá (nội dung vẫn từ revision
  đã ghim); gỡ theo lịch ghi revision `unpublished` như gỡ tay; xoá tài khoản huỷ lịch từng cái,
  kèm event `content.schedule_canceled`; dọn revision không bao giờ xoá revision đang là bản công
  khai, kể cả khi một lịch vừa chạy.
- **Ảnh đại diện riêng tư của tác giả** không lộ cả ID trong bài công khai.
- **Làm mới cache khi thẻ / tác giả đổi:** event `content.term_changed` / `content.author_changed`
  không kèm danh sách trang (một thẻ có thể trên hàng nghìn bài); website gắn tag cache theo ID
  term / tác giả và làm mới khi nhận event — ghi trong `docs/CONTENT.md`.
- `docs/CONTENT.md` sửa phạm vi `If-Match` cho khớp D-105 (xoá term, tác giả, chuyển hướng và gắn
  term / tác giả không cần).

Lý do: các lỗi trên hoặc lộ nội dung chưa duyệt, hoặc mở đường nhúng trang lạ, hoặc làm website
giữ nội dung cũ, hoặc chỉ hiện ra khi ghi đồng thời trên Postgres — đều trái tiêu chí "đầy đủ" và
luật hai database.

Đã cân nhắc: phát event "mục đổi" cho từng bài khi xoá / đổi tên thẻ, tác giả — có thể hàng nghìn
event một lần, nặng hơn quy ước tag cache; `ETag` từ thời điểm lớn nhất của mọi thứ gộp vào — phải
thêm cột thời gian cho bảng liên kết và vẫn sót ảnh (thuộc module `media`); khoá danh mục bằng
bảng riêng — thêm migration chỉ để có một dòng khoá.

## D-107 · Tắt CI trên GitHub; gate cục bộ là gate duy nhất — 2026-11-12 — chủ dự án quyết

Quyết định:

- Ba workflow trong `.github/workflows` (`CI`, `Security audit`, `Release`) **không tự chạy
  nữa** (không theo push, pull request, lịch hay tag); chỉ còn `workflow_dispatch` (bấm tay). File
  giữ nguyên để bật lại dễ.
- `./gate.sh` trên máy là gate duy nhất, như CLAUDE.md đã quy định: fmt, clippy, kiểm ranh giới,
  `cargo deny` (gồm lỗ hổng thư viện), test trên SQLite và Postgres.
- Phát hành làm tay theo `docs/RELEASING.md`: tag, bản phát hành GitHub tạo trên trang web,
  image Docker build trên máy và chỉ đăng khi chủ dự án cấp token.
- Dependabot vẫn mở PR nâng thư viện Rust và Docker (chạy trên hạ tầng riêng của GitHub, không
  cần runner trả phí); người duyệt chạy `./gate.sh` trên nhánh PR trước khi merge. Bỏ phần nâng
  phiên bản GitHub Actions vì workflow không còn tự chạy.

Lý do: chủ dự án không gắn thẻ thanh toán cho tài khoản GitHub; mọi lần chạy CI đều dừng sau vài
giây mà không khởi động job, chỉ sinh báo đỏ vô nghĩa trên mỗi commit.

Đã cân nhắc: xoá hẳn các workflow — mất công dựng lại khi có runner (ví dụ runner tự host); runner
tự host — cần một máy chạy liên tục, chủ dự án chưa có.

Hệ quả: mất quét secret bằng gitleaks trong CI (D-074). Thay thế: bật "Secret scanning" và "Push
protection" của GitHub (miễn phí cho repo công khai, Settings → Code security) — việc của chủ dự
án; agent vẫn giữ luật "secret không vào git".

Thay: phần "chạy trên GitHub Actions" của D-074 và phần workflow tự động của D-078; mục "CI chạy
xanh trên GitHub" và "CI build và đăng image" của bước 13.

## D-108 · Quét secret bằng gitleaks trong gate — 2026-11-12 — chủ dự án quyết, agent quyết cách làm

Quyết định: `./gate.sh` có thêm bước 5 "secrets": gitleaks quét toàn bộ lịch sử git, rồi thay
đổi đã stage và chưa stage, theo `.gitleaks.toml`. Máy có `gitleaks` thì dùng nó; không thì chạy
image Docker ghim `zricethezav/gitleaks:v8.30.0` (Docker vốn đã cần cho Postgres của gate; đổi
bằng `GATE_GITLEAKS_IMAGE`). Có secret thì gate đỏ, không commit được.

Lý do: chủ dự án giữ repo riêng tư (chọn hướng B): GitHub không cho secret scanning miễn phí với
repo riêng tư, và CI đã tắt (D-107) nên bước gitleaks trong CI (D-074) không còn chạy. Đưa vào
gate thì mỗi lần "xong" đều được quét, không cần dịch vụ ngoài.

Ngoại lệ: lần quét đầu tìm một chỗ trong lịch sử — khoá ví dụ **công khai** của bộ test chính
thức Standard Webhooks (`modules/webhooks`, test khớp chữ ký), không phải secret. `.gitleaks.toml`
thêm một ngoại lệ đúng chuỗi đó, không nới luật nào khác. Đã thử ngược: một khoá giả chưa commit
làm bước này đỏ.

Đã cân nhắc: chuyển repo sang công khai để dùng secret scanning và CI miễn phí của GitHub (hướng A)
— chủ dự án chưa muốn; image `ghcr.io/gitleaks/gitleaks` như CI cũ — cùng phiên bản nhưng tải từ
GHCR không ổn định ở mọi mạng, Docker Hub phổ biến hơn.

Thay: phần quét secret trong CI của D-074; phần "Hệ quả" của D-107 (bật secret scanning của GitHub).

## D-109 · Tính năng AI: tầng kết nối trung lập, hai adapter, quyền cho agent, mặc định tắt — 2026-11-12 — chủ dự án quyết, agent quyết cách làm

Chủ dự án đề xuất bốn tính năng AI (tìm theo ý nghĩa, AI dịch bài vi ↔ en, gợi ý SEO và alt
text, giới hạn quyền cho agent tự viết và đăng bài) và đồng ý hướng làm dưới đây. Đề bài đầy đủ:
`docs/prompts/ai.md`; khảo sát và số đo: `docs/research/ai.md`. Thành bước 18 của lộ trình.

Quyết định (chủ dự án):

- Làm cả bốn tính năng, theo thứ tự: quyền cho agent → tầng kết nối AI → gợi ý SEO / alt →
  dịch → tìm theo ý nghĩa.
- **Hỗ trợ nhiều nhà cung cấp ngay từ đầu**, không chỉ Anthropic: adapter `anthropic`
  (Messages API gốc) và adapter `openai` (Chat Completions — dùng được với OpenAI, Gemini qua
  endpoint tương thích OpenAI, Ollama, vLLM, LM Studio, OpenRouter… bằng cách đổi base URL).
  Embedding là client riêng theo dạng `/v1/embeddings`.
- **Mặc định tắt.** Người cài tự chọn nhà cung cấp và tự đặt key; không gửi dữ liệu cho bên nào
  khi chưa cấu hình.

Quyết định (agent, cách làm):

- Trait `TextModel` và `Embedder` trong kernel (trung lập với nhà cung cấp và nghiệp vụ); module
  hạ tầng `ai` công bố cài đặt — cùng khuôn với `Mailer` / module `mail`. Module khác không gọi
  nhà cung cấp trực tiếp.
- Mỗi lời gọi sinh chữ trả JSON theo schema, và luôn được kiểm lại trong Rust.
- Không gọi AI trong transaction ghi. Việc có ghi chạy bằng job; gợi ý chạy ngay trong request.
  Kết quả AI chỉ thành nháp hoặc gợi ý, không bao giờ tự đăng.
- Dịch tách thân bài thành mẩu có thẻ giữ chỗ, ráp lại theo cấu trúc gốc. Mô hình không bao giờ
  thấy hay sửa link, ID ảnh, cấu trúc node.
- Tìm theo ý nghĩa: vector lưu nhị phân trong bảng thường, cosine duyệt hết trong bộ nhớ, gộp
  với tìm theo chữ bằng Reciprocal Rank Fusion; endpoint công khai tắt mặc định. Đo trước khi
  tính tới index ANN.
- Quyền mới: `content.delete` (tách khỏi `content.publish`), `content.write_own`,
  `content.publish_own`; xin duyệt; `ai_generated` / `ai_reviewed`; rate limiting theo API key.
- Không thêm thư viện.

Lý do: OpenAI, Gemini và phần mềm chạy tại máy đều nói được Chat Completions. Vì vậy hai adapter
phủ gần hết lựa chọn mà người cài có, không phải giữ một adapter cho mỗi hãng. Anthropic giữ
adapter gốc vì lớp tương thích OpenAI làm mất fallback khi bị từ chối và `stop_reason` đầy đủ.

Về hai database: `pgvector` chỉ Postgres có. `sqlite-vec` là extension C; với sqlx 0.9, nạp
extension phải gọi API `unsafe`, mà repo cấm `unsafe`. Vector trong bảng thường với cosine tính
trong Rust chạy như nhau trên hai database. Ước tính dưới 50 ms với 100.000 đoạn — số đo thật
ghi khi làm.

Về quyền: tách quyền và bắt người duyệt các việc có tác động lớn là cách chống prompt injection
mà OWASP khuyên (LLM01, LLM06). `ai_reviewed` ghi lại sự thật để người vận hành làm đúng EU AI
Act Điều 50.

Đã cân nhắc:

- **Chỉ Anthropic:** trái mục tiêu "ai cũng dùng được".
- **Một adapter riêng cho mỗi hãng**, gồm cả Gemini gốc và Responses API của OpenAI: nhiều code
  phải giữ hơn. Chỉ thêm khi đo được Chat Completions thiếu thứ cần.
- **Thư viện nhiều nhà cung cấp** (`genai`, `rig`, `async-openai`): kéo theo phụ thuộc và thay
  đổi API thường xuyên, trong khi phần cần dùng chỉ vài trăm dòng HTTP.
- **Chạy mô hình trong binary** (ONNX, candle): binary nặng, trái "không thêm thư viện thừa".
- **Gửi nguyên HTML / JSON của bài cho mô hình dịch:** module AI Translate của Drupal làm vậy và
  có nhiều lỗi mở về thẻ bị mất hoặc sửa.
- **Để AI tự ghi thẳng vào bản công khai:** mọi CMS đã khảo sát đều đưa kết quả về nháp hoặc gợi
  ý.

## D-110 · Bước 18 (AI): những chỗ đề bài chưa nói hoặc làm khác — 2026-11-12 — agent quyết

Bối cảnh: làm bước 18 theo `docs/prompts/ai.md` và D-109. Ghi lại để phiên sau không đảo nhầm.

Quyết định:

- **Tên biến cấu hình theo module đọc nó.** `AUTOXIDE_AI_SEND_PRIVATE_MEDIA` của đề bài thành
  `AUTOXIDE_MEDIA_AI_SEND_PRIVATE`, `AUTOXIDE_AI_TRANSLATE_BATCH_TOKENS` thành
  `AUTOXIDE_CONTENT_TRANSLATE_BATCH_TOKENS`. Lý do: mỗi biến khai một lần, ở module dùng nó;
  module `ai` không biết ảnh hay bài viết. Thêm `AUTOXIDE_AI_IMAGES` (mô hình có đọc ảnh không),
  vì Chat Completions không cho biết điều đó.
- **`TextModel::ready()` / `Embedder::ready()`** trong trait của kernel. Module dùng AI kiểm "đã
  bật chưa" trước việc tốn kém (đọc và thu nhỏ ảnh, xếp job dịch). Lỗi là chính lỗi `ai.*` của
  module `ai`, nên không module nào phải tự chế mã riêng.
- **Mã lỗi của `content` cho phần dịch.** Bản dịch sai cấu trúc là `content.translation_invalid`,
  không phải `ai.output_invalid`: mã lỗi mang tiền tố module tạo ra nó (`cargo xtask check`).
  Ngôn ngữ đích trùng ngôn ngữ nguồn là lỗi trường `locale` (`validation_failed`), như khi tạo
  bản dịch bằng tay, không có mã riêng `translation_same_locale`. Tài khoản đã yêu cầu bị xoá thì
  yêu cầu dừng với `error_code` `content.translation_canceled`; đây là giá trị lưu trong bảng,
  không phải lỗi API.
- **Chặn hai yêu cầu dịch cùng lúc bằng cột `active_key`** (`<nhóm bản dịch>:<ngôn ngữ>` khi đang
  chờ hay đang chạy, `NULL` khi xong) có unique index. Cả SQLite và Postgres coi hai `NULL` là
  khác nhau, nên không cần partial index. Khoá theo nhóm bản dịch, không theo bản gốc: hai bản gốc
  của cùng nhóm không cùng ghi một bản dịch.
- **Bản dịch mới lấy slug từ tiêu đề đã dịch.** Gửi `slug: ""` khi tạo bản dịch nghĩa là "sinh từ
  tiêu đề, trùng thì thêm hậu tố", như khi không gửi slug. Bản dịch có sẵn giữ slug, để địa chỉ
  không đổi.
- **Tìm công khai: ngân sách chung mỗi phút, hết thì chỉ tìm theo chữ.** Đề bài viết "rate
  limiting theo địa chỉ client". Đổi lại vì điều cần chặn là chi phí embedding, không phải số
  request (rate limiting theo địa chỉ của kernel đã có). Hết ngân sách thì vẫn trả kết quả theo
  chữ, không trả 429. Đếm trong tiến trình. `AUTOXIDE_CONTENT_PUBLIC_SEARCH_PER_MINUTE` mặc
  định 30.
- **Lập index còn thiếu bằng lịch chạy mỗi giờ** (`content.catch_up_embeddings`, mỗi lần tối đa
  500 mục), không bằng lệnh CLI. Bật embedding hay đổi mô hình là tự lập lại, không ai phải nhớ
  chạy lệnh.
- **Job lập index không dùng `unique_key`.** Khoá chống trùng của job queue giữ cả job đã xong
  cho tới khi bị dọn, nên sẽ chặn lần lập index sau. Job lập index tự nhận ra khi không có gì
  đổi: đoạn có cùng SHA-256 và cùng mô hình thì không gọi embedding.
- **Ngưỡng thời gian tìm chấp nhận được: dưới 50 ms cho phần tính toán.** Đo được 8,3 ms với
  10.000 đoạn và 107 ms với 100.000 đoạn × 1.024 chiều, một nhân 2,1 GHz (`docs/BENCHMARKS.md`).
  Tức khoảng 40.000 đoạn mỗi ngôn ngữ, cỡ 7.000–8.000 bài. Vượt ngưỡng đó mới là lý do cho bước
  sau (vector `f16` / `int8`, nhiều nhân, ANN hay `pgvector` sau trait).
- **Máy chủ HTTP giả trong test của module `ai` là một handler không có router.** Luật
  `module-routes` cấm `.route(…)` và `.fallback(…)` trong module, kể cả trong test.

Đã cân nhắc:

- **Đặt biến AI của `media` và `content` dưới `AUTOXIDE_AI_`:** module `ai` phải biết nghiệp
  vụ.
- **Partial unique index cho yêu cầu dịch đang chạy:** cú pháp tạo khác nhau qua SeaORM, trong
  khi cột `active_key` đơn giản và đã kiểm trên cả hai database.
- **Trả 429 khi hết ngân sách tìm công khai:** người đọc không có kết quả nào.

## D-111 · Bước 18: sửa sau lượt rà soát độc lập — 2026-11-12 — agent quyết

Một agent khác đọc độc lập toàn bộ code bước 18. Mọi phát hiện đều kiểm lại với code và sửa,
mỗi cái kèm test khi kiểm được:

- **Yêu cầu dịch kẹt ở `running` mãi** (nặng). Worker cắt job sau 270 giây, còn một lần gọi AI
  được phép tới 600 giây, nên job bị cắt ngang mà không ai ghi kết quả. Sửa:
  - mỗi lần chạy job dịch tự giới hạn 240 giây; quá thì là lỗi tạm thời
    `content.translation_timed_out`, được thử lại;
  - yêu cầu đứng yên ở `running` quá 15 phút bị yêu cầu mới cùng ngôn ngữ thay chỗ
    (`error_code` `content.translation_stale`);
  - bước đánh dấu xong chỉ ghi khi yêu cầu còn `queued` / `running`, nên không ghi đè yêu cầu đã
    huỷ hay đã bị thay.
- **HTTP 400 / 413 / 422 của nhà cung cấp** thành `ai.request_rejected` (503), không còn là
  `internal_error`, và không bị thử lại như lỗi tạm thời. Không ghi thông báo của 401 / 403 vào
  log, vì có nhà cung cấp nhắc lại một phần key.
- **`translation_outdated`** so nội dung (snapshot) của bản gốc với revision đã dịch, không so
  `updated_at`. Đăng, gỡ hay xin duyệt không làm bản dịch thành "cũ".
- **Lập index:**
  - job kiểm lại dấu thời gian của mục trong transaction ghi; mục đã đổi thì không ghi vector
    của chữ cũ;
  - dòng `version` được tạo trước transaction, vì trên Postgres lỗi trùng khoá làm hỏng cả
    transaction;
  - job lập bù có `unique_key` theo ngày (mỗi mục tối đa một job mỗi ngày). Đây là điều chỉnh
    một phần D-110: job lập index thường vẫn không có khoá.
- **Mô tả `AUTOXIDE_CONTENT_PUBLIC_SEARCH_PER_MINUTE`** nói đúng là một ngân sách chung, đếm
  trong từng tiến trình.
- **Quyền:**
  - publish, unpublish, move, xin duyệt kiểm quyền sở hữu **trước** phiên bản (`If-Match`), để
    người chỉ có `_own` không dò được mục của người khác qua 412;
  - khôi phục revision không cần `content.read` nữa: quyền sửa mục, kể cả `content.write_own`,
    là đủ, khớp với tool MCP.
- **Dịch:**
  - tài khoản đã yêu cầu bị xoá thì `error_code` là `content.translation_canceled`;
  - thẻ rỗng `<mN></mN>` bị từ chối, để không mất link.

Để lại, ghi rõ:

- **Job dịch chết đúng giữa lúc ghi xong bản dịch có sẵn và lúc đánh dấu xong:** lần chạy lại
  báo `content.translation_conflict` dù bản dịch đã được ghi. Hiếm, và không mất dữ liệu.
- **Tool MCP `get_translation_request`** hiện với `content.read` hay `content.write_own`. Người
  chỉ có `content.write` dùng REST.

## D-112 · Ba module `forms`, `newsletter`, `comments`, sau phần nền chung trong core — 2026-11-12 — chủ dự án quyết

Bối cảnh: đề bài `docs/prompts/forms-newsletter-comments.md` và khảo sát
`docs/research/forms-newsletter-comments.md`. Cả ba module nhận dữ liệu từ người lạ không đăng
nhập, nên cần chung một nền (chống spam, token dùng một lần, IP người gọi, băm IP, giới hạn số
lần) mà core chưa có.

Quyết định (chủ dự án, ngày 12/11/2026):

1. Ba module nghiệp vụ mới, mỗi module một crate: `forms` (biểu mẫu), `newsletter` (bản tin
   email), `comments` (bình luận). Không gộp. Thứ tự ưu tiên khi phải chọn: `forms` →
   `newsletter` → `comments`.
2. Thứ dùng chung cho cả ba làm trong core, ở dạng trung lập, trước khi làm ba module — không
   module nào tự chế cách riêng.
3. Làm theo ba giai đoạn: nền chung tuần tự (bước 19); ba module song song, mỗi module một
   subagent; agent chính gộp (bước 20–22).
4. Yêu cầu pháp lý chưa chắc chắn (Nghị định 91/2020 về thư rác, Nghị định 147/2024 về bình
   luận, Luật Bảo vệ dữ liệu cá nhân 2025) làm thành cấu hình, không cài cứng; tài liệu nói rõ
   người cài phải tự kiểm văn bản gốc.
5. Người lạ cài là chạy được, không cần tài khoản dịch vụ bên thứ ba (CAPTCHA, Akismet chỉ là
   tuỳ chọn cắm thêm).

Không làm ở đợt này: tải file kèm bài gửi form, email tự trả lời người gửi form, form nhiều
bước, thanh toán, đo lượt mở / bấm link, trình soạn email kéo thả, reaction / vote, đăng nhập
mạng xã hội, phát hiện nội dung độc hại bằng AI, kết nối riêng từng nhà cung cấp email.

## D-113 · Khoá ký của kernel: `AUTOXIDE_SECRET_KEY`, không đặt thì sinh một lần và lưu database; module nội bộ `kernel` — 2026-11-12 — agent quyết

Bối cảnh: token form, đề chống spam, băm IP cần một khoá HMAC mà module nghiệp vụ lấy được
không qua `auth` (D-046). Khảo sát đề xuất "không đặt thì sinh ngẫu nhiên lúc khởi động" như
`AUTOXIDE_AUTH_TOKEN_SECRET`; đề bài chọn lưu database để khởi động lại và nhiều máy chủ vẫn
dùng chung khoá mà người cài không phải cấu hình gì.

Quyết định:

- `autoxide_kernel::secret::SecretKey`: khoá gốc là `AUTOXIDE_SECRET_KEY` (ít nhất 32 ký tự)
  nếu có; không thì đọc dòng `signing_key` của bảng `kernel_secrets`, chưa có thì sinh 48 byte
  ngẫu nhiên và chèn bằng `ON CONFLICT DO NOTHING` rồi đọc lại (hai máy chủ khởi động cùng lúc
  vẫn ra một khoá). Đọc database nhiều nhất một lần mỗi process.
- Module không dùng khoá gốc: `derive(purpose)` = HMAC-SHA256(khoá gốc, nhãn mục đích), 32 byte.
- Bảng `kernel_secrets` thuộc **module nội bộ `kernel`**, mà `Modules::new` luôn thêm vào đầu
  danh sách; tên `kernel` dành riêng. Module này công bố `SecretKey` qua `provide`; module khác
  lấy bằng `context.require::<SecretKey>()`. Nhờ là một module bình thường, luật sở hữu bảng,
  `migrate`, `backup` / `restore` / `copy` áp cho nó không cần code riêng (khoá đi theo backup).
- `random_token` và `sha256` có bản trong kernel (`secret`); `auth` chuyển sang dùng ở bước token
  dùng một lần.
- Không thứ gì sống lâu được phụ thuộc khoá này.

Lý do: người lạ cài là chạy được, nhiều máy chủ đúng ngay; không thêm cấu hình bắt buộc.

Đã cân nhắc: sinh lúc khởi động (token form hỏng khi khởi động lại và giữa các máy chủ); bảng
khoá riêng cho từng module (endpoint chống spam trung lập của core và module kiểm token phải dùng
chung một khoá); thêm tham số khoá vào `Modules::start` (đổi chữ ký mọi chỗ gọi, trong khi
`provide` đã có sẵn).

## D-114 · Địa chỉ client: đếm proxy tin cậy (`AUTOXIDE_HTTP_TRUSTED_PROXY_COUNT`), `ClientInfo` dùng chung — 2026-11-12 — agent quyết

Bổ sung D-079. Bối cảnh: hàm lấy địa chỉ client nằm riêng trong rate limiter; chỉ đúng khi có
đúng một proxy. Website server-side (Next.js) đứng sau CDN gọi API thì có hai, ba proxy, và
mọi khách sẽ trông như một client.

Quyết định:

- Lớp HTTP `client::attach` giải địa chỉ **một lần** cho mỗi request; rate limiter và extractor
  `autoxide_kernel::http::ClientInfo` (IP, user agent, `Origin`, `Referer`, cắt ở 1024 ký tự)
  dùng chung kết quả đó.
- Mọi dòng của header cấu hình gộp thành một danh sách theo thứ tự (RFC 9110); địa chỉ là phần
  tử thứ `AUTOXIDE_HTTP_TRUSTED_PROXY_COUNT` (1–10, mặc định 1) tính từ cuối. Mặc định 1 giữ
  đúng hành vi của D-079 (phần tử cuối của dòng cuối). Danh sách ngắn hơn số proxy thì lấy
  phần tử đầu; giá trị không phải IP thì dùng địa chỉ kết nối kèm cảnh báo trong log.
- `docs/DEPLOY.md` có sơ đồ trình duyệt → CDN → máy chủ website → proxy → Autoxide.

Đã cân nhắc: liệt kê dải địa chỉ proxy tin cậy (CIDR) — chắc hơn khi địa chỉ proxy cố định, nhưng
CDN đổi dải địa chỉ thường xuyên; để sau. Tin header do website gửi kèm khi gọi bằng API key —
hai cách cho cùng một việc (khảo sát 0.15), không làm.

## D-115 · Băm IP: gộp /24 – /64 rồi HMAC; `bucket` theo ngày, `fingerprint` cố định — 2026-11-12 — agent quyết

Quyết định: `autoxide_kernel::ip_hash`. Gộp địa chỉ theo khối (IPv4 /24, IPv6 /64; IPv4 viết
dạng `::ffff:` tính như IPv4) rồi HMAC-SHA256 bằng khoá dẫn từ `SecretKey` (D-113), giữ 16 byte,
base64url 22 ký tự. `bucket(secret, ip, now)` dùng khoá đổi theo ngày UTC, cho bộ đếm;
`fingerprint(secret, ip)` dùng khoá cố định, lưu cùng bản ghi, module tự xoá sau thời hạn của
mình. Không module nào lưu IP thô.

Lý do: không gian IPv4 nhỏ, băm không khoá thì thử hết được; khoá theo ngày làm bộ đếm các ngày
không nối được với nhau. Gộp khối vì một nhà mạng cấp cả khối cho một khách.

Đã cân nhắc: cắt bớt octet cuối mà không băm (Isso) — vẫn lộ khối địa chỉ; băm từng địa chỉ
không gộp — đổi địa chỉ trong khối IPv6 là thoát bộ đếm.

## D-116 · `Throttle` dùng chung trong kernel, đếm trong database; `auth` chuyển sang — 2026-11-12 — agent quyết

Quyết định: `autoxide_kernel::throttle::Throttle` tổng quát từ `LoginThrottle` của `auth`:
`check` (không đếm), `hit` (đếm trước rồi xét — D-079), `record`, `refund`, `clear`, `purge`.
Cửa sổ cố định `window`; vượt `limit` thì chặn trong `lock` (mặc định bằng `window`) rồi đếm
lại. Bảng `<module>_throttle(key, count, window_started_at, locked_until)` sinh bằng
`throttle_migration`, module tự thêm như `jobs_migration`. Khoá đếm băm SHA-256 trước khi
lưu. Mọi bước là `UPDATE` có điều kiện hoặc `INSERT … ON CONFLICT DO NOTHING`: chạy như nhau
trên hai database và đúng khi nhiều máy chủ.

`auth` chuyển sang bằng migration m0018 (tạo `auth_throttle`, bỏ `auth_login_throttle`; số đếm
cũ không chép vì chỉ sống một cửa sổ). Mã lỗi `auth.login_throttled` và thời gian chờ không đổi.

Đã cân nhắc: rate limiter trong bộ nhớ (GCRA) cho việc này — mất khi khởi động lại, mỗi máy chủ
đếm riêng; cửa sổ trượt — cần lưu từng lần, tốn bảng hơn mà lợi ích nhỏ ở các giới hạn thô
(số lần gửi form mỗi giờ).

## D-117 · Token dùng một lần trong kernel (`OneTimeTokens`) — 2026-11-12 — agent quyết

Quyết định: `autoxide_kernel::tokens::OneTimeTokens` trên bảng `<module>_tokens(id, purpose,
subject, data, token_hash UNIQUE, expires_at, used_at, created_at)` sinh bằng
`tokens_migration`. `issue` (32 byte ngẫu nhiên, chỉ lưu SHA-256), `recent` (token đã phát cho
một đối tượng từ một mốc — bên gọi tự quyết thời gian chờ gửi lại, kể cả theo `data`),
`consume` (một câu `UPDATE … WHERE token_hash = ? AND purpose = ? AND used_at IS NULL AND
expires_at > now`, đúng một bên thắng khi đua), `revoke`, `purge`. Mọi hàm nhận kết nối, để phát
token cùng transaction với thay đổi. `subject` là chuỗi (ID của bất kỳ đối tượng nào), `data`
là chuỗi tuỳ chọn (vd địa chỉ email đang xác minh).

`auth` (đặt lại mật khẩu, xác minh email) chuyển sang ở commit sau, hành vi không đổi.

Lý do: `newsletter` (xác nhận hai bước), `comments` (xác minh email khách) cần đúng luồng mà
`auth` đang có; module nghiệp vụ không được phụ thuộc `auth` (D-046).

## D-118 · `auth` dùng token và hàm băm của kernel; token xoá cùng tài khoản — 2026-11-12 — agent quyết

Quyết định: đặt lại mật khẩu (`password_reset`) và xác minh email (`email_verification`, `data`
là địa chỉ đang xác minh) chuyển sang `OneTimeTokens` (D-117) trên bảng `auth_tokens`.
Migration m0019 chép các token còn hạn, chưa dùng từ hai bảng cũ (link trong mail đã gửi vẫn
dùng được) rồi bỏ hai bảng đó. Hai bảng cũ có foreign key `ON DELETE CASCADE` tới `auth_user`;
`auth_tokens` thì không (bảng của kernel, `subject` là chuỗi), nên `UserService::delete` gọi
`OneTimeTokens::forget` trong cùng transaction — xoá tài khoản vẫn xoá token như trước.
`random_token` / `sha256` của `auth` là bản của kernel (`autoxide_kernel::secret`), không còn bản
riêng. Thời gian chờ gửi lại, thời hạn token, mã lỗi giữ nguyên.

## D-119 · Chống spam hai pha trong core: module nội bộ `antispam`, `SpamGuard`, nonce là idempotency cho người lạ — 2026-11-12 — agent quyết

Bối cảnh: `forms`, `newsletter`, `comments` nhận dữ liệu từ người lạ; core bỏ qua
`Idempotency-Key` khi request không xác thực. Đề bài: ba kết luận nhận / cách ly / từ chối, hai
pha, không cần tài khoản bên thứ ba.

Quyết định:

- Module nội bộ thứ hai **`antispam`** (luôn đăng ký, sau `kernel`; tên dành riêng): đọc
  `AUTOXIDE_ANTISPAM_*`, công bố `SpamGuard`, phục vụ `GET /api/v1/antispam/token?scope=` (công
  khai, `Cache-Control: no-store`). Không có bảng: bảng nằm ở module dùng nó.
- **Token form** ký HMAC bằng khoá dẫn từ khoá kernel (D-113): phạm vi, lúc phát, nonce 16 byte.
  Tuổi tối đa `AUTOXIDE_ANTISPAM_TOKEN_MAX_AGE_MINUTES` (120), tối thiểu
  `AUTOXIDE_ANTISPAM_MIN_FILL_SECONDS` (3).
- **Pha một** `SpamGuard::check` (trước transaction, mọi việc gọi mạng ở đây): trường bẫy, token,
  đề (`ChallengeVerifier`, mặc định `NoChallenge`), giới hạn theo khối IP băm theo ngày (D-115),
  theo email, theo người dùng (`Limits`, bảng `<module>_throttle`, D-116), trùng (module tự dò
  bằng `content_hash` trên bảng của mình rồi báo cờ `duplicate`), bộ phân loại
  (`ContentClassifier`, mặc định `NoClassifier`).
- **Kết luận** (`Verdict`) có thêm `Discard` bên cạnh `Accept` / `Quarantine`, và "từ chối" là
  `Err(Error)`. Với người lạ, kết luận không bao giờ lộ cho bot (đề bài `forms`: bẫy, quá nhanh,
  quá nhiều lần trả cùng một câu trả lời): bẫy và vượt giới hạn → `Discard` (trả lời như đã
  nhận, không lưu); quá nhanh, trùng, bị phân loại spam, nhà cung cấp lỗi khi
  `AUTOXIDE_ANTISPAM_FAIL_MODE=quarantine` → `Quarantine` (lưu, giữ chờ duyệt). Chỉ token thiếu /
  sai / hết hạn, đề sai, nhà cung cấp lỗi khi `FAIL_MODE=reject` (503) là lỗi. Vì vậy core
  không có mã `antispam.too_fast`; `antispam.rate_limited` (429) chỉ dành cho người gọi đã đăng
  nhập (API key của website, MCP), là người cần biết lý do.
- **Pha hai** `Checked::claim` trong transaction của module: chèn băm nonce kèm kết quả (vd ID
  bản ghi) vào `<module>_antispam_nonces` (sinh bằng `antispam_migration`). Đụng unique →
  `Claim::Replay { result }`: module rollback và trả lại kết quả lần đầu — idempotency cho người
  lạ, chạy như nhau trên hai database.
- Người gọi đã đăng nhập bỏ qua bẫy, token, đề; vẫn bị giới hạn theo người dùng; idempotency của
  họ là `Idempotency-Key`.

Đã cân nhắc: đặt endpoint token ở module `kernel` (đường dẫn `/api/v1/kernel/…` khó hiểu) hay
ghép thẳng vào `http::build` (endpoint không thuộc module nào, cấu hình đọc ở chỗ khác); trả
429 / mã lỗi cho người lạ (bot đọc được và né); lưu bài trúng bẫy (bảng phình vì rác).

## D-120 · ALTCHA v2 tự viết, phiên bản giao thức và widget được ghim — 2026-11-12 — agent quyết

Quyết định: `autoxide_kernel::antispam::Altcha` (bật bằng `AUTOXIDE_ANTISPAM_CHALLENGE=altcha`,
mặc định tắt) cài phần kiểm của giao thức ALTCHA Proof-of-Work v2 bằng `hmac` / `sha2` đang có
(~150 dòng). Không thêm crate `altcha` vì nó ghim `hmac 0.12` / `sha2 0.10`, kéo vào bộ thư viện
mã hoá thứ hai. Ghim:

- thuật toán `SHA-256` lặp `AUTOXIDE_ANTISPAM_ALTCHA_COST` vòng (mặc định 500, chưa đo trên
  điện thoại — ghi rõ trong mô tả biến), bộ đếm `uint32` big-endian, `keyLength` 32;
- chế độ deterministic: máy chủ chọn bộ đếm ngẫu nhiên trong [32, 64], `keyPrefix` là 16 byte
  đầu khoá của bộ đếm đó; không dùng chữ ký khoá (đường kiểm nhanh) — máy chủ dựng lại khoá từ
  bộ đếm (chi phí bằng một lần thử);
- chữ ký HMAC-SHA256 (hex) trên JSON của `parameters` sắp khoá ở mọi tầng; khoá ký dẫn từ khoá
  kernel (D-113); đề sống 10 phút;
- `data` mang phạm vi form và `binding` = nonce của token form: lời giải chỉ dùng được với đúng
  token đó, mà token chỉ được nhận một lần (D-119) — đây là cách chống dùng lại đề mà thư viện
  chính thức không có;
- widget: `altcha@3.5.0` (npm); website đưa `challenge` từ `GET /api/v1/antispam/token` cho
  widget (`widget.configure({ challenge })`) và gửi giá trị ô ẩn `altcha` (base64) kèm form.

Kiểm: vector sinh một lần bằng thư viện chính thức `altcha` 1.1.0 (`create_challenge` +
`solve_challenge`, commit ở `crates/kernel/src/antispam/fixtures/altcha_v2.json`): chữ ký và lời
giải khớp; cộng test từ chối khi hết hạn, sai khoá, sai bộ đếm, đổi tham số, sai form, sai token.

## D-121 · `Mail` có `reply_to`, header theo danh sách cho phép, khoá chống trùng; lỗi vĩnh viễn thành event — 2026-11-12 — agent quyết

Quyết định:

- `autoxide_kernel::mail::Mail` thêm `reply_to`, `headers` (chỉ `List-Unsubscribe`,
  `List-Unsubscribe-Post`, `List-Id`, `Auto-Submitted`; giá trị một dòng, ≤ 998 ký tự — chặn
  chèn header bằng CR/LF), `dedupe_key`; có `Default`, dựng bằng `..Mail::default()` nên thêm
  trường sau này không làm hỏng chỗ gọi. `Mail::header_problems` kiểm; module `mail` kiểm khi
  xếp hàng và báo mọi lỗi một lần.
- `dedupe_key` thành `unique_key` của job `mail.send` (`mail:<khoá>`): xếp hàng lại cùng khoá
  không gửi lần hai (vd lượt gửi bản tin chạy lại).
- Lỗi SMTP **vĩnh viễn** (5xx, theo `lettre::transport::smtp::Error::is_permanent`) không thử
  lại: job xong, module `mail` phát event trung lập `mail.delivery_failed`
  (`{"to", "dedupe_key", "code"}`, tên ở `autoxide_kernel::mail::DELIVERY_FAILED`) qua outbox
  của mình (`mail_events`, migration m0002). Lỗi tạm thời (4xx, mạng) vẫn thử lại như cũ.
  Event mang địa chỉ người nhận vì bên nghe (danh sách chặn của bản tin) cần nó; nó cũng đi qua
  webhook như mọi event.
- Không làm `send_batch` (khảo sát gợi ý): chưa module nào cần gửi một lần nhiều mail; bản tin
  xếp từng mail vào hàng đợi theo lô của nó.

## D-122 · Người thực hiện "người lạ" (`Actor::public`), cột `actor_kind` trong outbox và audit — 2026-11-12 — agent quyết

Bối cảnh: event do người lạ gây ra (gửi form, đăng ký bản tin, bình luận của khách) trước đây
chỉ ghi được như hệ thống (`actor_user_id` trống) — audit không phân biệt được.

Quyết định:

- `Actor` thêm `public: bool` (mặc định `false`), `Actor::public()`, `Actor::kind()` →
  `system` / `user` / `public`.
- Kernel thêm `event::outbox_actor_migration(name, module)`: cột `actor_kind` (chữ, có thể
  trống) trong `<module>_events`; outbox ghi nó, bộ phát đọc lại. Mọi module có outbox thêm
  migration này (auth m0020, notes m0003, mail m0003, media m0006, content m0007 (sau m0004–m0006 của `main`), files m0003,
  webhooks m0004); module mới thêm ngay sau `outbox_migration`. Không sửa `outbox_migration` vì
  migration đã chạy thì không đổi được.
- `audit_entry` thêm cột `actor_kind` (m0002); `GET /api/v1/audit` trả `actor_kind` (dòng cũ hiểu
  theo `actor_user_id`). Webhook gửi `data.actor.kind`.

Đã cân nhắc: một UUID đặc biệt trong `actor_user_id` thay cho cột mới — rẻ hơn nhưng là giá trị
"thần kỳ", người đọc audit dễ hiểu nhầm là một người dùng.

## D-123 · Sổ đối tượng trung lập `ResourceDirectory`; `content` đăng ký `content.entry` — 2026-11-12 — agent quyết

Quyết định: `autoxide_kernel::resource` — sổ giống sổ dữ liệu cá nhân. Module đăng ký loại đối
tượng `<module>.<loại>` kèm `ResourceResolver` bằng `StartContext::resource_type`; module khác lấy
sổ bằng `StartContext::resources()` và gọi `resolve(loại, id)` → `Option<Resource { public,
title, summary, url, locale, owner_user_ids }>` (`url` là đường dẫn không có tên miền). Kernel
chỉ biết chuỗi loại, nên vẫn trung lập nghiệp vụ; không module nào phụ thuộc thẳng `content`
hay đọc bảng của nó.

`content` đăng ký `content.entry` (ID là UUID của mục): đang công khai thì trả tiêu đề, tóm tắt,
đường dẫn, ngôn ngữ của bản công khai (`public: true`); chỉ có nháp thì `public: false`, không
đường dẫn; `owner_user_ids` là người tạo mục.

Đã cân nhắc: để `comments` / `newsletter` phụ thuộc `autoxide_content::api` — gắn chặt hai module
vào `content`, trong khi bình luận gắn được vào đích bất kỳ.

## D-124 · `AUTOXIDE_HTTP_PUBLIC_URL`, công bố bởi module nội bộ `kernel` — 2026-11-12 — agent quyết

Quyết định: URL công khai của API (không bắt buộc) do module nội bộ `kernel` đọc và công bố
`autoxide_kernel::public_url::PublicUrl` (`context.require::<PublicUrl>()`). `link(path)` ghép
link tuyệt đối; chưa đặt thì lỗi 503 `public_url_missing` nói rõ phải đặt biến nào — chỉ chức
năng cần link tuyệt đối (vd header `List-Unsubscribe` của bản tin) bị ảnh hưởng, chương trình vẫn
khởi động. Giá trị phải là địa chỉ `https://` (hoặc `http://`) tuyệt đối, không query, không
fragment; dấu `/` cuối bị bỏ.

Lý do: module không đọc cấu hình HTTP của binary; công bố qua `provide` như `SecretKey` là một
cách làm sẵn có.

## D-125 · Module `forms`: bảng, trường có phiên bản, đường dẫn, permission — 2026-11-12 — agent quyết

Quyết định:

- Ba bảng: `forms_forms` (định nghĩa hiện hành; cột JSON `fields`, `allowed_origins`,
  `success_message`, `notify_emails` lưu dạng chữ), `forms_form_versions` (ảnh chụp `fields`, unique
  `(form_id, version)`), `forms_submissions`. Cộng các bảng sinh từ kernel: `forms_events` (+ cột
  `actor_kind`), `forms_jobs`, `forms_throttle`, `forms_antispam_nonces`. Không foreign key, kể cả
  trong module: xoá form thì service xoá phiên bản và bài gửi cùng transaction.
- `forms_forms.version` bắt đầu từ 1 và chỉ tăng khi danh sách trường thật sự đổi (so sau khi chuẩn
  hoá); mỗi lần tăng chụp một dòng `forms_form_versions`. Bài gửi giữ `form_version`, đọc ra với
  trường và nhãn của đúng phiên bản đó (`answers[]` trong API). Vì có ảnh chụp nên KHÔNG khoá `key`
  của trường khi đã có bài gửi (báo cáo nghiên cứu đề xuất khoá) — bài cũ đọc theo ảnh chụp của nó;
  CSV lấy cột theo trường hiện hành rồi tới trường chỉ có ở phiên bản cũ.
- Trạng thái form `open` / `closed` (theo đề bài D, không theo báo cáo `draft/published/archived`);
  không có bước "publish". Form đóng: `GET /public/{key}` vẫn trả (kèm `status: closed`) để website
  báo đã đóng; gửi bài thì 409 `forms.closed`.
- Kiểu trường đóng: `text`, `email`, `phone`, `textarea`, `number`, `select`, `radio`, `checkbox`,
  `date`, `hidden`, `consent`. KHÔNG làm `message` (khối chỉ hiển thị) của báo cáo — đề bài D không có.
  Nhãn / gợi ý / placeholder / nhãn lựa chọn là `{en, vi}`, thiếu ngôn ngữ nào thì dùng ngôn ngữ kia.
  Mỗi form nhiều nhất một trường `consent`.
- Lỗi câu trả lời dùng mã chung `validation_failed`, mỗi lỗi một `FieldError` đường dẫn
  `data.<key>`, mã trường: `required`, `consent_required`, `type`, `length`, `email`, `phone`,
  `option`, `range`, `date`, `invalid` (ký tự điều khiển), `unknown` (trường lạ). Lỗi định nghĩa
  cũng `validation_failed`, đường dẫn `fields[i].…`. Không thêm mã `forms.validation_failed` riêng
  như báo cáo đề xuất: kernel đã có một cách báo lỗi trường.
- `number` nhận số JSON hoặc chuỗi số; `min` / `max` của trường là `f64` (không phải tiền — luật
  "tiền là số nguyên" không áp). Số nguyên được lưu dạng nguyên.
- Đường dẫn: quản trị `GET/POST /forms`, `GET/PATCH/DELETE /forms/{id}`, `GET /forms/{id}/submissions`,
  `GET /forms/{id}/submissions/export?format=csv`, `GET/PATCH/DELETE /forms/submissions/{id}`;
  xuất / xoá theo email `GET/DELETE /forms/data?email=` (theo hợp đồng chung §3, thay cho
  `/submissions?email=` của đề bài D); công khai `GET /forms/public/{key}` và
  `POST /forms/public/{key}/submissions`.
- `If-Match` bắt buộc với `PATCH` / `DELETE` form và `PATCH` bài gửi (endpoint sửa mới, D-084);
  `DELETE` một bài gửi không cần (xoá là xoá, không có gì để ghi đè).
- Xoá form xoá luôn mọi bài gửi của nó (event `forms.form_deleted` có `submissions_deleted`);
  muốn giữ bài thì đóng form. Đã cân nhắc: chặn xoá khi còn bài (409) — buộc quản trị viên xoá từng
  bài, không giúp gì cho quyền riêng tư.
- Permission theo đề bài D: `forms.read` (form và bài gửi), `forms.manage`, `forms.review` (đổi trạng
  thái, xoá một bài), `forms.export` (CSV, xuất / xoá theo email). Không tách
  `forms.submissions.*` như báo cáo.
- Danh sách bài gửi mặc định bỏ `spam` (hộp thư); `status=spam` mới thấy spam.
- Tool MCP: `forms.list`, `forms.get`, `forms.list_submissions`, `forms.get_submission`,
  `forms.set_submission_status` (nhận `version` như `If-Match`). Không có tool tạo / sửa form ở bản
  đầu (đề bài D chỉ yêu cầu năm tool này) và không có tool gửi form.

Lý do: đề bài (mục D) và hợp đồng chung là nguồn chính; chỗ khảo sát khác đề bài thì theo đề bài.
Ảnh chụp phiên bản giữ bài cũ đọc được mà không phải khoá trường.

## D-126 · Bài gửi công khai của `forms`: `Origin`, email người gửi, bằng chứng đồng ý, trùng, giới hạn — 2026-11-12 — agent quyết

Quyết định:

- `allowed_origins` rỗng = mọi nơi được gửi (người lạ cài xong chạy được ngay). Có danh sách thì
  request có `Origin` không nằm trong đó bị 403 `forms.origin_not_allowed`; request không có
  `Origin` (máy chủ gọi) không bị kiểm. Origin lưu dạng `scheme://host[:port]` viết thường.
- Email người gửi (cột `email`, trim + chữ thường, có index): trường `reply_to_field` nếu có trả
  lời, không thì trường `email` đầu tiên có trả lời. `reply_to_field` phải trỏ tới một trường
  `email` của form.
- Bằng chứng đồng ý: máy chủ tự lấy chữ của ô `consent` theo ngôn ngữ đã gửi (`locale`) từ định
  nghĩa — không nhận chữ do client gửi (hợp đồng chung §3 nói "client gửi kèm"; với form máy chủ biết
  chính xác chữ đã phát ra nên đáng tin hơn). Cột `consent` =
  `{text, version: "<phiên bản form>", at, source: "forms:<key>", field, ip_fingerprint}`; chỉ có khi
  ô được tích.
- Dò trùng: cùng form, cùng `content_hash` của câu trả lời đã chuẩn hoá, trong 24 giờ → báo
  `duplicate` cho `SpamGuard` (bài bị giữ ở `spam`).
- `page_url` lấy từ thân; không gửi thì lấy header `Referer` nếu là địa chỉ `http(s)` (header lạ
  không thành lỗi 422). `referrer` chỉ từ thân (`document.referrer`).
- Câu trả lời công khai: 201 `Data{id, message}` + `Location: /api/v1/forms/submissions/{id}` cho
  nhận, giữ lại và bỏ đi (bỏ đi thì ID giả UUIDv7). Theo D-119, token thiếu / sai / hết hạn vẫn là lỗi
  400 của `SpamGuard` (hợp đồng chung §6 liệt "token thiếu / sai" vào nhóm trả lời giống bài nhận,
  nhưng §2 và kernel coi đó là lỗi — giữ theo kernel; test chỉ kiểm bẫy / quá nhanh / quá nhiều).
- Giới hạn mặc định (cấu hình, form ghi đè được hai cái đầu): 10 bài / giờ / khối IP / form, 5 bài
  / giờ / email / form, 1000 bài / giờ / người gọi đã đăng nhập (máy chủ website gửi thay mọi khách
  bằng một API key nên ngưỡng cao). Đếm theo email nghĩa là gửi lại cùng token sau khi đã vượt
  ngưỡng nhận ID giả khác ID lần đầu — chấp nhận được (bot mới vượt ngưỡng).
- Kích thước thân của `POST /public/{key}/submissions` giới hạn bằng `Module::body_limits` theo
  `AUTOXIDE_FORMS_MAX_SUBMISSION_BYTES` (mặc định 64 KiB); service kiểm thêm cỡ JSON của câu trả lời.

Lý do: hợp đồng chung §2–§3 và D-119. Máy chủ biết chính xác chữ đồng ý đã phát ra nên lấy từ định
nghĩa đáng tin hơn chữ do client gửi; hợp đồng chung được sửa cho khớp (forms lấy từ định nghĩa,
module không có định nghĩa form thì nhận từ client).

## D-127 · `forms`: mail báo quản trị viên qua event, event không mang câu trả lời, thời hạn giữ, CSV tự viết — 2026-11-12 — agent quyết

Quyết định:

- Mail báo đi theo event: module tự nghe `forms.submission_received` của mình và gọi `dyn Mailer`
  (mail vào job queue của module `mail`). Không có job riêng trong `forms_jobs`: `Mailer::send` đã là
  hàng đợi bền, và event ghi cùng transaction với bài nên không sót. `dedupe_key` =
  `forms.notify:<id bài>:<sha256(địa chỉ nhận)>` (băm để vừa cột `unique_key` 200 ký tự) — event giao
  lại không sinh mail thứ hai. `Reply-To` = email người gửi. Ngôn ngữ mail theo
  `AUTOXIDE_FORMS_NOTIFY_LANGUAGE` (mặc định `en`). Mail chứa câu trả lời (chỉ gửi quản trị viên).
- Chuyển bài từ `spam` ra trạng thái khác phát thêm `forms.submission_received` (bài lọt bộ lọc
  thì quản trị viên vẫn được báo); bài spam không bao giờ phát event này.
- Event: `forms.form_created`, `forms.form_updated`, `forms.form_deleted`,
  `forms.submission_received`, `forms.submission_status_changed`, `forms.submission_deleted`,
  `forms.submissions_erased` (xoá theo email: chỉ ID và số lượng, không email),
  `forms.submissions_expired` (job dọn: form, số lượng, lý do `spam` / `retention`). Hình dạng
  `{before, after}`; tóm tắt form không có `notify_emails`, tóm tắt bài chỉ có `id`, `form_id`,
  `form_key`, `status`.
- Job `forms.cleanup` mỗi ngày: xoá spam quá `AUTOXIDE_FORMS_SPAM_RETENTION_DAYS` (30), xoá bài quá
  `retention_days` của form (`null` → `AUTOXIDE_FORMS_RETENTION_DAYS`, mặc định 0 = giữ mãi — an toàn
  cho dữ liệu; người cài tự đặt theo luật áp dụng), xoá `ip_fingerprint` quá
  `AUTOXIDE_FORMS_IP_RETENTION_DAYS` (90), dọn nonce và bộ đếm hết hạn. `ip_fingerprint` nằm trong
  bằng chứng đồng ý thì giữ cùng bằng chứng (mục đích khác: chứng minh đồng ý).
- Thêm hai cấu hình ngoài đề bài: `AUTOXIDE_FORMS_MAX_FIELDS` (50) và
  `AUTOXIDE_FORMS_IP_RETENTION_DAYS` (hợp đồng chung §3 bắt xoá dấu vết IP theo thời hạn), cùng ba
  giới hạn số lần và `AUTOXIDE_FORMS_NOTIFY_LANGUAGE`. Bỏ `AUTOXIDE_FORMS_STORE_IP` của báo cáo:
  chỉ lưu dấu vết băm, có hạn xoá.
- CSV tự viết (`csv.rs`, vài chục dòng): mọi ô trong ngoặc kép, ô bắt đầu bằng `=`, `+`, `-`, `@`,
  tab, CR thêm `'`, BOM UTF-8, CRLF. Không thêm thư viện. Xuất đồng bộ (đọc theo trang 200 dòng);
  xuất bằng job ghi vào `Storage` để sau (báo cáo cũng để sau).

Lý do: event ghi cùng transaction với bài nên mail không sót; `dedupe_key` của D-121 chống mail
trùng khi event giao lại (D-054). CSV vài chục dòng tự viết đủ, không thêm thư viện.

## D-128 · Tool MCP tạo / sửa form; không có tool xoá form; khoá phiên bản tên `etag` — 2026-11-12 — agent quyết

Quyết định:

- Thêm `forms.create` và `forms.update` (permission `forms.manage`), gọi đúng `create_form` /
  `update_form` của service như REST. Danh sách trường và `success_message` khai trong JSON Schema
  là object chung (tool MCP không có `$ref`); tool đọc thành kiểu thật, sai hình dạng thì báo
  `validation_failed` theo đường dẫn (`fields[1]`), mô tả của tool nêu đủ hình dạng một trường.
- Khoá chống ghi đè của form trong MCP tên `etag` (đầu ra của `forms.get` / `create` / `update`,
  đầu vào bắt buộc của `update`), không phải `version` như tool của `content` và tool đổi trạng
  thái bài gửi: form đã có trường `version` (đếm số lần đổi danh sách trường), hai khoá cùng tên
  thì một khoá bị đè.
- Không có tool xoá form: xoá form xoá luôn mọi bài gửi, không hoàn tác được. Agent đóng form bằng
  `forms.update` (`status: closed`); xoá vẫn làm được qua REST. MCP làm ít hơn REST là được, luật
  kiến trúc 8 chỉ cấm làm nhiều hơn.

Lý do: chủ dự án yêu cầu agent quản lý được form qua MCP (12/11/2026); việc phá huỷ hàng loạt thì
giữ cho người làm qua REST.

## D-129 · `newsletter`: bảng, trạng thái, không foreign key — 2026-11-12 — agent quyết

Quyết định:

- Sáu bảng: `newsletter_lists`, `newsletter_subscribers` (email unique, đã chuẩn hoá),
  `newsletter_subscriptions` (khoá chính `(subscriber_id, list_id)`), `newsletter_campaigns`
  (unique `(source, source_ref)`), `newsletter_deliveries` (unique `(campaign_id,
  subscriber_id)`, unique `unsubscribe_token_hash`), `newsletter_suppressions` (email unique).
  Cộng các bảng của kernel theo thứ tự của hợp đồng: events (+ `actor_kind`), jobs, throttle,
  antispam nonces, tokens. Không foreign key, kể cả trong module: service xoá dòng con cùng
  transaction.
- Trạng thái đúng hợp đồng §4: đăng ký `unconfirmed` / `confirmed` / `unsubscribed`; người đăng
  ký `active` / `blocked`; chiến dịch `draft` / `scheduled` / `sending` / `paused` / `sent` /
  `cancelled`. Lượt gửi `pending` / `sent` / `failed` / `skipped` (đề bài E). Lý do chặn:
  `hard_bounce`, `complaint`, `manual` (đề bài E) và thêm `soft_bounce` (soft bounce tới ngưỡng)
  để không ghi sai thành `hard_bounce`.
- Thêm cột ngoài bảng của đề bài: `subscriptions.hold_reason` (lý do `Quarantine`, như
  `hold_reason` của comments — hợp đồng §4), `subscribers.soft_bounces`,
  `campaigns.locale` (ngôn ngữ chân thư), `campaigns.run` và `prepared_at` (D-132),
  `deliveries.claimed_at`, `lists.new_post_locale` (D-135).
- Số đếm của chiến dịch **không lưu** trong `newsletter_campaigns` mà đếm từ
  `newsletter_deliveries` khi đọc (`GROUP BY status`, có index `(campaign_id, status, id)`):
  lưu bộ đếm thì mỗi lượt gửi thêm một lần ghi tranh nhau trên cùng một dòng (SQLite một người
  ghi), và dễ lệch.

Đã cân nhắc: bảng nhật ký đồng ý chỉ thêm và bảng `feedback_events` của khảo sát — đề bài E không
có; bằng chứng đồng ý nằm ở cột `consent` của đăng ký (hợp đồng §3), bounce ghi vào danh sách chặn.

## D-130 · `newsletter`: token huỷ đăng ký: riêng cho từng mail, không phải từng đăng ký — 2026-11-12 — agent quyết

Bối cảnh: đề bài E nói "token ngẫu nhiên riêng của từng đăng ký, lưu băm, không hết hạn (không
phụ thuộc khoá ký)". Nếu chỉ lưu bản băm thì máy chủ không có lại token gốc để đặt vào các mail
sau; muốn có thì phải lưu token gốc (trái "lưu băm") hoặc dẫn từ khoá (trái "không phụ thuộc
khoá ký"), hoặc sinh token mới mỗi lần gửi và xoá token cũ (link trong mail cũ chết — trái "không
hết hạn").

Quyết định:

- Mỗi mail chiến dịch có token ngẫu nhiên 32 byte riêng (`secret::random_token`), chỉ lưu
  SHA-256 ở `newsletter_deliveries.unsubscribe_token_hash`. Token trỏ tới người đăng ký và danh
  sách của chiến dịch → huỷ đúng danh sách của mail đó; trang lựa chọn đọc / sửa mọi danh sách
  của người đó. Không hết hạn, không phụ thuộc khoá — đúng ý đề bài, chỉ khác đơn vị (mail thay
  vì đăng ký). Đây cũng là cách của khảo sát (mẫu Listmonk).
- Vì token nằm ở lượt gửi: chiến dịch đã gửi không xoá được (`newsletter.campaign_not_deletable`);
  xoá dữ liệu của một người thì token trong mail cũ của họ hết tác dụng (dữ liệu đã xoá).
- Mail gửi thử dùng token ngẫu nhiên không lưu: link không làm gì.

## D-131 · `newsletter`: huỷ một chạm nhận thân form-urlencoded — ngoại lệ của `ValidJson` — 2026-11-12 — agent quyết

Quyết định: `POST /api/v1/newsletter/public/unsubscribe?token=…` nhận thân bất kỳ (thường là
`List-Unsubscribe=One-Click`, `application/x-www-form-urlencoded`, RFC 8058) và bỏ qua nó — token
nằm ở query của URL trong header `List-Unsubscribe`. Không dùng `ValidJson` vì nhà cung cấp hộp
thư không gửi JSON. Câu trả lời luôn `200 {"data": {"unsubscribed": true}}`, kể cả token lạ (không
lộ gì). Có hiệu lực ngay trong request. `GET` không đổi dữ liệu: trang lựa chọn đọc bằng `GET
/public/preferences?token=`, sửa bằng `PUT`.

Đã cân nhắc: `axum::Form` (từ chối 415 khi thiếu / sai content type — nhà cung cấp lạ sẽ hỏng
lặng lẽ); đặt token trong đường dẫn (`/unsubscribe/{token}`) — log của kernel ghi đường dẫn
nhưng không ghi query string, nên token để ở query.

## D-132 · `newsletter`: gửi chiến dịch: hai loại job, nhận dòng bằng `UPDATE` có điều kiện, nhiều nhất một lần — 2026-11-12 — agent quyết

Quyết định:

- `newsletter.prepare` chụp người nhận: keyset theo `subscriber_id` trên các đăng ký `confirmed`,
  500 dòng mỗi transaction, chèn `ON CONFLICT DO NOTHING`; chạy quá 60 giây thì đưa job tiếp theo
  (khoá `newsletter.prepare:<id>:<run>:<sau-người>`) vào hàng đợi. Xong thì ghi `prepared_at` và
  đưa lô 0 vào hàng đợi.
- `newsletter.send` gửi một lô `AUTOXIDE_NEWSLETTER_BATCH_SIZE` (mặc định 100, tối đa 1000) dòng
  `pending`: kiểm lại ngay trước khi gửi (chặn, khoá, đã huỷ, giới hạn quảng cáo → `skipped` kèm
  lý do), rồi mỗi dòng: `UPDATE … SET unsubscribe_token_hash = ?, claimed_at = ? WHERE id = ? AND
  status = 'pending' AND unsubscribe_token_hash IS NULL` — đúng một worker thắng trên cả hai
  database; đưa mail vào hàng đợi của `mail` với `dedupe_key = newsletter.delivery:<id>`; đánh
  `sent`. Lô sau có `run_at` = bây giờ + `cỡ lô × 60 / AUTOXIDE_NEWSLETTER_SEND_RATE_PER_MINUTE`
  giây, khoá `newsletter.send:<id>:<run>:<số lô>`. Một lô chỉ là vài trăm lần ghi database nên
  xong rất xa dưới 240 giây.
- `dedupe_key` có tiền tố `newsletter.delivery:` (đề bài: "= ID delivery") vì khoá chống trùng của
  module `mail` dùng chung mọi module, và để nhận ra mail của mình trong `mail.delivery_failed`.
- `sent` nghĩa là "đã giao cho hàng đợi mail"; mail bị máy chủ nhận từ chối hẳn sau đó thì event
  `mail.delivery_failed` đổi lượt gửi sang `failed` (D-134).
- Khe hở "worker chết sau khi xếp mail, trước khi đánh `sent`": dòng đã nhận quá 300 giây (thời
  hạn giữ job) mà vẫn `pending` thành `failed` / `interrupted` — **nhiều nhất một lần**, không gửi
  lại vì token trong mail có thể đã đi mà bản băm đã bị ghi đè. Ghi rõ trong hướng dẫn.
- Gửi / hẹn giờ / tiếp tục tăng `campaigns.run`; job mang `run`, thấy khác số (hay chiến dịch
  không còn `sending`) thì thôi. Tạm dừng có hiệu lực sau lô đang chạy; tiếp tục gửi đúng phần
  `pending` còn lại; huỷ đổi mọi dòng `pending` chưa ai nhận thành `skipped` / `cancelled`.
- Hẹn giờ là job `prepare` có `run_at`; tới giờ job đổi `scheduled` → `sending`.

Đã cân nhắc: trạng thái `sending` cho từng lượt gửi (khảo sát) — đề bài chỉ có bốn trạng thái;
"đã nhận" thể hiện bằng `unsubscribe_token_hash` khác `NULL`. Một job cho mỗi người nhận (Keila)
— quá nhiều dòng job cho danh sách lớn.

## D-133 · `newsletter`: đăng ký công khai: một câu trả lời, giữ lại thì buộc xác nhận, giới hạn mail xác nhận — 2026-11-12 — agent quyết

Quyết định:

- `POST /public/subscribe` luôn `202 {"data": {"accepted": true}}` (mới, đang chờ, đã xác nhận,
  bị khoá, bị chặn, bẫy, vượt giới hạn, gửi lại cùng token). Lỗi chỉ cho dữ liệu sai (422 — kể cả
  `key` không phải danh sách công khai), token / đề hỏng (400 của `SpamGuard`), thiếu cấu hình
  (503 `newsletter.confirm_url_missing`).
- `Verdict::Quarantine` (gửi quá nhanh…) không bị bỏ: đăng ký bị **buộc xác nhận qua mail** kể cả ở
  danh sách không bật double opt-in, lý do lưu ở `hold_reason`. Người thật bấm link là xong; bot
  không có hộp thư.
- Người đã có: tên và ngôn ngữ chỉ ghi khi tạo mới — người lạ không sửa được tên của người khác.
- Mail xác nhận không chứa chữ người đăng ký gõ (kể cả tên): chỉ tên danh sách (quản trị viên đặt)
  và link. Tối đa `AUTOXIDE_NEWSLETTER_CONFIRM_MAILS_PER_DAY` (3) mail xác nhận tới một địa chỉ
  mỗi 24 giờ (`Throttle`, đếm trước transaction vì bộ đếm ghi bằng kết nối riêng).
- Token xác nhận: `OneTimeTokens` mục đích `confirm`, `subject` = ID người đăng ký, `data` = các
  ID danh sách; hạn `AUTOXIDE_NEWSLETTER_CONFIRM_HOURS` (168). Xác nhận bằng `POST
  /public/confirm` từ trang của website; xác nhận không đăng ký lại danh sách người đó đã huỷ
  sau khi xin.
- Nonce của token form lưu ID người đăng ký làm kết quả; `Claim::Replay` → rollback, trả cùng câu.

## D-134 · `newsletter`: bounce và danh sách chặn — 2026-11-12 — agent quyết

Quyết định:

- Nghe `autoxide_kernel::mail::DELIVERY_FAILED` **bằng tên**. Lỗi vĩnh viễn của mail có
  `dedupe_key` của module (`newsletter.delivery:` → lượt gửi thành `failed` / `hard_bounce`), hay
  gửi tới một địa chỉ đang là người đăng ký (vd mail xác nhận tới hộp thư không có), coi là hard
  bounce → chặn. Giao lại cùng event không đổi gì thêm.
- `POST /api/v1/newsletter/bounces` `{email, type: hard|soft|complaint, reason}` cần
  `newsletter.manage` (đề bài chỉ có bốn permission; không thêm permission riêng cho bounce).
  `hard` / `complaint` chặn ngay; `soft` cộng `soft_bounces`, tới
  `AUTOXIDE_NEWSLETTER_SOFT_BOUNCE_LIMIT` (3) thì chặn với lý do `soft_bounce`. Bỏ chặn thì đếm lại
  từ 0.
- Danh sách chặn theo email (đề bài E), không theo hash như khảo sát.

## D-135 · `newsletter`: chiến dịch cho bài mới — 2026-11-12 — agent quyết

Quyết định:

- Nghe `content.entry_published` bằng tên, không phụ thuộc crate `content`. Kernel báo lỗi khởi
  động nếu đăng ký nhận event không module nào khai, nên module chỉ đăng ký khi
  `context.events().contains(…)` (tương tự cho `mail.delivery_failed`) — chương trình không có
  `content` vẫn chạy.
- Chỉ bài viết (`published.kind = post`), đang công khai (`ResourceDirectory`, D-123); trang
  (`page`) không sinh bản tin. Tiêu đề, tóm tắt, đường dẫn lấy qua `resolve("content.entry", id)`.
- `source = "content.entry"`, `source_ref = "<entry_id>:<list_id>"`, unique, chèn `ON CONFLICT DO
  NOTHING`: một bài sinh nhiều nhất một chiến dịch cho mỗi danh sách, mãi mãi — event đăng lại
  hay giao lại không sinh gì mới.
- Thêm `lists.new_post_locale` (`en` / `vi` / `null`): không có thì một bài và bản dịch của nó sinh
  hai bản tin cho cùng danh sách.
- Link tới bài cần địa chỉ website: thêm cấu hình `AUTOXIDE_NEWSLETTER_SITE_URL`
  (`Resource.url` chỉ là đường dẫn). Chế độ `send` mà thiếu địa chỉ website, URL công khai hay
  trang lựa chọn thì chỉ tạo nháp, ghi cảnh báo — không để job lỗi lặp lại.

Đã cân nhắc: `AUTOXIDE_NEWSLETTER_AUTO_POST_MAX_AGE_HOURS` của khảo sát (bỏ qua bài cũ khi nhập
hàng loạt) — đề bài không có; chế độ mặc định `off` và `draft` đủ an toàn. Ghi trong hướng dẫn.

## D-136 · `newsletter`: thư quảng cáo — 2026-11-12 — agent quyết

Quyết định: cờ `advertising` theo chiến dịch (đề bài E). Tiêu đề thêm
`AUTOXIDE_NEWSLETTER_ADVERTISING_LABEL` (mặc định `[QC]`, `none` để bỏ). Mỗi địa chỉ nhận tối đa
`AUTOXIDE_NEWSLETTER_ADVERTISING_MAX_PER_DAY` (mặc định 3, `0` là không giới hạn) mail quảng cáo
trong 24 giờ, đếm từ lượt gửi `sent` của các chiến dịch quảng cáo; vượt thì lượt gửi `skipped` /
`advertising_limit` (không dời sang hôm sau — đơn giản, dễ đoán). Chiến dịch quảng cáo cần
`AUTOXIDE_NEWSLETTER_POSTAL_ADDRESS` (503 `newsletter.postal_address_missing` khi gửi / hẹn giờ /
gửi thử). Mọi giá trị là cấu hình, chưa đối chiếu Nghị định 91/2020 bản gốc (D-112 mục 4).

## D-137 · `newsletter`: permission, `If-Match`, đường dẫn, MCP — 2026-11-12 — agent quyết

Quyết định:

- Bốn permission theo đề bài E: `newsletter.read`; `newsletter.manage` (danh sách, người đăng ký,
  danh sách chặn, báo bounce, **soạn chiến dịch nháp**); `newsletter.send` (gửi, hẹn giờ, tạm
  dừng, tiếp tục, huỷ, gửi thử — gửi thử tới địa chỉ bất kỳ nên cần quyền gửi);
  `newsletter.export` (xuất / xoá theo email).
- `If-Match` bắt buộc: `PATCH` / `DELETE` danh sách và chiến dịch, `PATCH` người đăng ký, `send`,
  `schedule` (gửi đúng bản đã duyệt). Tuỳ chọn: `pause`, `resume`, `cancel`. `DELETE` người đăng
  ký: không cần (xoá là xoá).
- Đường dẫn dưới `/api/v1/newsletter`: `/lists`, `/lists/{id}`, `/subscribers`,
  `/subscribers/import`, `/subscribers/{id}`, `/campaigns`, `/campaigns/{id}`,
  `/campaigns/{id}/{send,schedule,pause,resume,cancel,test}`, `/suppressions`,
  `/suppressions/{id}`, `/bounces`, `/data?email=`; công khai `/public/lists`,
  `/public/subscribe`, `/public/confirm`, `/public/unsubscribe`, `/public/preferences`.
- Nhập người đăng ký: `POST /subscribers/import` (1–1000 người, 100 người mỗi transaction), luôn ở
  `confirmed` và **bắt buộc** `consent_source`; không đăng ký lại người đã huỷ ở danh sách đó.
  Quản trị viên không đăng ký thẳng được ai không có nguồn đồng ý (`PATCH` chỉ huỷ được).
- Tool MCP: `newsletter.list_lists`, `newsletter.find_subscribers`, `newsletter.get_campaign`
  (tiến độ), `newsletter.create_campaign` (nháp, `newsletter.manage`),
  `newsletter.send_campaign`, `newsletter.schedule_campaign` (`newsletter.send`, nhận `version`
  như `If-Match`).
- Tên schema OpenAPI trùng với module khác (`Locale`, `Localized`, `DeliveryResponse`,
  `EmailDataResponse`, `ErasedResponse`, `ScheduleRequest`) được đổi bằng `#[schema(as =
  Newsletter…)]`: kernel ghép component theo tên và bản sau ghi đè bản trước **không báo lỗi**.

## D-138 · `newsletter`: dữ liệu cá nhân — 2026-11-12 — agent quyết

Quyết định:

- Người đăng ký **không gắn với tài khoản**: đăng ký công khai chỉ có email, không có `user_id`.
  Mọi yêu cầu của chủ dữ liệu đi theo email: `GET` / `DELETE /api/v1/newsletter/data?email=`
  (`newsletter.export`). `PersonalData` và `USER_DELETED` chỉ áp cho `created_by` của danh sách
  và chiến dịch (như `forms`).
- Xoá theo email xoá người đăng ký, đăng ký (kèm bằng chứng đồng ý), lượt gửi, token xác nhận —
  nhưng **giữ dòng của danh sách chặn** nếu có: nghĩa vụ không gửi lại cho người đã bounce / khiếu
  nại / bị chặn tay cần giữ địa chỉ. Đây là ngoại lệ có lý do của quyền xoá; người cài xoá dòng đó
  bằng `DELETE /suppressions/{id}` nếu người đó yêu cầu. Bản xuất có dòng chặn.
- IP chỉ lưu dạng `ip_hash::fingerprint` bên trong bằng chứng đồng ý (giữ cùng bằng chứng, như
  D-127); không có cột IP riêng nên không cần cấu hình thời hạn xoá IP.
- Event không mang email; webhook nhận ID và `key` danh sách.
- Lượt gửi giữ mãi (link huỷ trong mail không hết hạn); job hằng ngày chỉ xoá đăng ký chưa xác
  nhận quá `AUTOXIDE_NEWSLETTER_UNCONFIRMED_DAYS` (30) và người không còn gì.

## D-139 · `newsletter`: cấu hình — 2026-11-12 — agent quyết

Quyết định: `AUTOXIDE_NEWSLETTER_CONFIRM_URL`, `…_PREFERENCES_URL` (có `{token}`, như
`AUTOXIDE_AUTH_*_URL`), `…_SITE_URL`, `…_CONFIRM_HOURS` (168), `…_CONFIRM_MAILS_PER_DAY` (3),
`…_UNCONFIRMED_DAYS` (30), `…_SUBSCRIBE_PER_IP_PER_HOUR` (10), `…_SUBSCRIBE_PER_EMAIL_PER_HOUR`
(5), `…_SUBSCRIBE_PER_USER_PER_HOUR` (1000), `…_SEND_RATE_PER_MINUTE` (300), `…_BATCH_SIZE` (100),
`…_SOFT_BOUNCE_LIMIT` (3), `…_ADVERTISING_LABEL` (`[QC]`), `…_ADVERTISING_MAX_PER_DAY` (3),
`…_POSTAL_ADDRESS`. Không biến nào bắt buộc lúc khởi động; thiếu thì chỉ chức năng cần nó báo
503 rõ ràng: trang xác nhận (đăng ký vào danh sách double opt-in), `AUTOXIDE_HTTP_PUBLIC_URL` và
trang lựa chọn (gửi, hẹn giờ, tiếp tục, gửi thử), địa chỉ người gửi (thư quảng cáo).

## D-140 · Kernel báo tên schema OpenAPI trùng nhau giữa các module — 2026-11-12 — agent quyết

Bối cảnh: khi ghép tài liệu OpenAPI của các module, utoipa gộp component theo tên — hai module khai
cùng tên kiểu (vd `Locale`) với nội dung khác nhau thì bản sau lặng lẽ thay bản trước, endpoint
của module kia mô tả sai. Subagent viết `newsletter` gặp sáu tên trùng.

Quyết định: `http::build` so component schema của kernel và từng module trước khi ghép; cùng tên
khác nội dung thì ghi vào `Api::schema_conflicts`. Lệnh `autoxide openapi` coi đó là lỗi như
`http::lint`, nên test giữ `docs/openapi.json` khớp code (chạy trong gate) đỏ ngay. Cùng tên cùng
nội dung (vd `Problem`) không sao. Module đặt tên riêng bằng `#[schema(as = <Module>…)]`.

Đã cân nhắc: tự thêm tiền tố tên module vào mọi schema — đổi tên mọi kiểu đang có trong
`docs/openapi.json` (thay đổi phá vỡ với SDK đã sinh).

## D-141 · `comments`: bảng, trạng thái, hai cấp, số đếm — 2026-11-12 — agent quyết

Quyết định:

- Sáu bảng theo đề bài F: `comments_comments`, `comments_targets` (khoá chính `(target_type,
  target_id)`), `comments_bans`, `comments_reports` (unique `(comment_id, reporter_key)`),
  `comments_reply_subscriptions` (unique `(comment_id, email)`), `comments_profiles` (khoá chính
  `user_id`, unique `name_key`). Cộng các bảng của kernel theo thứ tự của hợp đồng: events
  (+ `actor_kind`), jobs, throttle, antispam nonces, tokens. Không foreign key, kể cả trong module.
- Cột ngoài bảng của đề bài: `depth` (0 / 1, cho index luồng), `published_at` (lần đầu được hiện —
  dùng cho "đã từng được duyệt"), `content_hash`, `search_key`, `reply_count`, `report_count`,
  `reports_held_at` (bình luận); `hidden` (đích); `reporter_user_id`, `note` (báo cáo);
  `name_key` (hồ sơ).
- Trạng thái đúng hợp đồng §4: `pending` / `published` / `rejected` / `spam` / `deleted`;
  `hold_reason` là lý do của `Verdict::Quarantine` nối bằng dấu phẩy, hoặc lý do riêng:
  `moderation`, `first_comment`, `reported`, `suspect_word`, `too_many_links`.
- Hai cấp như Ghost: bình luận gốc có `root_id` = chính nó; trả lời có `parent_id` = `root_id` =
  bình luận gốc và `reply_to_id` = bình luận đang được trả lời. Trả lời một trả lời gắn vào gốc.
- Số đếm (`published_count`, `pending_count` của đích; `reply_count` của bình luận gốc) đổi trong
  CÙNG transaction với trạng thái bằng `UPDATE … SET n = n + k`; đổi trạng thái dùng `UPDATE` có
  điều kiện trạng thái + `updated_at` cũ (đua thì 412). Dòng đích chèn `ON CONFLICT DO NOTHING`.
  Test đồng thời trên hai database.
- Bình luận gốc đã xoá mềm vẫn hiện (không chữ, không tên) khi còn trả lời đang hiện; trả lời đã
  xoá thì không hiện.

Đã cân nhắc: bảng `comments_revisions` của khảo sát — đề bài F không có; sửa chạy lại kiểm nội dung
là đủ chặn trò "đăng sạch rồi sửa bẩn". Đếm trả lời bằng `GROUP BY` khi đọc — bộ đếm cùng
transaction rẻ hơn và cho lọc "gốc đã xoá còn trả lời".

## D-142 · `comments`: pipeline kiểm duyệt và chế độ duyệt; "đã từng được duyệt" chỉ theo tài khoản — 2026-11-12 — agent quyết

Quyết định:

- Thứ tự cố định của đề bài F: đích đóng → độ dài → lọc nội dung (ký tự điều khiển / vô hình bị
  từ chối, xuống dòng chuẩn hoá) → trùng (cùng chữ, cùng đích, 24 giờ → cờ `duplicate` cho
  `SpamGuard`) → bị cấm (403 `comments.banned`) → từ cấm (422 `comments.banned_word`) / từ nghi
  ngờ (giữ `suspect_word`) → quá `AUTOXIDE_COMMENTS_MAX_LINKS` link (giữ `too_many_links`) →
  `SpamGuard` → chế độ duyệt. Lỗi đầu vào gom một lần (D-083) với mã trường `required`, `length`,
  `invalid`, `email`, `reserved`.
- `Verdict::Quarantine` → `spam` (kèm lý do; không tính vào `pending_count`); module tự giữ →
  `pending`; `Discard` → không lưu.
- Từ cấm / nghi ngờ / tên dành riêng so theo TỪ TRỌN (đã `search::fold`: không hoa thường, không
  dấu) để tránh lỗi "Scunthorpe".
- Chế độ `pre` / `first_time` / `post`, mặc định khách `pre`, người dùng `first_time`, chỉnh theo
  loại đích bằng `AUTOXIDE_COMMENTS_MODERATION_BY_TYPE` (`<loại>:guest=<chế độ>`). Người có
  `comments.moderate` bỏ qua chế độ (không bỏ qua giữ lại vì từ / link).
- "Đã từng được duyệt" = tài khoản có bình luận với `published_at` khác `NULL`. **Không bao giờ
  tính theo email**, kể cả email đã xác minh: khách gõ lại email mỗi lần, nên xác minh chỉ chứng
  minh hộp thư đó nhận được mail, không chứng minh người đang gửi là chủ hộp thư (lỗ của Isso /
  WordPress). Vì vậy `first_time` cho khách tương đương `pre` (ghi trong mô tả biến).
- Test "khách lần đầu bị giữ, lần sau (email đã xác minh) được hiện" của đề bài F mâu thuẫn với
  quy tắc trên; giải theo quy tắc: test kiểm khách lần đầu bị giữ, xác minh email, được duyệt, lần
  sau cùng email (đã xác minh) VẪN chờ duyệt, kể cả ở chế độ `first_time`; còn người dùng đăng
  nhập lần đầu chờ, được duyệt rồi thì lần sau hiện ngay.

Đã cân nhắc: tin email đã xác minh ở các lần sau (giả mạo được bằng cách gõ email người khác); cấp
cho khách đã xác minh một "khoá khách" giữ ở trình duyệt — thực chất là một kiểu tài khoản mới, để
sau nếu chủ dự án muốn.

## D-143 · `comments`: endpoint công khai và câu trả lời giống nhau — 2026-11-12 — agent quyết

Quyết định:

- Đường dẫn dưới `/api/v1/comments/public/…` (hợp đồng §2): `settings`, `threads`, `comments`
  (`POST`), `comments/{id}` (`GET`, `PATCH`), `comments/{id}/replies`, `comments/{id}/delete`
  (`POST`), `comments/{id}/reports`, `counts`, `targets/{type}/{id}`, `verify`, `unsubscribe`.
  Tên hiển thị: `GET` / `PUT /api/v1/comments/profile` (cần đăng nhập).
- Phạm vi token form `comments:<target_type>`; giới hạn: IP 20 / giờ, email 10 / giờ (khách), người
  dùng 60 / giờ — đều là cấu hình.
- Câu trả lời `201` chỉ nói `published` hay `pending`: giữ chờ duyệt, giữ vì spam và bị bỏ đều là
  `pending` (bỏ thì ID và token sửa giả cùng định dạng). Bài được hiện thì nói `published` — không
  lộ gì thêm, vì bot đọc luồng công khai cũng thấy. Đây là cách đọc hợp đồng §2 ("không lộ trạng
  thái thật") cho module mà người viết thật cần biết bình luận có đang chờ duyệt không.
- Gửi lại cùng token form (`Claim::Replay`) trả lại bình luận đầu, khách nhận token sửa MỚI (máy
  chủ chỉ giữ bản băm nên không trả lại được token cũ; token cũ thôi dùng được).
- Khách sửa / xoá bằng `edit_token` trong THÂN request (không có header riêng: CORS của kernel chỉ
  cho một danh sách header cố định, không được sửa kernel); vì vậy xoá của tác giả là `POST
  …/delete` thay vì `DELETE` (không đặt token vào URL). Người dùng đăng nhập gửi `{}`.
- Báo cáo không cần token form: mỗi người (user / dấu vết IP) một lần mỗi bình luận, giới hạn
  `AUTOXIDE_COMMENTS_REPORTS_PER_HOUR`, vượt thì trả lời như thường mà không lưu.
- Thân `POST /public/comments` và `PATCH /public/comments/{id}` tối đa 128 KiB (`Module::body_limits`).

## D-144 · `comments`: kiểm duyệt, cấm, đích, MCP, permission — 2026-11-12 — agent quyết

Quyết định:

- Quản trị: `GET /` (hàng chờ, mặc định `pending`, lọc trạng thái / đích, tìm không dấu theo
  `search_key`), `GET` / `PATCH` (`If-Match` bắt buộc) / `DELETE /{id}` (xoá hẳn; gốc thì cả luồng),
  `GET /{id}/reports`, `GET` / `POST /bans`, `DELETE /bans/{id}`, `GET` / `PUT
  /targets/{type}/{id}` (`If-Match` tuỳ chọn — đặt trạng thái kiểu bật / tắt như `pause` của
  newsletter), `GET` / `DELETE /data?email=`.
- Báo cáo đủ ngưỡng giữ lại ĐÚNG MỘT LẦN: `UPDATE` có điều kiện `status = 'published' AND
  reports_held_at IS NULL AND report_count >= ngưỡng`; duyệt lại rồi thì báo cáo sau không ẩn nữa.
- Cấm theo `user` (ID), `email` (chữ thường), `ip` (`ip_fingerprint` của bình luận — cùng khối
  /24 hay /64); `reason` bắt buộc; `until` phải ở tương lai, trống là vĩnh viễn. Event cấm không
  mang giá trị.
- Đích: `open` / `closed` + `hidden` (ẩn là đóng luôn) + `closes_at` (tự đóng;
  `AUTOXIDE_COMMENTS_AUTO_CLOSE_DAYS` đặt sẵn khi tạo dòng). Event `comments.target_opened` /
  `comments.target_closed` khi trạng thái thật đổi.
- Nghe `content.entry_archived` (đóng) và `content.entry_deleted` (đóng và ẩn) BẰNG TÊN, chỉ khi
  event có trong chương trình và `content.entry` được bật; ID lấy từ chủ thể `content.entry:<id>`.
  Không nghe `content.entry_unpublished`: bài về nháp thì sổ đối tượng báo không công khai nên bình
  luận mới bị từ chối, còn bình luận cũ ẩn theo trang của website.
- Permission đúng đề bài F (`comments.post`, `comments.read`, `comments.moderate`,
  `comments.manage`) **cộng `comments.export`** cho xuất / xoá theo email — hợp đồng §3 bắt
  `<module>.export`, và hợp đồng thắng đề bài.
- Tool MCP: `comments.queue`, `comments.get` (`comments.read`), `comments.moderate`
  (`comments.moderate`, nhận `version` như `If-Match`), `comments.set_target_state`
  (`comments.manage`). Không có tool viết bình luận hay xoá hẳn.
- Tên schema OpenAPI mang tiền tố `Comment…` / `Comments…` (D-140).

## D-145 · `comments`: chữ thường, HTML an toàn tự viết — 2026-11-12 — agent quyết

Quyết định: lưu chữ người viết (đã lọc); trả cả `body` lẫn `body_html` dựng khi đọc: escape `& < >
" '`, xuống dòng thành `<br>`, chỉ `http://` / `https://` thành link `rel="nofollow ugc noopener"`
(link dừng ở khoảng trắng hay `< > " '`, bỏ dấu câu cuối, giữ `)` khi có `(`). Khoảng 100 dòng tự
viết, có test XSS (`<script>`, `on…`, `javascript:`, `data:`, chèn thuộc tính qua `"`). Không thêm
thư viện; Markdown (`pulldown-cmark` + `ammonia`) để sau.

## D-146 · `comments`: mail báo trả lời — 2026-11-12 — agent quyết

Quyết định:

- Chọn nhận: `notify_replies: true` + `email` → mail xác minh (token `OneTimeTokens` mục đích
  `verify`, `subject` = ID bình luận, `data` = email, sống 72 giờ, tối đa
  `AUTOXIDE_COMMENTS_VERIFY_MAILS_PER_DAY` mail / địa chỉ / ngày bằng `Throttle`) → trang
  `AUTOXIDE_COMMENTS_VERIFY_URL` gửi token tới `POST /public/verify` → `email_verified` và dòng
  theo dõi. Mỗi bình luận xác minh riêng (không ai đăng ký được email người khác).
- Gửi khi trả lời được HIỆN: module nghe `comments.comment_published` của chính nó (kể cả khi được
  duyệt sau), gửi tới người theo dõi bình luận được trả lời (`reply_to_id`), trừ chính người trả
  lời. Mail không có chữ do người lạ gõ (hợp đồng §5): chỉ tiêu đề và đường dẫn của đích lấy qua sổ
  đối tượng (đích không có bộ tra thì một từ chung, không dùng `target_id` do client gửi),
  `AUTOXIDE_COMMENTS_SITE_URL` + đường dẫn, và link huỷ.
- Token huỷ RIÊNG CHO TỪNG MAIL (như D-130 của newsletter): 32 byte ngẫu nhiên, chỉ lưu băm trong
  `comments_tokens` (mục đích `unsubscribe`, `subject` = ID dòng theo dõi, sống 365 ngày) — bảng
  `comments_reply_subscriptions` vì vậy không có cột token (khác bảng của đề bài F: một token
  cho mỗi dòng mà chỉ lưu bản băm thì không đặt lại được vào mail sau). `POST /public/unsubscribe`
  luôn trả 200; `all: true` bỏ mọi dòng theo dõi của địa chỉ đó.
- `dedupe_key` = `comments.reply:<ID trả lời>:<ID dòng theo dõi>` và `comments.verify:<ID token>`.
- Thiếu `AUTOXIDE_COMMENTS_VERIFY_URL` hoặc `AUTOXIDE_COMMENTS_UNSUBSCRIBE_URL` thì tính năng tắt
  (không lỗi khởi động, `GET /public/settings` báo `reply_mails: false`).

## D-147 · `comments`: dữ liệu cá nhân và thời hạn giữ — 2026-11-12 — agent quyết

Quyết định:

- Xoá theo email / tài khoản bị xoá là xoá mềm giữ cấu trúc luồng (hợp đồng §3): chữ, tên, email,
  token sửa, dấu vết IP và (với tài khoản) `user_id` bị bỏ; dòng theo dõi và token bị xoá; một event
  `comments.comments_erased` cho cả lô, chỉ có ID. Hồ sơ tên bị xoá; báo cáo của người đó giữ lại
  nhưng `reporter_key` thành `deleted:<id báo cáo>`.
- Lệnh cấm theo email GIỮ LẠI khi xoá theo email (để còn chặn được) — ngoại lệ có lý do, như danh
  sách chặn của newsletter (D-138); bản xuất có nó.
- Tác giả tự xoá thì dấu vết IP giữ tới hạn thường (để còn cấm được kẻ phá).
- Job hằng ngày `comments.cleanup`: `pending` / `spam` quá `AUTOXIDE_COMMENTS_RETENTION_DAYS`
  (30) bị xoá hẳn — bình luận gốc còn bình luận khác trong luồng thì xoá mềm; dấu vết IP quá
  `AUTOXIDE_COMMENTS_IP_RETENTION_DAYS` (90) bị xoá; nonce, bộ đếm, token hết hạn bị dọn.
  `rejected` giữ lại (đề bài chỉ nói chờ duyệt / spam; quyết định của người kiểm duyệt là hồ sơ).
- `PersonalData` xuất hồ sơ, bình luận, báo cáo của tài khoản.

## D-148 · `comments`: cấu hình — 2026-11-12 — agent quyết

Quyết định: `AUTOXIDE_COMMENTS_ENABLED` (true), `…_TARGET_TYPES` (`content.entry`),
`…_ALLOW_GUESTS` (true), `…_SIGNED_IN_ONLY_TYPES`, `…_GUEST_MODERATION` (`pre`),
`…_USER_MODERATION` (`first_time`), `…_MODERATION_BY_TYPE`, `…_MAX_LENGTH` (4000), `…_MAX_LINKS`
(2), `…_EDIT_MINUTES` (15), `…_REPORT_THRESHOLD` (3), `…_AUTO_CLOSE_DAYS` (0), `…_BANNED_WORDS`,
`…_SUSPECT_WORDS`, `…_RESERVED_NAMES` (`admin,administrator,moderator,root,system`),
`…_PER_IP_PER_HOUR` (20), `…_PER_EMAIL_PER_HOUR` (10), `…_PER_USER_PER_HOUR` (60),
`…_REPORTS_PER_HOUR` (30), `…_RETENTION_DAYS` (30), `…_IP_RETENTION_DAYS` (90), `…_VERIFY_URL`,
`…_UNSUBSCRIBE_URL`, `…_SITE_URL`, `…_VERIFY_MAILS_PER_DAY` (3). Không biến nào bắt buộc lúc
khởi động. Bỏ `…_FLOOD_SECONDS` và `…_TRUSTED_AFTER` của khảo sát: giới hạn theo giờ của
`SpamGuard` và chế độ `first_time` đã che; `…_MAX_DEPTH` cố định 2 theo đề bài.

## D-149 · Rà soát bảo mật phần thêm sau D-079: sửa và các quy ước mới — 2026-11-12 — agent quyết

Bốn agent đọc độc lập phần thêm sau lượt rà soát D-079: `media` (chưa từng rà), kernel cùng
`files` / `webhooks` / `auth`, `content`, và luồng dữ liệu giữa các module. Không có lỗi nặng.
Mọi phát hiện được kiểm lại với code; những cái dưới đây đã sửa, mỗi cái kèm test chạy trên cả
hai database.

**Kernel và hạ tầng:**

- **Idempotency không giữ thân request lớn trong RAM.** Trước đây lớp idempotency đọc hết thân
  (tới `AUTOXIDE_MEDIA_MAX_BYTES`, mặc định 100 MiB) để băm, trước khi handler kiểm permission:
  người chỉ cần đăng nhập giữ được hàng GB RAM. Nay chỉ thân không lớn hơn
  `AUTOXIDE_HTTP_BODY_LIMIT_BYTES` được đọc trước; thân lớn hơn (hay không khai độ dài) ở route
  nhận thân lớn chảy thẳng vào handler, và dấu của request dùng độ dài thay cho nội dung. Cái
  giá: gửi lại cùng key, cùng độ dài mà khác nội dung thì nhận lại kết quả lần đầu thay vì
  `idempotency_key_reused` — chấp nhận, vì dùng lại key cho request khác là lỗi của client.
- **Response chứa secret không bị lưu.** Lớp idempotency từng lưu 24 giờ response của mọi lệnh
  ghi có key, kể cả API key mới, token, mã khôi phục — những thứ `auth` chỉ lưu dạng băm. Quy
  ước mới: response mang `Cache-Control: no-store` thì không lưu; lần gửi lại nhận
  `409 idempotency_result_not_kept` (không chạy lại việc). Module `auth` gắn `no-store` cho mọi
  response của nó (RFC 6749 §5.1). Tool MCP trả secret khai `Tool::returns_secret()`; response
  MCP khi đó mang `no-store` (cờ dùng chung giữa lớp xác thực và `call_tool` qua extensions).
- **Webhook chỉ gửi tới địa chỉ public (SSRF).** Từ chối khi tạo / sửa địa chỉ là IP không
  public hay `localhost`; lúc gửi, tên miền được phân giải bằng bộ lọc riêng (`PublicOnly`) bỏ
  mọi IP không public — lọc lúc kết nối nên chặn cả DNS rebinding; không dùng proxy hệ thống.
  Cấu hình mới `AUTOXIDE_WEBHOOKS_ALLOW_PRIVATE_NETWORKS` (mặc định `false`) cho máy nhận trong
  mạng riêng. Lịch sử gửi chỉ ghi lý do cố định (không chép chuỗi lỗi, vốn giúp dò cổng); chi
  tiết vào log máy chủ.
- **Cursor sai kiểu** (chữ cho cột số) trả `invalid_cursor` trên cả hai database, không còn 500
  trên Postgres.
- **Admin cuối cùng:** `count_active_admins` khoá các role toàn quyền trước khi đếm, nên mọi
  thay đổi có thể làm mất admin nối đuôi nhau; trên Postgres hai admin cuối xoá nhau cùng lúc
  không còn để lại 0 admin.
- **Xoá tài khoản:** audit bỏ `before` / `after` / `details` của mọi dòng cùng `subject` với event
  xoá (email, tên); event `auth.user_deleted` không bao giờ bị đánh dấu hỏng — thử lại tối đa mỗi
  giờ một lần tới khi mọi module xong, vì tài khoản đã mất thì không ai xin xoá lại được.
- **Mô tả permission** `audit.read` và `media.write` nói rõ chúng thấy được những gì.

**`media`:**

- SVG có nội dung chạy được (`<script>`, `on…=`, `javascript:`, HTML nhúng) ở **bất cứ đâu**
  trong file bị từ chối (`media.svg_active_content`), quét từng mảnh khi đang nhận.
- Gợi ý alt text luôn gửi bản làm lại (bỏ EXIF), không bao giờ gửi chính file gốc, khớp tài liệu.
- Ảnh có file lớn hơn mọi cách nén của số pixel (8 byte mỗi pixel + 1 MiB) không được giải mã
  (không làm bản thu nhỏ, không gợi ý alt); file được đọc vào một vùng nhớ cấp sẵn, không giữ
  hai lần.
- `Range: bytes=-N` trên file rỗng trả `416`.
- Tài liệu: file gốc giữ metadata như lúc tải lên; `media.upload` qua MCP thực tế khoảng 750 KiB
  với giới hạn thân mặc định.

**`content`** (thêm vào D-102, D-105, D-111):

- Gỡ hay lưu trữ huỷ lịch đăng đang chờ (lưu trữ huỷ cả lịch gỡ), kèm event
  `content.schedule_canceled`: trước đây lịch đăng chạy sau đó đăng lại mục vừa bị gỡ.
- Người chỉ có `content.publish_own`: không đăng lại mục đã lưu trữ (`content.archived`), không
  tự đặt `published_at`, không đăng vào địa chỉ có chuyển hướng làm tay
  (`content.path_has_redirect`).
- Người chỉ có `content.write_own`: `ai_generated` luôn `true`; chỉ gắn hồ sơ tác giả của chính
  mình (hồ sơ đã có trên mục được giữ); trang cha phải là trang của mình.
- Chuyển hướng và link menu từ chối `//host`.
- `q` của tìm kiếm công khai tối đa 200 ký tự; bản xem trước công khai không lộ id người dùng
  (`PreviewEntryResponse`).
- Gắn chủ đề / tác giả khoá dòng của mục trong transaction (Postgres hết 500 khi gọi đồng thời);
  việc đó và tạo token xem trước chỉ cần quyền sửa mục, không cần `content.read`.
- Xoá tài khoản bỏ cả `review_requested_by` và `ai_reviewed_by`; `ai_reviewed` công khai tính từ
  `ai_reviewed_at` nên vẫn đúng.

Để lại, ghi rõ:

- **Xoá dữ liệu khi xoá tài khoản không phát event cho từng file / item** (`files`, `media`): một
  tài khoản lớn sẽ đẩy hàng nghìn event vào audit và webhook; event `auth.user_deleted` trong
  audit đã ghi việc xoá. Ngoại lệ có chủ ý cho luật 3.
- **Thân event đã gửi qua webhook** (có email, tên) nằm trong lịch sử gửi tới 30 ngày sau khi
  tài khoản bị xoá; API không trả thân đó.
- **Lịch đăng chạy với quyền của người đặt lịch lúc đặt.** Đặt lịch cần `content.publish`; người
  bị khoá hay mất quyền sau đó thì lịch vẫn chạy — muốn dừng thì huỷ lịch. Xoá tài khoản huỷ lịch.
- **Xoá chủ đề đúng lúc đang gắn chủ đề** vẫn có thể để lại một liên kết mồ côi trên Postgres
  (khoá dòng mục, không khoá dòng chủ đề); cửa sổ đã hẹp lại vì kiểm chủ đề tồn tại trong
  transaction.
- **Job làm ảnh thu nhỏ chậm:** hết giờ thì job bị huỷ nhưng thread giải mã vẫn chạy tới xong;
  chưa đo được ảnh nào gây ra, nên chưa đổi.
- `GET /api/v1/content/entries?q=` (cần `content.read`) chưa giới hạn độ dài.
- **Ngoại lệ gitleaks:** commit `e7ecf3d` lỡ chứa một chuỗi giả (`0123456789abcdef…`) làm secret
  xem trước trong test của `content`; commit sau thay bằng hằng số test có sẵn. Vì không viết
  lại lịch sử nhánh đã đẩy, `.gitleaks.toml` bỏ qua đúng commit đó, đúng file đó.

## D-150 · Rà soát bảo mật forms, newsletter, comments và nền chung cho dữ liệu người lạ — 2026-11-12 — agent quyết

Bốn agent đọc độc lập ba module mới và các khối dùng chung trong core (D-113…D-124). Một lỗi
nặng (comments), còn lại trung bình và nhẹ. Mọi phát hiện được kiểm lại với code và sửa, kèm test
chạy trên cả hai database.

**Quy ước mới:**

- **Đăng nhập không phải bằng chứng "không phải bot".** Bỏ qua lớp chống spam cho người lạ (bẫy,
  token form, đề, giới hạn theo IP / email) cần permission riêng của module: `forms.submit`,
  `newsletter.subscribe` — cấp cho API key của máy chủ website. Tài khoản khác (vd tự đăng ký) đi
  đúng đường của người lạ. `comments` vốn đã đòi `comments.post`. Đây là thay đổi phá vỡ cho website
  đang gọi bằng API key không có hai permission đó.
- **Việc dẫn tới dữ liệu cá nhân hay thư gửi đi đòi đúng quyền đó:** đổi `notify_emails` của form
  cần thêm `forms.read`; bật `new_posts: send` cần `newsletter.send`; sửa chiến dịch đang hẹn giờ
  thì nó về nháp và tăng `run` (người có `newsletter.send` phải hẹn lại).
- **Form token đã dùng** (bài đã nhận) dừng ở pha một: không đếm giới hạn theo IP / email, không
  gọi bộ phân loại — trước đây gửi lại một token nhiều lần khoá được email của người khác. Pha một
  đọc bảng nonce của module qua `Throttle` của nó.
- **Điền tham số i18n một lượt** (`fill_all`): chữ người dùng chứa `{x}` không còn làm mail rơi về
  mẫu thô, `{id}` trong câu trả lời không bị thay.
- **Bộ tra của `ResourceDirectory` không đưa nội dung chưa công khai ra ngoài:** `title`,
  `summary` rỗng khi `public` là `false`.
- **Audit chỉ giữ bản băm email** (`sha256:…`) của event `mail.delivery_failed` — event không gắn
  tài khoản nên không xoá được theo `subject`.
- Tính SHA-256 của ALTCHA chạy trên thread cho việc chặn.

**comments:**

- **(nặng)** Job dọn chỉ xoá bình luận **chưa từng hiện** (`published_at IS NULL`); mỗi dòng xoá có
  điều kiện trạng thái và `updated_at`. Trước đây ba báo cáo vô danh đưa một bình luận cũ về chờ
  duyệt, rồi job dọn xoá vĩnh viễn nó. Khách báo cáo gom theo IPv6 /48.
- Sửa bình luận đang hiện thì xét lại chế độ duyệt như bình luận mới; `PATCH` trả thêm `status`.
- "Đã từng được duyệt" chỉ tính bình luận đang `published`.
- `reporter_key` của khách bỏ dấu vết IP sau `IP_RETENTION_DAYS`.
- Trả lời khoá dòng gốc bằng `UPDATE` có điều kiện trong transaction; `hard_delete` khoá trả lời,
  rồi gốc; thứ tự khoá trong module là bình luận → gốc → đích.
- Xoá với id lạ trả như xoá thật (`PATCH` với id giả vẫn 404 — giới hạn đã ghi).
- Tên từ chối ký tự định dạng Unicode (Cf), ký tự trông như khoảng trống, và từ trộn chữ Latin /
  Hy Lạp / Kirin.

**forms:** email kiểm theo byte như lettre và cả dạng đã chuẩn hoá (hết 500 trên Postgres); mail
báo bỏ `Reply-To` thay vì hỏng; xoá / xuất theo email khớp mọi câu trả lời bằng đúng địa chỉ; pha
hai chạm dòng form (form bị xoá giữa chừng thì `forms.not_found`), job dọn xoá bài mồ côi; tối đa
50 lỗi cho key lạ.

**newsletter:** người đã `unsubscribed` hay đang `unconfirmed` phải xác nhận qua mail kể cả ở danh
sách single opt-in; bộ đếm mail xác nhận đếm mọi yêu cầu đã lưu, sau commit
(`AUTOXIDE_NEWSLETTER_CONFIRM_MAILS_PER_DAY` đổi nghĩa); lỗi xếp hàng mail sau commit chỉ ghi log;
`body_html` không được làm sạch — giao diện quản trị phải hiện trong `<iframe sandbox>`.

Để lại, ghi rõ:

- **Dấu IP giống nhau giữa các module, và tính ngược được khi khoá nằm trong database.** Đổi khoá
  theo module đụng tới việc kiểm token chống spam giữa module; ghi trong DEPLOY: khi chạy thật nên
  đặt `AUTOXIDE_SECRET_KEY` ngoài database.
- **Đo thời gian trả lời đăng ký bản tin** vẫn phân biệt được phần nào (đường cần mail chèn thêm
  token và một mail vào hàng đợi).
- **Tên viết trọn bằng chữ Kirin** giống tên Latin vẫn lọt.
- Đếm proxy (D-114): trình duyệt gọi thẳng qua proxy cuối khi `N > 1` thì tự chọn được địa chỉ; đã
  có cảnh báo trong DEPLOY, cách tin theo dải địa chỉ để sau.

## D-151 · `comments`: so tên theo dạng "khung" — chữ Kirin / Hy Lạp giống chữ Latin thành chữ Latin — 2026-11-12 — agent quyết

Đóng giới hạn "tên viết trọn bằng chữ Kirin giống tên Latin vẫn lọt" của D-150.

Quyết định: `text::name_key` = `words_key` rồi đổi chữ Kirin / Hy Lạp trông như chữ Latin
thành chữ Latin (bảng tự liệt kê ~60 chữ, xét theo hoa / thường, đổi cả trước khi viết thường
lẫn sau khi bỏ dấu). Dùng cho: khoá unique `comments_profiles.name_key` và việc so với danh sách
tên dành riêng (`contains_name`, cho cả khách lẫn người dùng). Migration m0008 tính lại khoá
của hồ sơ có sẵn; hai hồ sơ có sẵn ra cùng khoá thì hồ sơ sau giữ khoá cũ (không đổi tên của
ai, không chạm index unique giữa chừng). Luật "không trộn bảng chữ trong một từ" giữ nguyên.

Không dùng `name_key` cho nội dung bình luận và từ cấm: từ cấm tiếng Nga sẽ khớp nhầm chữ
Latin.

Đã cân nhắc: thư viện bảng confusables đầy đủ (Unicode TS #39) — vài nghìn dòng dữ liệu cho
một việc chỉ cần ba bảng chữ hay bị dùng giả; cấm hẳn tên Kirin / Hy Lạp — loại người dùng
thật.

## D-152 · Địa chỉ client: tin proxy theo dải địa chỉ (`AUTOXIDE_HTTP_TRUSTED_PROXIES`) — 2026-11-12 — agent quyết

Bổ sung D-114, đóng giới hạn "đếm proxy" của D-150: khi `N > 1`, trình duyệt gọi thẳng proxy
cuối (bỏ qua CDN) thì tự chọn được địa chỉ.

Quyết định: biến không bắt buộc `AUTOXIDE_HTTP_TRUSTED_PROXIES` — danh sách địa chỉ / dải CIDR
(IPv4, IPv6; `::ffff:a.b.c.d` tính như IPv4). Có giá trị thì thay cho
`AUTOXIDE_HTTP_TRUSTED_PROXY_COUNT`: kết nối không đến từ dải thì bỏ qua header; có thì đi từ
phải sang trái, bỏ qua địa chỉ thuộc dải, lấy địa chỉ đầu tiên không thuộc dải (toàn dải: phần
tử đầu; giá trị không đọc được: địa chỉ kết nối, như D-114). Đặt dải mà không đặt
`AUTOXIDE_HTTP_CLIENT_IP_HEADER` là lỗi cấu hình. Kiểu `config::IpRange` tự viết (~40 dòng),
không thêm thư viện CIDR.

Giữ cách đếm làm mặc định: không cần biết địa chỉ proxy, đúng cho trường hợp phổ biến (một
reverse proxy).

## D-153 · Dấu IP riêng theo phạm vi; cảnh báo khi khoá ký nằm trong database — 2026-11-12 — agent quyết

Sửa D-115, đóng giới hạn "dấu IP giống nhau giữa các module" của D-150.

Quyết định:

- `ip_hash::fingerprint(secret, scope, ip)` và `ip_hash::bucket(secret, scope, ip, now)` nhận
  phạm vi; khoá dẫn theo nhãn `autoxide/ip-hash/{fingerprint,bucket}/v2/<scope>…`. Module truyền
  tên mình cho `fingerprint`; chống spam truyền phạm vi form (`forms:contact`…) cho `bucket`.
  Đọc database của một module không nối được người gửi sang module khác.
- Thay đổi phá vỡ: dấu đã lưu không khớp dấu mới. Dấu của bài gửi / bình luận tự xoá theo thời
  hạn của module; lệnh cấm bình luận theo dấu mạng làm trước bản này hết tác dụng — ghi trong
  CHANGELOG. Không giữ cách tính cũ song song (hai cách cho cùng một việc).
- Khoá gốc nằm trong database thì vẫn tính ngược được (2^24 khối IPv4 là ít): không sửa được
  mà vẫn giữ "người lạ tự cài được không cần cấu hình". `SecretKey::is_configured`; tiến trình
  phục vụ API ghi cảnh báo lúc khởi động khi chưa đặt `AUTOXIDE_SECRET_KEY`. DEPLOY nhắc đặt
  khoá khi chạy thật.

Đã cân nhắc: hàm băm chậm (Argon2) cho dấu IP — bộ đếm chạy trên mọi lần gửi, quá chậm, và
2^24 lần Argon2 vẫn làm được; cắt dấu còn vài bit (k-ẩn danh) — lệnh cấm theo mạng chặn nhầm
người khác; bắt buộc `AUTOXIDE_SECRET_KEY` — người lạ không cài được nếu không đọc tài liệu.

## D-154 · `newsletter`: đăng ký công khai chỉ xếp job; việc theo trạng thái địa chỉ chạy nền — 2026-11-12 — agent quyết

Đóng giới hạn "đo thời gian trả lời đăng ký bản tin" của D-150: đường cần mail chèn thêm token
và một mail vào hàng đợi, đường "đã xác nhận" / "bị chặn" không ghi gì — đo thời gian là đoán
được địa chỉ nào đã có trong danh sách.

Quyết định: request `POST /public/subscribe` làm cùng một việc cho mọi địa chỉ — kiểm dữ liệu,
`SpamGuard::check`, rồi một transaction ngắn: xếp job `newsletter.subscribe` (email, tên, ngôn
ngữ, ID danh sách, bằng chứng đồng ý, lý do bị giữ, người gọi) và chèn nonce của token form.
Job (`apply_subscription`) làm phần còn lại như trước: người, đăng ký, event, token xác nhận,
bộ đếm mail và mail. Gửi lại cùng token không xếp job thứ hai. Job chạy lại thì như người đó
gửi lại yêu cầu (không thêm người; có thể thêm một mail xác nhận, vẫn trong giới hạn ngày).
Danh sách bị xoá giữa lúc nhận và lúc chạy job thì bỏ qua.

Hệ quả: mail xác nhận đi sau câu trả lời vài giây (chu kỳ của worker); cần worker chạy (mặc
định có). Job mang email và tên tới khi job queue dọn job đã xong — như job gửi mail. Yêu cầu
xoá theo email đến khi job còn chờ thì job vẫn tạo lại người đó, như một lần đăng ký mới ngay
sau khi xoá.

Đã cân nhắc: chờ thêm cho đủ một thời gian cố định — vẫn lộ khi máy chủ bận, và giữ kết nối
lâu hơn; luôn phát token / luôn xếp mail giả — ghi rác vào database và vẫn khác nhau ở phần
đổi đăng ký.

## D-155 · Sổ module đã cài: API chỉ đọc trong module nội bộ `kernel` — 2026-11-12 — chủ dự án quyết hướng, agent quyết cách làm

Bối cảnh: chủ dự án muốn "một CMS quản lý module". Theo D-096 / D-097 và luật kiến trúc 9,
giao diện quản trị là repo riêng; chủ dự án chọn làm trước phần API trong core mà màn hình đó
cần.

Quyết định:

- Module nội bộ `kernel` phục vụ sổ module: `GET /api/v1/kernel/modules` (mọi module theo thứ tự
  đăng ký, không chạm database) và `GET /api/v1/kernel/modules/{name}` (một module, kèm trạng thái
  từng migration đọc từ bảng `<tên>_migrations` của riêng module đó). Mỗi module nêu: tên, nội bộ
  hay không, vị trí khởi động, permission, event, event nhận, loại job, lịch, tool MCP, endpoint
  deprecated, ngôn ngữ có bản dịch, migration.
- Permission mới `kernel.modules.read`; tool MCP `kernel.list_modules`, `kernel.get_module` gọi cùng
  hàm với REST. Mã lỗi `kernel.module_not_found` (404).
- Danh sách trả `Data<Vec<…>>`, không phân trang — như `GET /api/v1/auth/permissions`: số module
  nhỏ, cố định lúc biên dịch.
- Sổ được điền ở cuối `Modules::start` (tool, job, lịch chỉ có sau `start`), cất trong
  `OnceLock` dùng chung giữa `Modules` và module `kernel`. Không thêm hàm nào vào trait `Module`:
  module cũ không phải sửa.
- **Chỉ đọc.** Không bật / tắt module lúc chạy: module đăng ký lúc biên dịch ở
  `crates/app/src/modules.rs`; muốn bật / tắt lúc chạy là thay đổi thiết kế lớn, cần chủ dự án mở.

Lý do: màn hình quản lý module (và AI agent) cần biết máy chủ có gì mà không phải đọc mã nguồn;
gom vào `kernel` vì đây là thông tin của cả chương trình, không của module nghiệp vụ nào, và
`kernel` đã là module nội bộ có sẵn (không phải dành thêm tên).

Đã cân nhắc: module nội bộ thứ ba tên `system` — thêm một tên cấm dùng mà không được gì; đặt trong
module `auth` — module nghiệp vụ không được biết các module khác; trả trạng thái migration ngay
trong danh sách — mỗi lần gọi đọc bảng lịch sử của mọi module (N query thừa trên đường hay gọi).

## D-156 · Bước 25 — `auth`: mật khẩu đổi thì mọi thứ mở bằng mật khẩu cũ đóng cùng lúc; cấp quyền kiểm lại trong transaction — 2026-11-12 — agent quyết

Rà soát độc lập module `auth` (bước 25). Các phát hiện được kiểm lại với code; sửa kèm test
chạy trên cả hai database.

Quyết định:

- **`UserService::write_password`** (đổi, đặt lại, quản trị viên đặt mật khẩu đầu) trong CÙNG
  transaction với việc ghi mật khẩu: thu hồi phiên (như cũ), xoá lượt chờ mã MFA, xoá nghi thức
  thêm passkey đang dở, thu hồi token đổi email đang chờ. Trước đây kẻ gian giữ mật khẩu cũ mở
  sẵn mail đổi email về địa chỉ của nó (sống 24 giờ) và chiếm lại tài khoản sau khi chủ thật đặt
  lại mật khẩu.
- **Đặt lại mật khẩu là một transaction** (`PasswordWrite::Recovery`): mật khẩu mới, thu hồi
  phiên, xoá passkey, API key, token đặt lại khác. Trước đây là bốn transaction; hỏng giữa chừng
  thì token đã dùng mà key cũ còn.
- **Cấp quyền mới kiểm lại điều đã chứng minh, trong transaction cấp** (`session::lock_grant`):
  câu `UPDATE auth_user … WHERE id = ? AND disabled_at IS NULL [AND password_hash = <hash vừa
  kiểm>]`; 0 dòng thì không cấp. Đăng nhập bằng mật khẩu, tạo API key so hash; hoàn tất MFA dùng
  hết lượt chờ trong cùng transaction; đăng nhập passkey kiểm passkey còn đó trong cùng
  transaction; thêm passkey dùng nghi thức và lưu passkey trong cùng transaction. Đổi / đặt lại
  mật khẩu ghi cùng những dòng đó, nên hai bên nối đuôi nhau (SQLite một bên ghi; Postgres khoá
  dòng): mật khẩu đổi xen giữa thì không cấp, cấp trước thì bị thu hồi cùng phần còn lại.
  `check_password` / `verify_credentials` trả kèm hash vừa kiểm (hash mới nếu vừa hash lại).
- **Bật TOTP đòi mật khẩu hiện tại** (`POST /me/mfa/totp` nhận `current_password`), như thêm
  passkey, tạo API key (D-079). Thay đổi phá vỡ của API `/api/v1` — ngoại lệ cho D-059 vì là lỗ
  hổng: access token bị lộ đủ để gắn ứng dụng authenticator của kẻ gian rồi khoá chủ thật ở
  ngoài (đặt lại mật khẩu không gỡ MFA). Cùng tiền lệ với D-150. Ghi trong CHANGELOG.
- **Xác nhận email mới thu hồi token đặt lại mật khẩu** đã gửi tới địa chỉ cũ.
- **Thứ tự khoá:** sửa / xoá người dùng khoá dòng người dùng trước rồi mới khoá role toàn quyền
  (`count_active_admins`), như `set_user_roles` — trước đây ngược nhau, Postgres có thể
  deadlock. Sửa người dùng chỉ đếm admin khi khoá tài khoản.
- **Dọn refresh token:** index `expires_at`, `revoked_at` (m0021); hai lệnh xoá thay cho một
  lệnh `OR`; xoá theo lô 1000 dòng (`DELETE … WHERE id IN (SELECT id … LIMIT n)`, chạy như nhau
  trên hai database). `revoke_user` chỉ thu hồi token chưa dùng — token đã dùng không đổi được
  nữa, và gửi lại nó vẫn bị phát hiện (dòng còn với `used_at`). Giữ token đã dùng tới hết hạn để
  phát hiện dùng lại (D-045), không rút ngắn.

Để lại, ghi rõ:

- **Đoán email qua thời gian trả lời** của `POST /password-reset` (địa chỉ có thật thì ghi token
  và xếp mail) và `auth.email_taken` ở `POST /me/email` (cần đăng nhập và mật khẩu, có throttling).
  `POST /register` vốn đã lộ email đã dùng khi bật tự đăng ký. Làm như D-154 (xếp job) nếu cần.
- **Lệnh CLI gỡ MFA** cho admin duy nhất mất thiết bị: chưa có; gỡ bằng một admin khác
  (`DELETE /users/{id}/mfa`) hoặc sửa database.
- Truy vấn tuần tự khi xác thực mỗi request (2 với phiên, 4 với API key): cần đo bằng
  `scripts/loadtest.sh` trước khi gộp.

## D-157 · Bước 25 — lớp HTTP, idempotency, webhooks, files: các phát hiện của lượt rà soát — 2026-11-12 — agent quyết

Rà soát độc lập lớp HTTP / MCP / quyền và phần dữ liệu, I/O của core (bước 25). Không có lỗi
vượt quyền hay lộ dữ liệu giữa người gọi; sửa các lỗi sau, kèm test chạy trên cả hai database.

Quyết định:

- **Địa chỉ client ở dạng chuẩn** (`IpAddr::to_canonical`) ngay khi giải: máy chủ nghe trên
  `[::]` thấy client IPv4 dưới dạng `::ffff:a.b.c.d`, và rate limiter gom IPv6 theo /64 đã gom
  MỌI client IPv4 vào một khối — một client làm cả nhóm bị 429. Rate limiter cũng tự chuẩn hoá.
  Giá trị header không đọc được chỉ ghi 64 ký tự đầu vào log.
- **Idempotency:** lỗi tạm (429, 408, 425) không lưu, như 5xx. Response mang dấu
  `OutcomeUnknown` (hết giờ ở `enforce_timeout`, panic ở `PanicToProblem`) — việc có thể đã xong
  — thì không bỏ cũng không lưu lỗi: lần gửi lại nhận `409 idempotency_result_unknown`. Lưu kết
  quả hỏng thì vẫn trả response thật (ghi log). Response lớn hơn 256 KiB không lưu
  (`409 idempotency_result_too_large` khi gửi lại). `If-Match` vào dấu vân tay của request;
  `ETag` được giữ khi trả lại.
- **Ký tự NUL** bị từ chối ở mọi cửa vào bằng một lỗi `400 invalid_character`: thân JSON
  (`ValidJson` đọc thân một lần, tìm `\u0000` không bị thoát), tham số tool MCP, đường dẫn và
  query (`%00`, một lớp cho mọi route), con trỏ trang (coi là con trỏ hỏng). Postgres không lưu
  được NUL trong cột chữ (500), SQLite lưu — hai database phải như nhau (luật số một).
- **`/health/ready`** dùng lại kết quả hỏi database trong 1 giây, nhiều request cùng lúc chỉ hỏi
  một lần: endpoint công khai, ngoài rate limit, mà mỗi lần hỏi chiếm kết nối ghi duy nhất của
  SQLite.
- **Nhãn `method` của metrics** chỉ nhận method chuẩn, còn lại là `other`: HTTP nhận method bất
  kỳ, mỗi method lạ thêm một chuỗi số đo sống mãi.
- **Webhooks:** tạo endpoint và đổi secret trả `Cache-Control: no-store` (quy ước D-149 — lớp
  idempotency không lưu secret). Event về endpoint chỉ mang origin của URL: đường dẫn / query của
  URL webhook (Slack, Discord…) thường là secret, mà event đi tới audit log và webhook khác nhận
  `*`. Bộ lọc địa chỉ chặn thêm: IPv4-compatible `::/96`, 6to4 `2002::/16` (xét IPv4 bên trong),
  NAT64 nội bộ `64:ff9b:1::/48`, site-local `fec0::/10`, Teredo `2001::/32`, `192.88.99.0/24`.
- **Files:** tên file không nhận ký tự định dạng vô hình (đảo chiều chữ…); tải về với
  `If-None-Match` khớp thì trả 304 mà không mở nội dung (với S3 là một lần GET bỏ dở).

Để lại, ghi rõ:

- **Audit giữ `before` / `after` của các dòng mà người bị xoá là người làm** (vd tên file họ
  tải lên): xoá đi thì mất dấu vết "ai đã làm gì với dữ liệu của người khác" — trách nhiệm quan
  trọng hơn ở audit. Chỉ dòng về chính tài khoản bị xoá mới bị bỏ dữ liệu (D-080, D-149).
- **Không có timeout đọc header HTTP** (`axum::serve` không gắn timer cho hyper): bố cục DEPLOY
  đặt Autoxide sau reverse proxy trên `127.0.0.1`, proxy lo việc này.
- Trang tài liệu `/api/docs` (tắt mặc định) tải script Scalar từ CDN không ghim phiên bản.
- Bản ghi idempotency (có thể chứa dữ liệu cá nhân trong response) không xoá theo tài khoản; sống
  tối đa 24 giờ.
- `/metrics` chạy trên cùng cổng với API: DEPLOY phải chặn đường dẫn này ở proxy khi bật.

## D-158 · Bước 25 — job queue, event, mail, lưu trữ, backup: các phát hiện của lượt rà soát — 2026-11-12 — agent quyết

Rà soát độc lập phần chạy nền và lưu trữ của core, và hiệu năng đường nóng (bước 25). Sửa kèm
test chạy trên cả hai database.

Quyết định:

- **Worker chạy liên tục, không theo lượt** (`Worker::run`): mỗi module một `Semaphore` có
  `concurrency` chỗ; vòng lặp claim đúng số chỗ trống của từng module, mỗi job một task giữ chỗ
  tới khi xong, rồi chờ (một job xong, 1 giây, hay lệnh dừng). Trước đây worker claim một lô
  của một module và chờ cả lô xong mới sang module khác: một job AI 270 giây giữ mail đặt lại mật
  khẩu chừng đó. `run_once` (test, công cụ) vẫn chạy một lượt và chờ. Claim SQL không đổi.
- **Job panic** là một lần lỗi (`catch_unwind`), thử lại rồi vào dead-letter queue. **Job đã
  chạy đủ `max_attempts` lần** không được claim lại (`attempts < max_attempts` trong điều kiện
  chờ); lượt dọn hằng giờ đưa job hết lượt mà lần cuối không ghi được kết quả (tiến trình chết)
  và đã hết hạn giữ vào dead-letter queue. Trước đây một job làm tiến trình hết bộ nhớ được chạy
  lại mỗi 5 phút, mãi mãi.
- **Thêm job trùng `unique_key`** dùng `INSERT … ON CONFLICT (unique_key) DO NOTHING` thay vì
  để lệnh lỗi rồi bắt lỗi: trong transaction, Postgres hỏng cả transaction sau một lệnh lỗi
  (`COMMIT` thành `ROLLBACK` mà không báo), SQLite thì không — hai database khác nhau.
- **Dispatcher event:** mỗi lần nhận việc giao dùng giờ hiện tại (trước đây dùng giờ đầu lô —
  lô chậm làm hạn giữ đã qua ngay khi ghi, tiến trình khác giao trùng). Một lần giao tối đa 50
  giây (ngắn hơn hạn giữ 60 giây); bên nhận panic hay quá giờ là giao lỗi, thử lại theo backoff,
  không làm chết dispatcher của cả tiến trình. Dòng event không đọc được bị đánh dấu hỏng hẳn
  rồi đi tiếp, không chặn mọi event sau nó.
- **Mail:** chỉ lỗi SMTP vĩnh viễn có mã mở rộng `5.1.x` / `5.2.x` (địa chỉ, hộp thư) mới là
  lỗi của người nhận và phát `mail.delivery_failed`. Lỗi 5xx khác (535 sai mật khẩu, địa chỉ gửi
  bị chặn, hết hạn mức 5.4.5, nội dung bị chặn) là lỗi bên gửi: thử lại rồi vào dead-letter
  queue. Không có mã mở rộng thì không đoán. Trước đây SMTP cấu hình sai giữa một chiến dịch làm
  bản tin chặn mọi người đăng ký.
- **Backup** tạo thư mục `0700` (Unix), cả thư mục tạm khi restore: backup chứa hash mật khẩu,
  secret webhook, MFA, và khoá ký khi nó nằm trong database.
- **Files:** lần ghi dở bị bỏ (request hết giờ, client ngắt) tự huỷ multipart upload trên S3
  (`object_store` không tự làm).
- **Hiệu năng:** index `audit_entry(module, id)` (m0003 của `audit`); `OneTimeTokens::purge`
  chỉ xoá theo `expires_at` (có index) — token đã dùng đi theo khi tới hạn, vài phút tới một
  ngày; tài liệu OpenAPI trả bằng `Bytes` (không chép hơn 1 MB mỗi request).

Để lại, ghi rõ (cần đo trước, theo luật "đo trước rồi mới tối ưu"):

- Xác thực hai lần cho request ghi có `Idempotency-Key` (lớp idempotency và handler); truy vấn
  tuần tự khi xác thực (D-156); `Throttle::hit` nhiều lệnh; mỗi event ba lệnh ghi; worker và
  dispatcher hỏi mỗi module mỗi giây khi rảnh; rate limiter một mutex; log ghi thẳng stderr.
  Đo bằng `scripts/loadtest.sh` trên Postgres trước khi đổi.
- `files` chưa có lượt quét nội dung mồ côi trong nơi lưu (như `media`); DEPLOY nên khuyên đặt
  lifecycle rule của S3 cho multipart upload dở.
- `autoxide jobs retry` một job `webhooks.send` đã hỏng không gửi lại (lượt gửi đã `failed`).

## D-159 · Lệnh `autoxide user reset-mfa`: người vận hành tắt xác thực hai bước — 2026-11-12 — agent quyết

Đóng một việc để lại của D-156: admin duy nhất mất thiết bị thì không ai tắt được xác thực hai
bước của họ qua API (`DELETE /users/{id}/mfa` cần một admin khác), chỉ còn sửa database.

Quyết định: `autoxide user reset-mfa --email …`, như `user create`, chạy với quyền của người vận
hành máy chủ (ai chạy được lệnh trên máy đã đọc được database). `auth` công bố trait mới
`api::AccountRecovery` (không gộp vào `Accounts` — việc khác, service khác), cài bằng
`MfaService::reset` với `Actor::system()`; event `auth.mfa_disabled` ghi `by_administrator`. Tài
khoản không có thì `auth.account_not_found`. Không đổi mật khẩu, không đăng xuất phiên: người dùng
đăng nhập bằng mật khẩu rồi bật lại.

## D-160 · `files`: job dọn nội dung mồ côi hằng ngày — 2026-11-12 — agent quyết

Đóng một việc để lại của D-158. Nội dung dưới `files/` có thể không còn dòng nào trỏ tới:
request bị huỷ giữa lúc ghi nội dung và lúc lưu dòng, xoá nội dung hỏng sau khi dòng đã xoá
(event xoá tài khoản giao lại không làm lại được), upload xong sau khi tài khoản đã bị xoá.

Quyết định: như `media` (`media.clean_orphans`): job `files.clean_orphans` chạy mỗi ngày (bảng
`files_jobs`, m0004), liệt kê khoá dưới `files/`, bỏ qua khoá mới hơn 24 giờ (tuổi đọc từ UUIDv7
— ID của file cũng là tên nội dung), kiểm theo lô 200 ID trên kết nối ghi, xoá khoá không có
dòng. Khoá dưới `files/` không phải UUID thì để nguyên, ghi cảnh báo. DEPLOY khuyên đặt
lifecycle rule của S3 cho multipart upload dở (lớp thứ hai cho D-158).

## D-161 · `webhooks`: chạy lại job gửi đã hỏng thì gửi lại thật — 2026-11-12 — agent quyết

Đóng một việc để lại của D-158: `autoxide jobs retry` một job `webhooks.send` đã vào
dead-letter queue không gửi gì — lần gửi đã `failed`, bên gửi chỉ gửi lần đang `pending`.

Quyết định: job chạy với `attempt == 1` (lượt đầu, hay lượt đầu sau `jobs retry` đặt lại số lần
thử) gặp lần gửi `failed` thì mở lại nó (`UPDATE … SET status = 'pending' WHERE status =
'failed'`, có điều kiện) rồi gửi như thường. Lượt đầu bình thường luôn gặp lần gửi `pending`,
nên điều kiện này chỉ khớp với lần chạy lại của người vận hành. Không thêm trạng thái hay cột mới.

## D-162 · Đặt lại mật khẩu: request chỉ xếp job, như đăng ký bản tin — 2026-11-12 — agent quyết

Đóng một việc để lại của D-156: `POST /password-reset` với email có thật thì tra tài khoản, ghi
token, ghi event, xếp mail; email lạ thì trả về sau một câu `SELECT` — đo thời gian là đoán được
email nào đã đăng ký.

Quyết định: như D-154. Request xếp job `auth.send_password_reset` (email, ngôn ngữ) — cùng một
lệnh ghi cho mọi email — rồi trả 202. Job (`PasswordResets::send`) làm phần cũ: tra tài khoản
đang hoạt động, giới hạn một mail mỗi phút, token, event, mail. Job xong thì job queue xoá payload
(có email) như mọi job (D-079). Mail đi sau câu trả lời vài giây; cần worker (mặc định có).

Không làm: `POST /me/email` báo `auth.email_taken` — cần đăng nhập và đúng mật khẩu (có
throttling), và `POST /register` vốn đã lộ email đã dùng khi bật tự đăng ký.

## D-163 · Đo xác thực mỗi request: chạy song song các câu đọc không có lợi, không làm — 2026-11-12 — agent quyết

Đóng việc "đo trước rồi mới tối ưu" để lại ở D-156 / D-158: xác thực mỗi request chạy tuần tự hai
câu đọc (access token: tài khoản, quyền theo role) hay bốn (API key).

Đã đo (`scripts/loadtest.sh`, Postgres 16 cùng máy, 4 CPU, 64 kết nối, 6 giây mỗi bài, hai lượt
xen kẽ trước / sau): bản thử chạy các câu đọc song song (`tokio::try_join!`) và ghi
`last_used_at` của API key ngoài request.

| Bài đo | Trước (request/giây) | Sau (request/giây) |
| --- | ---: | ---: |
| `GET /auth/me` (access token) | 4808 / 5606 | 4558 / 4716 |
| `GET /auth/me` (API key) | 3816 / 3752 | 3260 / 3226 |

Kết luận: không nhanh hơn, API key chậm hơn ~14%. Database cùng máy thì một vòng truy vấn rất
ngắn; lấy thêm kết nối từ pool và chạy hai future tốn hơn phần chờ tiết kiệm được. Theo luật "đo
trước rồi mới tối ưu", giữ cách tuần tự. Đáng đo lại khi database ở máy khác (vòng mạng dài) —
dùng `LOADTEST_BINARY` để so hai binary trên cùng máy.

Sửa kèm script đo: tắt giới hạn theo API key (`AUTOXIDE_HTTP_RATE_LIMIT_API_KEY_PER_MINUTE=0`,
D-109) — trước đó bài đo API key chỉ đo câu trả lời 429; thêm `LOADTEST_BINARY` để đo binary có
sẵn. Các tối ưu khác để lại ở D-158 (throttle, ghi event, hỏi khi rảnh, mutex rate limiter, log)
chưa có số đo cho thấy chúng là điểm nghẽn: không làm.

## D-164 · Autoxide là sản phẩm có cửa cho module; mượn cái hay của framework khác; core chuẩn để sau nâng thành framework — 2026-11-13 — chủ dự án quyết

Quyết định:

- **Giữ Autoxide là sản phẩm** (một binary, D-005), không công bố thành framework lúc này.
  Người khác viết được module mà không phải sửa core (cửa đã có: trait `Module`, D-038) — bước 27
  làm cửa đó dễ đi hơn.
- **Mượn cái hay của framework khác** khi nó hợp với luật trong `CLAUDE.md`: test harness dựng
  sẵn (Loco, Cot), lệnh sinh module (Loco / Rails), `doctor` (Loco), kiểm lúc khởi động do
  module khai (Django system checks), lệnh CLI do module khai (Loco tasks, Django management
  commands), module ở repo riêng (Django reusable apps, Rails engines). Danh sách và lý do:
  [`docs/research/frameworks.md`](research/frameworks.md).
- **Core chuẩn để sau này nâng thành framework rẻ:** hợp đồng module (kernel + testkit + các thư
  viện lộ ra trong nó) được liệt kê rõ; kiểu công khai sẽ lớn thêm thì đánh dấu
  `#[non_exhaustive]`; API Rust bỏ đi thì `#[deprecated]` trước một bản phát hành rồi mới xoá.
- Công bố thành framework (`publish` lên crates.io) chỉ khi chủ dự án quyết, sau khi: core xong
  lộ trình; có người ngoài thật sự viết module; `cargo-semver-checks` vào quy trình phát hành;
  luật của `cargo xtask check` chạy được trong repo của người viết module.

Lý do: thứ làm Autoxide mạnh là luật được gate kiểm trong repo này; ra khỏi repo thì mất. Ở sân
framework, Loco đã 1.0 và có cộng đồng; ở sân sản phẩm, Autoxide có lợi thế riêng (ranh giới
module do máy kiểm, hai database có test thật, OpenAPI + MCP cho AI agent). PocketBase cho thấy
"sản phẩm trước, chế độ thư viện sau" là con đường đã chạy được.

Đã cân nhắc: thành framework ngay — mỗi thay đổi của kernel thành thay đổi phá vỡ cho người
khác, phải hỗ trợ dải phiên bản Rust cũ, mất luật tự kiểm; re-export axum / SeaORM / utoipa từ
kernel — derive macro của chúng sinh đường dẫn cố định nên module vẫn phải khai trực tiếp; plugin
WASM, trang admin, auto-CRUD, ORM tự viết, DI lúc biên dịch — xem bản khảo sát.

## D-165 · Lệnh sinh module: chép khuôn `notes` rồi đổi tên — 2026-11-13 — agent quyết

Quyết định: `cargo xtask new-module <module> <entity>` (vd `bookmarks bookmark`) chép
`modules/notes` sang `modules/<module>`, trừ `benches/` (bộ đo của core, D-071), và thay sáu dạng
chữ `notes` / `note` / `Notes` / `Note` / `NOTES` / `NOTE` bằng dạng tương ứng của tên mới trong
nội dung và tên file, trong **một lượt quét** (tên mới chứa `note` không bị thay lần hai). Mô tả
trong `Cargo.toml` và chú thích đầu `lib.rs` được viết lại. Tên được kiểm theo D-038 (chữ thường,
số, `_`; không trùng; không là tiền tố bảng của module khác; không là tên dành riêng) và số ít phải
khác số nhiều. Lệnh không tự đăng ký module vào binary: nó in ba dòng cần thêm (như hướng dẫn).

Lý do: `notes` đã dùng đủ hợp đồng module và đi qua gate mỗi lần chạy, nên khuôn không cũ đi âm
thầm. Test `sinh_tu_khuon_notes_that` sinh thật từ `notes` mỗi lần gate chạy: khuôn đổi hình dạng
(mất header, đổi phần bench) thì test đỏ trước khi người dùng gặp lỗi. Đã thử sinh `bookmarks`
rồi chạy clippy, `cargo xtask check` và test trên cả hai database: xanh.

Đã cân nhắc: bộ template riêng có chỗ điền (`{{name}}`) — hai nguồn sự thật, template dễ hỏng mà
không ai biết vì nó không được biên dịch; `cargo-generate` — thêm công cụ ngoài cho người dùng
cài; tự sửa `Cargo.toml` và `modules.rs` — `notes` cố ý không đăng ký trong binary (core trung lập),
và người viết module mới nên tự quyết thứ tự đăng ký.

## D-166 · `autoxide doctor` và `Module::checks` — 2026-11-13 — agent quyết

Quyết định:

- Kernel có `autoxide_kernel::doctor`: `Check` (tên ngắn, mô tả, `Arc<dyn Probe>`), `Probe`
  (`async fn probe() -> Finding`), `Finding` (`ok` / `warning` / `failed`, câu tiếng Anh, gợi ý
  sửa). Mỗi bước tối đa 10 giây. Module khai qua `Module::checks` (mặc định rỗng, D-038); kernel
  thêm tiền tố `<module>.`. Bước kiểm không thay đổi dữ liệu thật — thứ ghi thử thì dọn ngay.
- Lệnh `autoxide doctor [--json]`: cấu hình → database (ping cả kết nối đọc và ghi) → migration
  → khởi động module → khoá ký (D-153) → bước kiểm của module. Bước nền hỏng thì bước sau báo
  `skipped`, để thấy đúng một nguyên nhân. Mã thoát: 0 khi không có bước `failed` (cảnh báo vẫn
  đạt), 1 khi có, 2 khi cấu hình sai (như mọi lệnh).
- Bước kiểm đầu tiên: `auth.administrator` (chưa có admin hoạt động → cảnh báo kèm lệnh tạo),
  `files.storage` (ghi, đọc lại, xoá một file thử), `mail.smtp` (SMTP trả lời `NOOP`; chưa cấu
  hình thì cảnh báo mail chỉ ghi ra file).
- Không gọi nhà cung cấp AI: `autoxide ai check` đã có và tốn token; doctor không được tốn tiền.

Lý do: người lạ tự cài phải tự biết mình thiếu gì (mục tiêu ở `CLAUDE.md`); AI agent vận hành
cần kết quả có cấu trúc (`--json`). Mượn `cargo loco doctor` và system checks của Django (D-164).

Phát hiện kèm: bước `files.storage` lộ ra một lỗi có sẵn — ghi một file nhỏ thất bại (thư mục
không ghi được, S3 từ chối) thì `Upload::finish` gọi `BufWriter::abort` sau `shutdown`, và
object_store 0.14 panic ("Already shut down") ở trạng thái đó. Đã bỏ lời gọi `abort` sau
`shutdown`; test `noi_luu_khong_ghi_duoc_thi_hong_kem_goi_y` giữ lỗi không quay lại.

Đã cân nhắc: gộp vào `/health/ready` — health check phải rẻ và chạy liên tục, còn doctor ghi thử
file và nối SMTP; báo lỗi đầu tiên rồi dừng — người cài phải chạy lại nhiều lần mới thấy hết.

## D-167 · Hợp đồng module: liệt kê thư viện công khai, `#[non_exhaustive]`, chính sách deprecate — 2026-11-13 — agent quyết

Theo D-164. Rà soát (một agent chỉ đọc, kiểm lại bằng trình biên dịch) mọi struct có field `pub`
và mọi enum công khai của kernel.

Quyết định:

- `docs/MODULE_GUIDE.md` (EN + VI) có mục "Stability of the module contract": bảng thư viện có
  kiểu lộ ra trong hợp đồng (axum 0.8, sea-orm / sea-orm-migration 2, utoipa 6, utoipa-axum 0.3,
  tokio 1, serde 1, uuid 1, chrono 0.4, async-trait 0.1, validator 0.21); thư viện trong bảng lên
  phiên bản lớn = thay đổi phá vỡ hợp đồng (bản minor khi còn `0.x`, ghi ở **Changed**); `Module`
  chỉ thêm hàm có thân mặc định (D-038); thứ sắp bỏ thì `#[deprecated]` ít nhất một bản phát hành
  trước khi xoá. Thêm mục "A module in its own repository" (phụ thuộc kernel qua git tag).
- Đánh dấu `#[non_exhaustive]` ngay cho kiểu sẽ lớn thêm mà không chỗ nào ngoài kernel dựng bằng
  literal hay `match` đủ nhánh: `Config`, `DatabaseConfig`, `HttpConfig`, `ProcessConfig`,
  `RateLimit`, `Setting`, `FieldError`, `BodyLimit`, `Deprecation`, `Health`, `Api`, `ClaimedJob`,
  `Schedule`, `mail::Rendered`, `MigrationState`, `Created`, `Problem`, `ProblemField`,
  `tokens::{Issued, Consumed, Recent}`, `IssuedToken`; enum `Effort`, `EmbedKind`, `FailMode`,
  `ChallengeKind`, `ChallengeOutcome`, `Classification`, `CorsOrigins`, `LogFormat`,
  `ProcessRole`, `ErrorKind`, `Language`, `idempotency::Begin`, `JobStatus`, `SortValue`,
  `IfMatch`.
- **Cố ý để exhaustive:** `ai::Part`, `antispam::Verdict` (bên dùng phải xử lý từng loại; nhánh
  `_` sẽ lặng lẽ bỏ dữ liệu hay cho spam lọt), `DatabaseBackend` (hai database là luật, D-015),
  envelope `Data`, `Page`, `Tagged` và newtype `ValidJson` (hình dạng là hợp đồng API), các tập
  đóng (`Order`, `LogLevel`, `Origin`, `doctor::Status`, `Disposition`, `Shutdown`,
  `throttle::Outcome`, `antispam::Claim`).
- **Còn lại (mục riêng của bước 27):** module đang dựng bằng literal trong code chạy thật, cần
  hàm tạo / builder trước rồi mới `#[non_exhaustive]`: `ai::{Prompt, Usage, Generation,
  Capabilities}`, `antispam::{Submission, Limits}`, `event::Actor`, `http::Extras`,
  `idempotency::StoredResponse`, `job::EnqueueOptions`, `mail::Mail`, `mcp::PageInput`,
  `pagination::{PageQuery, Pagination}`, `resource::Resource`, `storage::StoredFile`,
  `config::ConfigError`; và chỉ test dựng bằng literal: `event::Delivered`, `job::{JobContext,
  JobRecord}`, `http::ClientInfo`, `AntispamConfig`, `LogConfig`, `TraceConfig`, `CopiedTable`.

Lý do: đang ở `0.x`, thêm `#[non_exhaustive]` bây giờ rẻ; sau `1.0` mỗi field hay variant mới của
những kiểu này mới không còn là thay đổi phá vỡ. Làm phần không đụng module trước để mỗi bước nhỏ
và gate xác nhận được.

Đã cân nhắc: đánh dấu hết một lượt — phải sửa ~100 chỗ trong 12 module cùng lúc; `cargo
semver-checks` ngay trong gate — khi còn `0.x` mọi bản minor được phép phá vỡ nên công cụ chỉ bắt
được lỗi ở bản vá; để dành cho lúc chủ dự án quyết công bố framework (D-164).

## D-168 · Bản công khai kèm ảnh trong thân bài (`body_media`) — 2026-11-13 — agent quyết

Bối cảnh: thử một bài viết có ảnh 6000 × 4000 trong thân bài. Ảnh đại diện có `srcset` vì bản
công khai trả `featured_media` kèm kích thước và các bản nhỏ; ảnh trong thân bài thì node `image`
chỉ có `mediaId`, và khách không đọc được `GET /api/v1/media/{id}`. Website vì thế chỉ biết địa
chỉ bản gốc: mọi thiết bị tải ảnh gốc (đo được 4,2 MB cho một ảnh), không giữ được chỗ cho ảnh
(thiếu chiều rộng, chiều cao) và địa chỉ không mang phiên bản nên chỉ cache 60 giây.

Quyết định: `PublicEntryResponse` có thêm `body_media`: các ảnh mà node `image` trong `body`
dùng, theo thứ tự xuất hiện, không lặp, cùng dạng `PublicMedia` với `featured_media` (địa chỉ
có phiên bản, kích thước, các bản nhỏ). Ảnh không còn công khai thì không có trong danh sách,
như `featured_media`. `alt` ở đây là của thư viện media; node `image` vẫn mang `alt` riêng.

Cách làm: `public_views` gom ID ảnh trong thân bài của mọi dòng vào cùng lượt `Media::find_many`
đã có cho ảnh đại diện và ảnh tác giả: số câu query không đổi theo số bài. Thân bài không có
chữ `"image"` thì không đọc JSON.

Lý do: thêm trường, không đổi trường cũ, nên không phá client nào; website không cần quyền gì
thêm; core đã có sẵn mọi dữ liệu.

Đã cân nhắc: chép kích thước vào node `image` lúc lưu — cũ đi khi ảnh bị thay nội dung, và
không có các bản nhỏ (job nền làm sau khi tải lên); mở `GET /api/v1/media/{id}` cho khách với
ảnh công khai — website phải gọi thêm một lần cho mỗi ảnh; website tự đoán `?w=` — sai với ảnh
nhỏ hơn cỡ xin, và vẫn thiếu kích thước.

Chưa làm (ghi lại để chủ dự án quyết, đo trong cùng lần thử): các bản nhỏ chỉ có JPEG / PNG.
Với ảnh thử, WebP nhẹ hơn JPEG cùng cỡ khoảng 60–70%, AVIF khoảng 70–80%; bản nhỏ PNG của ảnh
có vùng trong suốt nặng gần bằng ảnh gốc (1,2 MB ở 1920 px, trong khi WebP có alpha khoảng
55 KB). Thêm định dạng mới cần thêm thư viện mã hoá (xem ghi chú đầu `variants.rs`).

## D-169 · `autoxide task` và `Module::tasks`; mọi module test HTTP qua `TestApp` — 2026-11-13 — agent quyết

Quyết định:

- Kernel có `autoxide_kernel::task`: `Task` (tên ngắn, mô tả, danh sách `TaskArgument` — tên,
  mô tả, bắt buộc hay không — và một `TaskRun`), `TaskArgs` (đã kiểm: chỉ tên đã khai, đủ tham
  số bắt buộc, không trùng; `flag()` đọc `true` / `false`). Module khai qua `Module::tasks` (mặc
  định rỗng, D-038); kernel thêm tiền tố `<module>.` và kiểm tên, mô tả, tham số lúc liệt kê.
- Lệnh `autoxide task` liệt kê; `autoxide task <module>.<lệnh> [tên=giá_trị …]` chạy, cần đủ
  migration. Tham số dạng `tên=giá_trị` (không dùng cờ `--tên`): module không phải phụ thuộc
  `clap`, và AI agent gọi được theo đúng một mẫu. Mã thoát: 0 xong, 64 gọi sai (như `EX_USAGE`),
  1 lệnh hỏng, 2 cấu hình sai. Lệnh chạy với quyền người vận hành máy chủ (như `user create`) và
  gọi service — không có logic riêng (luật 8).
- Lệnh đầu tiên: `media.refresh_variants [dry_run=true]` — đổi `AUTOXIDE_MEDIA_IMAGE_WIDTHS` thì
  ảnh cũ giữ bộ ảnh thu nhỏ cũ; lệnh duyệt ảnh theo lô 200, xếp job `media.make_variants` sẵn có
  cho ảnh có bản ở cỡ đã bỏ hoặc thiếu một cỡ chắc chắn làm được (nhỏ hơn cả hai cạnh — cỡ nằm
  giữa hai cạnh có thể bị bỏ khi xoay theo EXIF, không tính là thiếu, để chạy lại không xếp mãi).
  Khoá chống trùng `refresh:<item>:<phiên bản>`: chạy hai lần trước khi worker kịp làm không xếp
  trùng. Bản ở cỡ đã bỏ nằm lại nơi lưu tới khi job dọn mồ côi của `media` xoá.
- Các lệnh CLI sẵn có (`user`, `jobs`, `ai check`) giữ nguyên: đổi chúng thành `task` là thay
  đổi phá vỡ CLI mà không thêm lợi ích.
- Test HTTP / MCP của mọi module đi qua `TestApp` (trừ `notes/benches`, đo đường thô, và test
  của chính lớp HTTP trong `crates/kernel/tests`). `TestApp` có thêm `principal(token, Principal)`
  (bên gọi qua API key) và `TestRequest::body(Body)` (gửi theo từng đoạn). Module `auth` vẫn dựng
  `Services` riêng cạnh module thật (cùng database, khoá ký, cấu hình) cho test mức service.

Lý do: mượn tasks của Loco / management commands của Django (D-164) — thêm việc quản trị không
phải sửa binary. Bỏ khoảng 600 dòng dựng app lặp lại trong test của 10 module.

Đã cân nhắc: tham số kiểu `--tên giá_trị` qua `clap` cho từng lệnh — module phải phụ thuộc
`clap` và khai kiểu riêng; tự chạy `refresh_variants` khi khởi động thấy cấu hình đổi — phải lưu
cấu hình cũ, và việc nặng chạy ngầm lúc khởi động khó đoán.

## D-170 · Hàm tạo cho các kiểu module dựng bằng literal, rồi `#[non_exhaustive]` — 2026-11-13 — agent quyết

Đóng phần "còn lại" của D-167. 25 kiểu (`ai::{Prompt, Usage, Generation, Capabilities}`,
`antispam::{Submission, Limits, AntispamConfig}`, `event::{Actor, Delivered}`,
`http::{Extras, ClientInfo}`, `StoredResponse`, `job::{EnqueueOptions, JobContext, JobRecord}`,
`Mail`, `PageInput`, `PageQuery`, `Pagination`, `Resource`, `StoredFile`,
`config::{ConfigError, LogConfig, TraceConfig}`, `CopiedTable`) có hàm tạo và được đánh dấu
`#[non_exhaustive]`; mọi chỗ dựng ngoài kernel (kể cả `crates/kernel/tests` và benchmark) đã đổi.

Quy ước cho hàm tạo của kernel (áp cho kiểu mới về sau): `new(…)` nhận các trường người gọi luôn
phải cho; setter nối chuỗi mang đúng tên trường, không tiền tố `with_`, nhận giá trị của trường và
trả `Self` (như `Deprecation::new().sunset()` đã có); `const fn` khi được.

Mặc định đã chọn: `Prompt::new` dùng `Effort::Low` và `Prompt::DEFAULT_MAX_OUTPUT_TOKENS = 1024`;
mọi chỗ gọi hiện có vẫn ghi tường minh `effort` và `max_output_tokens` (mỗi chỗ một giới hạn khác).
`Actor::api_key(user_id, api_key_id)` nhận `Uuid`, không nhận `Option` — chỗ có thể không có key
rẽ nhánh sang `Actor::user`. Setter nhận giá trị trơn (`unique_key`, `run_at`, `html`,
`dedupe_key`): chỗ có `Option` chỉ gọi setter khi có giá trị — hành vi giữ nguyên.

Kiểm: hành vi không đổi (cả 795 test xanh trên SQLite và trên Postgres, không test nào bị xoá),
`docs/openapi.json` không đổi một byte.

## D-171 · Agent tự phát hành sau mỗi bước lộ trình xong — 2026-11-13 — chủ dự án quyết

Quyết định: xong trọn một bước của `docs/ROADMAP.md` (mọi mục `[x]`, gate xanh) thì agent tự phát
hành, không hỏi: đưa nhánh lên `main` mới nhất, gate xanh lại, commit `release: vX.Y.Z`, PR rồi
merge vào `main`, gắn tag, đẩy tag, tạo bản phát hành GitHub với ghi chú từ `CHANGELOG.md`, rồi
`cargo clean`. Các bước cụ thể và cách xử lý khi hai phiên đụng nhau ở `docs/RELEASING.md`.

- Chỉ phát hành ở ranh giới bước, không phát hành giữa bước; không phát hành khi `[Unreleased]`
  trống.
- Số phiên bản theo luật SemVer sẵn có của `RELEASING.md` (D-078).
- Image Docker vẫn chỉ đăng khi có token của chủ dự án (D-107): thiếu token thì bỏ qua bước đó,
  không dừng cả đợt phát hành.
- Thiếu quyền đẩy lên GitHub (chưa đăng nhập `gh`, không có `GH_TOKEN`) thì dừng và báo, không
  tìm đường vòng.

Lý do: chủ dự án muốn agent làm xuyên suốt; trước đây mỗi bản phát hành phải chờ họ giao, nên
`main` và bản phát hành đi sau code đã xong. Phát hành theo bước giữ nhịp đều và mỗi bản ứng với
một phần lộ trình đọc được.

Đã cân nhắc: phát hành sau mỗi commit — quá nhiều bản, và thay đổi phá vỡ của một bước bị chia
nhỏ qua nhiều bản; bật lại workflow `Release` trên GitHub cho tự chạy khi đẩy tag — tốn phút chạy
của repo riêng tư (D-107, chủ dự án quyết); phát hành theo lịch cố định — bản có thể chứa bước làm
dở.

Bổ sung: D-078 (quy trình), D-107 (phát hành chạy trên máy). Thay luật "không push khi chủ dự án
chưa yêu cầu" trong `CLAUDE.md` cho trường hợp phát hành.

## D-172 · Build dev và test chỉ giữ thông tin dòng code (`debug = "line-tables-only"`) — 2026-11-13 — agent quyết

Quyết định: `Cargo.toml` gốc khai `[profile.dev] debug = "line-tables-only"` (profile `test` kế
thừa). Backtrace và thông báo panic vẫn chỉ đúng file, dòng; mất thông tin biến cho debugger.
Ai cần debugger xem biến thì tạm đổi thành `debug = true` trên máy mình, không commit.

Lý do: thông tin debug đầy đủ chiếm phần lớn kích thước file test và thời gian link. Dự án có
khoảng 104 nghìn dòng Rust, 518 thư viện, hơn 40 chương trình test, mỗi chương trình gói trọn
thư viện. Đo ngày 2026-11-13 trên một máy dev Linux 12 nhân, cùng commit, trước và sau:

| Đo                                      | `debug = true` (mặc định) | `line-tables-only` |
| --------------------------------------- | ------------------------- | ------------------ |
| `./gate.sh` sau `cargo clean`           | 566 s                     | 422 s              |
| Trong đó build bản test                 | 254 s                     | 146 s              |
| Sửa `kernel`, `cargo test --no-run`     | 257 s                     | 43 s               |
| `target/` sau một lần gate              | 23 GB                     | 10 GB              |

Đã cân nhắc: `debug = false` — nhanh hơn nữa nhưng backtrace mất số dòng, test đỏ khó tìm chỗ
hỏng; trình link `mold` — người lạ clone về phải cài thêm; gộp các file `tests/*.rs` của mỗi
crate thành một chương trình — giảm số lần link nhưng phải dời nhiều file, để sau nếu cần.

## D-173 · Gate hai mức: `./gate.sh --quick` lúc dev, gate đầy đủ trước khi merge và phát hành — 2026-11-13 — chủ dự án quyết

Quyết định:

- `./gate.sh --quick` — trước mỗi commit trong lúc làm một bước: fmt, clippy, `cargo xtask
  check`, `cargo deny`, gitleaks như gate đầy đủ, nhưng test chỉ chạy cho crate bị sửa so với
  `origin/main` và mọi crate phụ thuộc vào chúng, **chỉ trên SQLite**, không cần Docker
  (`cargo xtask test-affected`). Sửa file dùng chung (`Cargo.toml` gốc, `Cargo.lock`,
  `rust-toolchain.toml`, `.cargo/`…) thì chạy test mọi crate; chỉ sửa tài liệu thì không chạy
  test nào.
- `./gate.sh` (đầy đủ, như trước) — bắt buộc khi xong một bước, trước khi merge vào `main` và
  trước khi phát hành (D-171). Luật số một không đổi: `main` và mọi bản phát hành luôn xanh
  trên cả SQLite và Postgres.
- Cả hai mức chạy ở ưu tiên CPU thấp (`renice -n 10`): việc khác trên máy chạy trước.

Lý do: chủ dự án thấy mỗi lần dev mà chạy gate đầy đủ thì tốn thời gian và làm máy giật. Sau
D-172, phần tốn nhất của gate không còn là build mà là chạy toàn bộ test hai lần. Đo ngày
2026-11-13 trên một máy dev Linux 12 nhân, cache build còn nóng:

| Thay đổi                   | `./gate.sh` đầy đủ        | `./gate.sh --quick`                              |
| -------------------------- | ------------------------- | ------------------------------------------------ |
| Không phải build lại gì    | 186 s                     | —                                                |
| Sửa một module (`content`) | trên 186 s                | khoảng 27 s (kiểm tĩnh 10–13 s, build và test 16–17 s) |
| Sửa `kernel`               | trên 186 s, cộng build lại cả workspace | khoảng 167 s (clippy 12 s, build và test cả 17 crate trên SQLite 155 s) |

Sửa `kernel` (hay `testkit`) thì mọi crate phụ thuộc nên gate nhanh chỉ bớt được phần Postgres.

Đánh đổi chủ dự án chấp nhận: lỗi chỉ có trên Postgres lộ ra lúc xong bước thay vì lúc commit.

Cách chọn test: không dùng `cargo test -p <crate>`, vì chọn vài crate thì cargo hợp feature
thư viện khác lúc build cả workspace và build lại sqlx, sea-orm… thành bộ file thứ hai (đo:
87 s chỉ cho `content`); cách gom feature cả workspace (`resolver.feature-unification`) còn là
tính năng nightly ở cargo 1.99. `test-affected` build cả workspace (chung cache với gate đầy
đủ) rồi chỉ chạy các file test của crate được chọn. Lệnh này bỏ các biến `cargo run` đặt cho
xtask (`CARGO_PKG_*`, `CARGO_MANIFEST_DIR`…) trước khi gọi cargo: build script của `ring` theo
dõi chúng, để nguyên thì mỗi lần đổi qua lại giữa `cargo xtask` và cargo gọi thẳng lại build
lại khoảng 36 crate. Doc-test không chạy ở gate nhanh (hiện workspace không có doc-test nào).

Đã cân nhắc: gate nhanh chạy cả Postgres cho crate bị sửa — chậm hơn khoảng một phút mỗi lần và
cần Docker, chủ dự án chọn chỉ SQLite; `cargo-nextest` hoặc `cargo-hakari` — thêm công cụ
người lạ phải cài; giới hạn số job của cargo (`CARGO_BUILD_JOBS`) — chậm hẳn kể cả khi máy
rảnh, `renice` chỉ nhường CPU khi có việc khác cần.

Bổ sung: luật Gate trong `CLAUDE.md`; D-171 (phát hành vẫn cần gate đầy đủ).

## D-174 · Một cách kiểm email cho dữ liệu người đọc gửi lên: `autoxide_kernel::email` — 2026-11-13 — agent quyết

Quyết định: `autoxide_kernel::email` có `normalize`, `is_valid`, `checked` và `invalid`; `forms`,
`comments`, `newsletter` dùng nó thay cho ba bản tự viết. Luật: cả địa chỉ tối đa 254 byte
(RFC 5321), phần trước `@` 1–64 byte, tên miền có ít nhất một dấu chấm, mỗi nhãn 1–63 byte chữ,
số hoặc `-`; độ dài đếm theo byte như bên gửi mail (`lettre`). `auth` chỉ dùng chung `normalize`
và giữ cách kiểm của `validator` cho tài khoản.

Lý do: lượt rà soát kiến trúc ngày 2026-11-13 thấy ba bản đã lệch nhau — `comments` đếm ký tự,
`forms` và `newsletter` đếm byte; `newsletter` nhận địa chỉ tới 320 byte (không gửi mail tới được),
`forms` giới hạn 254 — nên cùng một địa chỉ có chỗ nhận, chỗ báo sai. `auth` giữ cách riêng vì
tài khoản cần nhận địa chỉ nội bộ (`admin@localhost`) mà luật trên từ chối; đổi sẽ làm API tạo
tài khoản từ chối thứ trước đây nhận (thay đổi phá vỡ).

Đã cân nhắc: dùng `validator` cho cả ba module — nhận tên miền không dấu chấm, không hợp với form
công khai; thư viện email đầy đủ RFC 5322 — thêm thư viện cho vài chục dòng.

## D-175 · Công khai repo; bật lại CI và phát hành tự động trên GitHub — 2026-11-13 — chủ dự án quyết

Quyết định: repo `autoxide` chuyển sang công khai (đúng mục tiêu "mã nguồn mở, công khai toàn
bộ" trong `CLAUDE.md`). Khi đã công khai:

- Ba workflow tự chạy lại: `CI` theo push lên `main` và pull request, `Security audit` theo lịch,
  `Release` khi đẩy tag `vX.Y.Z` — Release build và đăng image Docker lên GitHub Container
  Registry bằng `GITHUB_TOKEN` của workflow, không cần token cá nhân, gói image để công khai.
- Bật secret scanning và push protection của GitHub (miễn phí cho repo công khai), giữ gitleaks
  trong `./gate.sh` (D-108).
- `./gate.sh` trên máy vẫn là định nghĩa "xong" (D-173); CI là lưới thứ hai.

Lý do: repo công khai thì GitHub Actions miễn phí, nên lý do tắt CI (D-107) không còn; người lạ
cần image Docker để cài mà không phải dựng toolchain Rust — README đã hứa image ở
`ghcr.io/<owner>/autoxide` nhưng chưa bản nào được đăng.

Thay: D-107. Việc làm cụ thể: bước 30 của `docs/ROADMAP.md`.

## D-176 · Ảnh thu nhỏ định dạng WebP trong module `media` — 2026-11-13 — chủ dự án quyết

Quyết định: module `media` sinh thêm bản nhỏ định dạng WebP bên cạnh JPEG / PNG, và được thêm
thư viện mã hoá WebP nếu thư viện đang có không đủ. Chọn thư viện, cách website nhận WebP (vd
`variants` kèm định dạng, hay theo header `Accept`) và làm lại ảnh đã có (`autoxide task
media.refresh_variants`) do agent làm bước 30 quyết và ghi thành mục mới.

Lý do: đo ở D-168 — WebP nhẹ hơn JPEG cùng cỡ khoảng 60–70%; bản nhỏ PNG của ảnh có vùng trong
suốt 1,2 MB ở 1920 px, WebP có alpha khoảng 55 KB.

Cần kiểm trước khi chọn (skill `check-library-api`, đúng phiên bản ghim): `image` 0.25 (đã có,
feature `webp`) giải mã được WebP, nhưng bộ mã hoá của nó có thể chỉ có kiểu không mất dữ liệu;
kiểu nén mất dữ liệu (cho số đo trên) có thể cần thư viện khác, vd bọc `libwebp` bằng C — xét
license qua `cargo deny` và việc build image Docker. Thêm thư viện thì ghi `STACK.md`.

## D-177 · SDK TypeScript nằm trong repo, không đăng npm — 2026-11-13 — chủ dự án quyết

Quyết định: SDK TypeScript sinh từ `docs/openapi.json` (nguồn sinh SDK theo D-004) nằm trong repo
này, không đăng lên npm. Giao diện (repo riêng, D-097) và người ngoài dùng nó bằng cách chép hoặc
trỏ tới thư mục / bản phát hành trên GitHub. Công cụ sinh, thư mục và cách gate kiểm SDK khớp hợp
đồng do agent làm bước 32 quyết và ghi thành mục mới.

Lý do: đăng npm cần tài khoản và token của chủ dự án và thêm một kênh phát hành phải giữ; chưa có
người dùng ngoài nào cần cài qua npm.

Đã cân nhắc: đăng npm — để sau, khi có người dùng ngoài cần; không làm SDK, mỗi client tự sinh —
mỗi bên một bản lệch nhau, đúng thứ D-004 muốn tránh.

## D-178 · Gom bước 30–36 của lộ trình thành 30–33 — 2026-11-13 — chủ dự án quyết

Quyết định: bảy bước 30–36 (viết cùng ngày, theo lúc nghĩ ra) gom thành bốn bước theo khu vực code
và theo điều phải chờ; không bỏ mục nào. Mục cũ nhắc số bước (D-175, D-176, D-177) đọc theo bảng:

| Bước cũ                                   | Bước mới                                   |
| ----------------------------------------- | ------------------------------------------ |
| 30 nhóm A, WebP, nhóm B                   | 30 "Ra mắt công khai", nhóm A và C         |
| 34 (file mã nguồn mở)                     | 30, nhóm B                                 |
| 35 image ARM64                            | 30, nhóm A                                 |
| 32 (cho người gọi API)                    | 31 "Hợp đồng API và chống hồi quy", nhóm A |
| 36 (chống hồi quy)                        | 31, nhóm B                                 |
| 30 nhóm C (bố cục)                        | 32 "Dọn bố cục và việc để lại", nhóm A     |
| 35 còn lại (email, passkey, CSV, Markdown)| 32, nhóm B                                 |
| 31 (vận hành sau ra mắt)                  | 33 "Vận hành và hiệu năng", nhóm A         |
| 33 (hiệu năng ở quy mô thật)              | 33, nhóm B                                 |

Lý do: bước 30 và 34 cùng sửa GitHub, CI, phát hành; 32 và 36 cùng sửa `xtask`, `testkit` và hợp
đồng API; 30 nhóm C và 35 cùng sửa module sau khi bước 29 merge; 31 và 33 cùng cần bố cục mới và
dùng chung số đo. Ít bước hơn thì ít lần phát hành và ít chỗ hai người sửa chung một file.

## D-179 · Bản release kiểm tràn số nguyên (`overflow-checks = true`) — 2026-11-13 — agent quyết

Quyết định: `[profile.release] overflow-checks = true` trong `Cargo.toml` gốc. Bản release
(binary, Docker image) panic khi số nguyên tràn — thành lỗi 500 có log — thay vì quay vòng.

Lý do: mặc định Rust chỉ kiểm tràn ở bản dev; bản build thật cộng tràn `i64` sẽ âm thầm thành
số âm (số dư, bộ đếm). Với nghiệp vụ tiền, dữ liệu sai lặng lẽ tệ hơn một request lỗi. Tiền đi
qua `Money` (D-180) còn trả lỗi trước cả khi tràn; cờ này chặn mọi phép tính khác.

Số đo: `scripts/loadtest.sh` trên SQLite, cùng một máy Linux 4 CPU (máy tạo tải chạy chung), cùng
code, chỉ khác cờ này, đo liền nhau (request/s, 64 kết nối trừ khi ghi khác):

| Bài đo                              | Không kiểm | Có kiểm |
| ----------------------------------- | ---------: | ------: |
| GET /health/ready                   |      16552 |   19746 |
| GET /auth/me (access token)         |       6179 |    5033 |
| GET /auth/me (API key)              |       4180 |    3974 |
| GET /auth/users?limit=50            |       2219 |    2041 |
| GET /audit?limit=50                 |       1264 |    1272 |
| GET /files/{id}/content (16 KiB)    |       4823 |    4449 |
| MCP tools/call auth.whoami          |       4532 |    5206 |
| PATCH /auth/users/{id} (ghi + event) |      1291 |    1328 |
| POST /files (4 KiB, ghi)            |       1729 |    1534 |
| đọc, 1 / 256 / 1024 kết nối         | 1317 / 4765 / 4739 | 1449 / 4644 / 4854 |
| ghi, 1 / 256 / 1024 kết nối         | 620 / 1209 / 1275 | 555 / 1182 / 1312 |

Chênh lệch đi cả hai chiều, trong khoảng ±20% — bằng độ dao động giữa hai lần đo cùng một
binary trên máy này; không thấy chi phí đo được. Bài `POST /auth/login` không so được: nó đo
login throttling, số request lọt qua trước khi bị chặn tuỳ thời điểm.

Đã cân nhắc: chỉ dùng `checked_*` ở chỗ tính tiền — mọi phép cộng khác (bộ đếm, kích thước,
thời gian) vẫn quay vòng lặng lẽ; `wrapping`/`saturating` — đổi lỗi thành dữ liệu sai.

## D-180 · Tiền là `autoxide_kernel::money::Money`: `i64` đơn vị nhỏ nhất + mã ISO 4217 — 2026-11-13 — agent quyết

Quyết định: kernel có `Money { minor: i64, currency: Currency }`. `Currency` chỉ nhận mã ISO 4217
đang lưu hành có đơn vị nhỏ (bảng `money::CURRENCIES`, kèm số chữ số thập phân; không có kim loại
quý, mã thử, mã đã thôi dùng). `checked_add` / `checked_sub` / `checked_mul` / `checked_neg` /
`sum` trả lỗi `money.overflow` thay vì tràn; phép tính giữa hai loại tiền trả
`money.currency_mismatch`; không có `Ord` (so bằng `compare`, cũng từ chối hai loại tiền).
`allocate(&[tỉ lệ])` / `split(n)` dùng phương pháp phần dư lớn nhất (bằng nhau thì phần đứng
trước), tính bằng `i128`, tổng các phần luôn bằng số ban đầu; số âm chia đối xứng. JSON:
`{"amount": "<chuỗi số nguyên>", "currency": "USD"}`, `amount` không nhận số JSON, `+`, khoảng
trắng hay số thập phân. Lưu: hai cột `<tên>_minor` (`BIGINT`) và `<tên>_currency`;
`Money::from_columns` coi mã lạ trong database là lỗi nội bộ. Mã lỗi `money.*` nằm ở kernel,
dịch trong `crates/kernel/locales/vi.json`.

Lý do: luật dữ liệu đã cấm `f32`/`f64` và kiểu thập phân nhưng chưa có kiểu chung, nên mỗi module
sẽ tự viết phép cộng, cách chia và cách đưa ra JSON — và lệch nhau. `amount` là chuỗi vì
JavaScript đọc số JSON thành `f64`, sai với mọi số lớn hơn 2⁵³. Bảng mã viết tay (165 mã) thay
vì thêm thư viện: vài chục dòng đủ, đổi khi ISO công bố sửa đổi.

Đã cân nhắc: `rusty-money` / `iso_currency` — thêm thư viện cho một bảng; phân bổ kiểu Fowler
(dư trao cho phần đầu) — 1 chia [1, 99] trao cho phần 1% thay vì phần 99%; nhận cả số JSON cho
`amount` — hai cách cho một việc, và số lớn đã hỏng trước khi tới máy chủ.

## D-181 · Khoá dòng và cập nhật có điều kiện: `autoxide_kernel::rows` thay mẹo "chạm dòng" — 2026-11-13 — agent quyết

Quyết định: kernel có `rows::lock_one(conn, select)` / `rows::lock(conn, select, cột_thứ_tự)`
(đọc và khoá tới hết transaction: Postgres `SELECT … FOR UPDATE`, SQLite câu `SELECT` thường vì
transaction ghi đã là `BEGIN IMMEDIATE`, D-009) và `rows::adjust(conn, cột, điều_kiện, delta,
Bounds)` — một câu `UPDATE … SET c = c + Δ WHERE … AND floor − Δ <= c <= ceiling − Δ`, trả số
dòng đã đổi. Các câu `UPDATE` không đổi gì dùng để khoá dòng ở `auth` (user, session),
`forms`, `media` (nhãn, thư mục, ảnh thu nhỏ) và `content` (mục, `content_tree_locks`) đổi sang
`rows::lock_one`; cột `content_tree_locks.version` thôi được tăng (giữ cột, migration đã commit).
Không đổi: `RoleService::set_user_roles` (cập nhật `updated_at` thật vì role là một phần người
dùng) và câu tăng `reply_count` của `comments` (đã là cập nhật có điều kiện thật).

Lý do: sáu module tự viết cùng một mẹo, mỗi chỗ một kiểu, và mẹo đó ghi vào dòng (WAL, trigger,
`updated_at` dễ lỡ đổi theo). Trừ số dư cần một cách chuẩn: đọc rồi ghi để hở khe giữa hai câu.
Điều kiện chặn tràn đặt ở `WHERE` vì SQLite âm thầm đổi số nguyên tràn thành số thực còn
Postgres báo lỗi — chặn trước thì hai database như nhau. Test đồng thời: 40 request trừ cùng lúc
(đúng 33 thành công, số dư không âm), chuyển qua lại giữa hai ví, trên cả hai database.

Đã cân nhắc: giữ "chạm dòng" và chỉ gom thành một hàm — vẫn ghi vào dòng mà không cần; khoá
mức transaction (`SERIALIZABLE` trên Postgres) — mọi lệnh ghi phải sẵn sàng chạy lại, đắt với
đường nóng; advisory lock của Postgres — SQLite không có.

## D-182 · Chạy lại thao tác ghi khi xung đột tạm thời: `Database::write_with_retry` — 2026-11-13 — agent quyết

Quyết định: `Database::write_with_retry(move || async move { … })` chạy lại closure khi lỗi (tìm
dọc chuỗi nguyên nhân tới `DbErr`) là Postgres `40001` / `40P01` hay SQLite có byte thấp của mã
kết quả là 5 (`BUSY`) / 6 (`LOCKED`); tối đa `RETRY_ATTEMPTS` = 5 lần, chờ 10, 20, 40, 80 ms cộng
độ lệch ngẫu nhiên tới bằng chính nó; mỗi lần chạy lại tăng số đo
`database_write_retries_total`. Closure tự `begin_write` và `commit`; lỗi khác (kể cả lỗi nghiệp
vụ, vi phạm unique) trả ngay. `database::is_transient` công khai để module tự phân loại khi cần.
Module `audit` dùng nó khi nối chuỗi băm.

Lý do: khoá nhiều dòng sai thứ tự hay `SERIALIZABLE` làm Postgres huỷ một bên; SQLite báo bận
khi tiến trình khác giữ quyền ghi quá `AUTOXIDE_DATABASE_WAIT_SECONDS`. Cả hai lành khi chạy
lại. Closure không nhận transaction làm tham số: future mượn transaction phải mang thời gian sống
của nó (`for<'t>`), và Rust 1.99 không chứng minh được future như vậy là `Send` (lỗi
"implementation of `Send` is not general enough" trong event handler), còn kiểu
`BoxFuture<'t>` buộc mọi dữ liệu mượn từ ngoài phải sống mãi. Test: deadlock thật trên Postgres
(≥ 3 lần chạy, cả hai xong), database bận thật trên SQLite (hai `Database`, thời gian chờ 1 s),
lỗi khác chạy đúng một lần.

Đã cân nhắc: `async |transaction|` (`AsyncFnMut`) — lỗi `Send` nói trên; hàm nhận closure trả
`BoxFuture<'t>` như `TransactionTrait::transaction` của SeaORM — không mượn được biến bên ngoài;
chạy lại cả vi phạm unique — che lỗi thật.

## D-183 · Audit log là chuỗi băm; xoá dữ liệu cá nhân vẫn giữ được chuỗi — 2026-11-13 — agent quyết

Quyết định: `audit_entry` thêm `sequence` (unique, 1, 2, 3…), `payload_hash` (SHA-256 của
`before` / `after` / `details`) và `chain_hash` (SHA-256 của `chain_hash` dòng trước, mọi cột còn
lại và `payload_hash`; mỗi phần ghi kèm độ dài, thời gian ở micro giây, tiền tố `audit-v1`).
Bảng `audit_chain` một dòng giữ đầu chuỗi. Recorder khoá đầu chuỗi (`rows::lock_one`), bỏ qua
event đã ghi, thêm dòng và dời đầu chuỗi trong một transaction (`write_with_retry`). Xoá tài
khoản vẫn bỏ dữ liệu của các dòng về tài khoản đó nhưng giữ `payload_hash`; khi kiểm, dòng mất
dữ liệu chỉ hợp lệ nếu chuỗi có `USER_DELETED` cùng `subject`. `autoxide doctor` có bước
`audit.chain` (đọc cả bảng trong một snapshot, tối đa 10 phút — kernel thêm
`Check::with_timeout`) báo dòng đầu tiên bị sửa, bị xoá hay chèn ngoài chuỗi. Migration `m0004`
điền chuỗi cho dữ liệu cũ theo thứ tự ID. Cách băm `v1` là định dạng lưu trữ: không sửa, cần
đổi thì thêm `v2`.

Lý do: audit log "chỉ thêm" chỉ là quy ước của code; ai có quyền ghi database vẫn sửa được dòng
cũ mà không để lại dấu. Băm `payload_hash` thay vì băm thẳng dữ liệu là cách duy nhất để vừa xoá
được dữ liệu cá nhân (D-080, D-149) vừa giữ chuỗi kiểm được. Nối chuỗi tuần tự làm audit ghi nối
đuôi nhau; audit chạy qua event (không trên đường request) nên chấp nhận được.

Đã cân nhắc: ký từng dòng bằng khoá bí mật (HMAC) — khoá nằm cùng máy chủ thì kẻ sửa database
thường cũng lấy được, và mất khoá là mất khả năng kiểm; Merkle tree hay neo ra ngoài — để sau,
khi có nhu cầu chứng minh với bên thứ ba; mã hoá dữ liệu cá nhân rồi xoá khoá
(crypto-shredding) — cần quản lý khoá cho từng người dùng.

## D-184 · Chống trùng lệnh bằng mã tham chiếu của client: `autoxide_kernel::reference` — 2026-11-13 — agent quyết

Quyết định: quy ước cho lệnh có hậu quả lâu dài: client gửi `reference` (1–100 ký tự ASCII nhìn
thấy được, kiểm bằng `Reference::checked`); module lưu nó vĩnh viễn cùng bản ghi trong cột
`reference` và `reference_fingerprint` (`reference::fingerprint` = SHA-256 của JSON chuẩn hoá,
khoá sắp tăng dần, tự viết để không phụ thuộc feature `preserve_order`), unique index trên
(người sở hữu, `reference`). Service chạy lệnh bằng `reference::submit_once` và gọi
`reference::replayed` trên bản ghi tìm thấy: cùng nội dung → `Submitted::Replayed` (REST trả
200), khác → lỗi `reference_reused` (422, như `idempotency_key_reused`), chưa có → tạo
(`Submitted::Created`, 201). Hai request trùng đồng thời trên Postgres: unique index chặn bên
sau, `submit_once` chạy lại nó một lần để thấy bản ghi của bên kia.

Lý do: idempotency key HTTP (D-079) hết hạn sau 24 giờ và gắn với một request; lệnh thanh toán
gửi lại sau một ngày (agent retry, hàng đợi phía client) vẫn phải không tạo bản ghi thứ hai.
Kernel chỉ có helper; khái niệm "lệnh", "giao dịch" thuộc module. Test 16 request đồng thời cùng
mã chỉ tạo một bản ghi, trên cả hai database.

Đã cân nhắc: kéo dài thời hạn idempotency key — bảng đó lưu cả response, không hợp để giữ mãi;
trả 409 khi nội dung khác — 422 khớp mã đã có cho cùng tình huống ở idempotency key; một bảng
`reference` chung của kernel — module sẽ phải ghi hai bảng và join chéo, trái luật sở hữu bảng.

## D-185 · Nền cho kinh doanh đa mô hình: khách hàng, tổ chức, gói, tiền, tuân thủ — 2026-11-13 — chủ dự án quyết hướng, agent quyết cách làm

Quyết định: core phục vụ bốn mô hình chủ dự án đặt — khách độc lập; một khách nhiều cửa hàng
(chung hay khác database, báo cáo gộp); SaaS bán theo tính năng (gói, hạn mức, tính tiền, bật /
tắt module theo khách); chuỗi cha-con nhiều cấp (chung hay khác database) — cùng mô hình sàn, chỉ
bằng cấu hình cho từng khách. Thiết kế đầy đủ, nguồn tham khảo và căn cứ pháp lý:
`docs/research/business-foundation.md`. Năm trụ:

1. Khách hàng và vị trí dữ liệu: mặt điều khiển tách khỏi mặt ứng dụng (AWS SaaS); vị trí dữ liệu
   `shared` / `dedicated` chọn theo từng khách (bridge); mọi bảng cấp khách hàng có `tenant_id`
   kể cả trong database riêng (Salesforce); chế độ `single` mặc định cho người cài một cửa hàng.
2. Cây tổ chức trong khách hàng, quyền có độ sâu `own` / `unit` / `subtree` / `tenant` (Dynamics
   365); dữ liệu nghiệp vụ mang `org_id`; cài đặt kế thừa theo cây; tên miền gắn với nút; danh
   tính hai lớp — nhân sự toàn nền tảng, người mua theo từng khách (Shopify); cây khách hàng
   xuyên database với báo cáo gộp nhận qua event (Shopify Plus).
3. Gói là dữ liệu; module khai tính năng và hạn mức; kernel cưỡng chế bật / tắt module, tính
   năng, hạn mức; vòng đời thuê bao; cấp phát (Stripe Entitlements).
4. Tiền, sổ cái, thuế, chứng từ — D-187, D-188.
5. Tuân thủ pháp luật Việt Nam — D-186.

Lý do: `tenant_id` và ngữ cảnh khách hàng chạm mọi bảng và mọi request; làm bây giờ (13 module, chưa
có dữ liệu thật) rẻ hơn nhiều so với sau khi có module thương mại và khách thật. Chọn trộn
`shared` / `dedicated` thay vì một kiểu vì chủ dự án cần cả hai (một database chung chia cửa hàng,
và mỗi khách SaaS một database); `tenant_id` có mặt cả trong database riêng để code một kiểu và dời
khách giữa hai kiểu bằng chép dữ liệu.

Đã cân nhắc: mỗi khách một bản cài (D-006 cũ) — không làm được báo cáo gộp, quản trị chung, gói và
tính tiền tập trung; mỗi khách một schema — chỉ Postgres có, trái Luật số một; chỉ dùng chung một
database (pool thuần) — không đáp ứng khách cần database riêng; Postgres row-level security —
SQLite không có.

Thay: phần "Không có multi-tenant" của D-006. Việc làm: bước 33–38 của `docs/ROADMAP.md`.

## D-186 · Tuân thủ pháp luật Việt Nam là luật thường trực của dự án — 2026-11-13 — chủ dự án quyết

Quyết định: mọi thiết kế và mọi tính năng tuân thủ pháp luật Việt Nam hiện hành — thương mại điện
tử (xác thực người bán trước khi bán; website bán hàng thông báo — logo xanh, nền tảng trung gian /
sàn đăng ký — logo đỏ với Bộ Công Thương), bảo vệ dữ liệu cá nhân, an ninh mạng, xác thực tài khoản
đăng nội dung, thuế (gồm khấu trừ thay của sàn), thanh toán, sở hữu trí tuệ. Chủ dự án nhắc rõ:
AI quyết định nhưng phải theo luật nhà nước.

Cách làm: (1) mỗi tính năng tuân thủ ghi căn cứ pháp lý (văn bản, điều, ngày hiệu lực) trong mục
quyết định của nó; (2) quy định hay đổi — biểu thuế, danh mục hồ sơ, thời hạn — để dạng dữ liệu
có ngày hiệu lực, cập nhật không cần sửa code; (3) AI không phải luật sư: chỗ các nguồn chưa
thống nhất (vd nghị định hướng dẫn Luật TMĐT 2025) ghi "cần đối chiếu văn bản gốc / người có
chuyên môn pháp lý" và không chặn công việc; trước khi chạy thật cho khách, chủ dự án cho người có
chuyên môn xác nhận; (4) không làm tính năng giúp né luật.

Lý do: sản phẩm phục vụ bán hàng và sàn tại Việt Nam; vi phạm có thể bị phạt theo doanh thu (Luật
BVDLCN 2025) hay buộc dừng hoạt động.

## D-187 · Dòng tiền: core không tự giữ tiền; ba chế độ qua đối tác có giấy phép — 2026-11-13 — agent quyết

Quyết định: core không bao giờ nhận và giữ tiền của người mua thay người bán. Tiền chỉ đi qua đối
tác có giấy phép (ngân hàng, tổ chức trung gian thanh toán) bằng trait `PaymentGateway` /
`PayoutGateway`; core ghi nghĩa vụ bằng sổ cái kép (module `ledger`: chỉ thêm, sửa bằng bút toán
đảo) và đối soát với sao kê của đối tác. Ba chế độ chọn theo từng khách / sàn: **tự thu** (tiền vào
thẳng tài khoản người bán), **thu hộ qua đối tác** (đối tác có giấy phép thu, sổ cái ghi phải trả
người bán, phí, thuế khấu trừ, giữ tới hết hạn đổi trả rồi chi hộ), **chia tại cổng**. Nhận
webhook của đối tác: kiểm chữ ký, xử lý không trùng.

Lý do: nhận tiền người mua rồi chi lại cho người bán là dịch vụ hỗ trợ thu hộ, chi hộ — loại trung
gian thanh toán cần giấy phép của Ngân hàng Nhà nước (NĐ 52/2024/NĐ-CP; cần đối chiếu Điều 3, 22 và
Thông tư 40/2024/TT-NHNN). Sàn có chức năng thanh toán phải khấu trừ thuế thay người bán (NĐ
117/2025), nên chế độ dòng tiền quyết định nghĩa vụ thuế — ghi rõ trong cấu hình của khách.

Đã cân nhắc: ví nội bộ giữ tiền người dùng — cần giấy phép ví điện tử / trung gian thanh toán; chỉ
hỗ trợ tự thu — không đáp ứng sàn kiểu Shopee, Lazada.

Thay: mục "module sổ cái kép chờ chủ dự án quyết" ở bước 29 — chủ dự án giao AI quyết (D-185).

## D-188 · Thuế và chứng từ: engine thuế trong core, biểu thuế là dữ liệu có ngày hiệu lực, `rust_decimal` — 2026-11-13 — agent quyết

Quyết định: core có engine thuế trung lập: nhóm hàng hoá, biểu thuế theo vùng thuế với ngày hiệu
lực từ / tới, giá gồm / chưa gồm thuế, làm tròn, hồ sơ thuế của người bán (doanh nghiệp khấu trừ,
hộ kinh doanh, miễn), thuế khấu trừ thay. Mỗi dòng chứng từ lưu ảnh chụp thuế suất lúc giao dịch.
Gói dữ liệu "vn" (GTGT 0% / 5% / 10%, mức giảm tạm thời, thuế tiêu thụ đặc biệt, tỉ lệ khấu trừ
của hộ kinh doanh trên sàn) cập nhật bằng CLI / API; `autoxide doctor` cảnh báo thuế suất sắp hết
hạn. Tính toán dùng `rust_decimal`, lưu bằng `Money` (D-180). Chứng từ đánh số tuần tự không nhảy
số theo phạm vi; hoá đơn điện tử qua trait `EInvoiceProvider`.

Lý do: thuế theo mặt hàng và thay đổi theo luật — mức GTGT 8% (giảm 2%) chỉ tới hết 31/12/2026 —
nên viết cứng trong code là sai ngay khi luật đổi; chứng từ cũ phải giữ đúng thuế suất lúc lập.
Tính thuế, chiết khấu, tỉ giá cần số thập phân có quy tắc làm tròn rõ, số nguyên đơn vị nhỏ nhất
không đủ cho phép nhân chia.

Đã cân nhắc: tự viết số thập phân — trái "không tự chế"; gọi dịch vụ thuế bên ngoài — không có
dịch vụ phổ biến cho thuế Việt Nam, và phải chạy được khi không có mạng.

Thay: mục "thư viện số thập phân chờ chủ dự án quyết" ở bước 29 — chủ dự án giao AI quyết (D-185).

## D-189 · Sắp lại lộ trình: nền đa khách đứng trước vận hành và năng lực còn thiếu — 2026-11-13 — agent quyết

Quyết định: bước 33 (vận hành và hiệu năng) thành 39; bước 34 (năng lực còn thiếu) thành 40; bước 35
(bảo vệ dữ liệu và tài khoản) tách: đăng nhập bằng link và mời người dùng vào 34, mã hoá dữ liệu
nhạy cảm vào 37. Bước 33–38 mới là nền đa khách (D-185…D-188). Bước 30–32 giữ nguyên.

Lý do: backup, số đo, OIDC, SSE, mã hoá đều phải biết ngữ cảnh khách hàng; dựng chúng trước nền đa
khách thì phải sửa lại. Chưa ai bắt đầu bước 33–35 cũ (kiểm nhánh trên GitHub ngày 13/11/2026).

## D-190 · Bổ sung nền đa khách sau rà soát: thứ ngoài bảng, secret theo khách, luật kế toán, giao dịch điện tử, người tiêu dùng, AI — 2026-11-13 — agent quyết

Quyết định: thêm vào bước 33–39 các mục ở mục 7 của `docs/research/business-foundation.md`:
(1) mọi thứ ngoài bảng cũng theo khách — khoá file, cache trong bộ nhớ, rate limiting, idempotency
key, con trỏ pagination, token và URL đã ký; (2) cấu hình và secret theo khách, mã hoá khi lưu — mục
mã hoá secret dời từ bước 37 lên 33 (thay phần đó của D-189); (3) tên miền riêng có xác minh DNS và
endpoint cho TLS tự động; (4) phiên bản migration theo từng database, lỗi `tenant_maintenance`
chỉ cho khách trên database chưa sẵn sàng, chia lượt job công bằng; (5) nhân sự nền tảng vào dữ
liệu khách có thời hạn, lý do, audit khách xem được; (6) chứng từ kế toán giữ ít nhất 10 năm, khoá sổ
theo kỳ; (7) hoá đơn từ máy tính tiền; (8) văn bản pháp lý có phiên bản và bằng chứng chấp nhận;
(9) bảo vệ người tiêu dùng; (10) gắn nhãn nội dung AI (`ai_assisted`); (11) trait `SmsSender`;
(12) log và trace mang `tenant_id`, không dùng làm nhãn số đo.

Lý do: mỗi mục là chỗ mà nếu không đặt từ đầu thì dựng xong nền đa khách vẫn rò rỉ dữ liệu (cache,
URL ký), không phục vụ được khách mang tài khoản đối tác riêng, hay vi phạm luật. Căn cứ pháp lý:
Luật Kế toán 88/2015 Điều 41 và NĐ 174/2016 (lưu 10 năm); NĐ 123/2020 sửa bởi NĐ 70/2025 (hoá đơn từ
máy tính tiền, từ 01/6/2025); Luật Giao dịch điện tử 20/2023; Luật Bảo vệ quyền lợi người tiêu dùng
19/2023 và NĐ 55/2024 (từ 01/7/2024); Luật Trí tuệ nhân tạo 134/2025 (từ 01/3/2026); NĐ 147/2024.
Nguồn là báo chí và công ty luật tổng hợp — cần đối chiếu văn bản gốc (D-186), nhất là nghị định
hướng dẫn Luật Trí tuệ nhân tạo và ngưỡng doanh thu hoá đơn từ máy tính tiền.

Đã cân nhắc: mỗi khách một khoá mã hoá để xoá được cả trong backup — phức tạp, chưa có luật buộc;
khuyến mại, phí sàn theo ngành hàng, người bán nước ngoài — để cho module thương mại.

Thay: phần "mã hoá dữ liệu nhạy cảm vào 37" của D-189.

## D-191 · Module thương mại: sáu module, bước 41–46 — 2026-11-13 — chủ dự án giao, agent quyết cách làm

Quyết định: chủ dự án giao dựng module thương mại. Sáu module, mỗi module một crate: `catalog` (41),
`inventory` (42), `promotions` (43), `shipping` (44), `orders` gồm giỏ hàng và đặt hàng (45),
`marketplace` (46). Thiết kế ở mục 8 của `docs/research/business-foundation.md`. Nhất quán giữa
module bằng giữ chỗ có hạn theo mã tham chiếu → xác nhận → trả lại, không có transaction xuyên
module. `shipping` không phụ thuộc `orders` (nhận và báo qua event). Bộ nối đối tác cụ thể là module
riêng sau trait của core, làm khi chủ dự án cấp tài khoản thử. Hạn mức khuyến mại, danh mục hàng có
điều kiện, đơn vị hành chính là dữ liệu có ngày hiệu lực.

Lý do: tách theo nghiệp vụ để khách chỉ bật module cần (bước 35) — website bán hàng không cần
`marketplace`, cửa hàng chỉ bán tại quầy không cần `shipping`. Giữ chỗ thay transaction chung vì luật
kiến trúc 1 (module sở hữu bảng của mình) và vì chạy đúng cả khi module nằm ở database khác. Luật
khuyến mại vừa đổi (Thông tư 39/2025, NĐ 239/2026) và địa chỉ hành chính vừa đổi (hai cấp từ
01/7/2025) cho thấy những thứ này phải là dữ liệu.

Đã cân nhắc: một module `commerce` lớn — không bật / tắt riêng được, file và bảng quá nhiều cho một
crate; giỏ hàng là module riêng — gắn chặt với đặt hàng, tách ra chỉ thêm hợp đồng; viết sẵn bộ nối
VNPay, GHN… trong core — cần secret thật, và core phải trung lập với đối tác.

Thay: mục "Module thương mại" trong phần "Sau core" của lộ trình (D-185).

## D-192 · Rà bước 30–40: thứ tự, phụ thuộc, đường nâng cấp — 2026-11-13 — agent quyết

Quyết định: lộ trình có bảng thứ tự và phụ thuộc ngay trước bước 30. Bước 33 bắt đầu sau bước 31 (gate
chặn thay đổi phá vỡ và test nâng cấp giữ an toàn khi mọi bảng đổi) và nhóm A của bước 32; ở chế độ
`single`, `/api/v1` không đổi và migration gán dữ liệu có sẵn cho khách mặc định; bước 34 chuyển tài
khoản có sẵn thành nhân sự của khách mặc định. Bước 35: mục tính tiền chờ bước 36, mục cấp phát chờ
bước 37. Bước 40: header bảo mật và MCP làm được ngay; bốn mục còn lại sau bước 34. Bước 32 làm được
ngay (bước 29 đã merge); dời file `media` sau khi WebP của bước 30 merge.

Lý do: chủ dự án hỏi lộ trình đã đúng thứ tự để phát triển chưa (13/11/2026, khi teammate mới làm
bước 30). Rà thấy: trạng thái ghi sai ("đang làm 30–32"); bước 33 thiếu đường nâng cấp cho bản cài
cũ; hai mục của 35 phụ thuộc 36 / 37 mà không ghi; bước 40 bị chặn cả bước dù hai mục không cần nền
đa khách — teammate thứ ba có việc làm ngay.

## D-193 · Đơn vị hành chính Việt Nam là phần nền: module `address`, chỉ danh mục sau sáp nhập — 2026-11-13 — chủ dự án quyết

Quyết định: nền có module `address` (bước 34) giữ danh mục đơn vị hành chính Việt Nam **sau sáp
nhập** — hai cấp tỉnh / thành → xã / phường / đặc khu, 34 tỉnh và 3.321 xã theo QĐ 19/2025/QĐ-TTg,
hiệu lực 01/7/2025. Chủ dự án yêu cầu chỉ lấy danh mục sau gộp: không có cấp huyện, không bảng đối
chiếu địa chỉ cũ → mới. Dữ liệu dùng chung mọi khách, lấy từ nguồn chính thức (ghi nguồn và ngày),
có ngày hiệu lực, cập nhật bằng task. `api` công bố kiểu `Address` và trait tra cứu / kiểm; mọi module
cần địa chỉ (tổ chức, KYC, hồ sơ pháp lý, người mua, kho, vận chuyển) dùng nó, không tự làm bảng địa chỉ.

Lý do: nhiều phần của nền cần địa chỉ chuẩn trước khi có module thương mại (KYC và hồ sơ đăng ký Bộ
Công Thương ở bước 37, cửa hàng và kho là nút tổ chức ở bước 34); để trong `shipping` (bước 44) thì
các phần đó phải chờ hoặc tự làm lại. Danh mục nhà nước là văn bản hành chính, không thuộc đối tượng
bảo hộ quyền tác giả (Luật Sở hữu trí tuệ, Điều 15 — cần đối chiếu), dùng được trong repo công khai.

Đã cân nhắc: đặt trong `kernel` — dữ liệu riêng của một nước, core phải dùng được cho mọi nước; gọi
API bên ngoài để tra — phải chạy khi không có mạng và không phụ thuộc dịch vụ của người khác.

Thay: mục địa chỉ của bước 44 trong D-191 (bỏ bảng đối chiếu địa chỉ cũ).

## D-194 · Đối chiếu với nền tảng thương mại điện tử đang chạy: thêm thông báo, VietQR, khách hàng, đa kênh, tiếp thị liên kết — 2026-11-13 — agent quyết

Quyết định: thêm module nền `notifications` (bước 37; thay mục "module thông báo chờ chủ dự án quyết",
chủ dự án đã giao AI quyết ở D-185); thêm vào bước 36 chuyển khoản VietQR tự khớp (trait `BankFeed`),
tỉ giá có ngày hiệu lực, mẫu in chứng từ, xuất dữ liệu kế toán; vào 41 hàng số / dịch vụ / combo; vào
42 nhập hàng từ nhà cung cấp; vào 45 chống bom hàng COD, trả góp, bảo hành; vào 46 báo cáo cho cơ quan
thuế và kho của sàn. Thêm ba module, ba bước: `customers` (47), `channels` (48), `affiliate` (49).
Thẻ quà tặng và số dư cửa hàng tắt mặc định, chỉ dùng mua hàng của chính khách, không rút ra tiền,
tới khi đối chiếu NĐ 52/2024 về ví điện tử và thẻ trả trước. Để sau: B2B, đặt lịch, chat, thuê bao định
kỳ, bán xuyên biên giới, quảng cáo trong sàn — ghi ở "Sau core".

Lý do: chủ dự án hỏi còn thiếu gì so với mô hình thương mại điện tử. Mỗi mục là thứ người bán ở Việt
Nam đang dùng hằng ngày, hay là nghĩa vụ luật: tin quảng cáo phải có đồng ý trước (NĐ 91/2020);
người tiếp thị liên kết bị khấu trừ thuế khi nhận hoa hồng (ngưỡng và tỉ lệ đang thay đổi theo NĐ
253/2026 — để dạng dữ liệu, cần đối chiếu). Thông báo là nền vì đơn hàng, KYC, thuê bao, bảo mật đều
cần báo người dùng.

Đã cân nhắc: gộp điểm thưởng vào `promotions` — điểm là nghĩa vụ kéo dài, khác chương trình giảm giá
có hạn; gộp đa kênh vào `orders` — bộ nối từng sàn là phần lớn nhất, cần bật / tắt riêng theo gói.

## D-195 · Khoảng rút trước khi tắt: `AUTOXIDE_HTTP_DRAIN_SECONDS`, mặc định 0 — 2026-11-13 — agent quyết

Quyết định: nhận tín hiệu dừng thì `serve` bật cờ `http::Draining` (`/health/ready` trả 503
`shutting_down`, không hỏi database; `/health/live` vẫn 200), vẫn phục vụ đủ
`AUTOXIDE_HTTP_DRAIN_SECONDS` (0–600, mặc định `0`), rồi mới tới graceful shutdown cũ
(`AUTOXIDE_HTTP_SHUTDOWN_SECONDS` đếm từ lúc đó). Việc ghép nằm ở `commands/serve.rs` (bọc future
tín hiệu dừng), `http::serve` không đổi. `health_routes(database, draining)` nhận thêm cờ — thay
đổi phá vỡ của kernel, ghi trong CHANGELOG (bản `0.x`). `compose.yml` đặt `stop_grace_period: 40s`
vì Docker mặc định giết tiến trình sau 10 giây, ngắn hơn 30 giây app chờ request đang chạy.

Lý do: bộ cân bằng tải chỉ biết gỡ một máy chủ sau vài lần kiểm `/health/ready` hỏng; nếu máy chủ
ngừng nhận kết nối ngay khi nhận `SIGTERM`, request gửi tới trong khoảng đó bị từ chối. Mặc định
`0` vì một máy chủ không có bộ cân bằng tải (cách cài phổ biến nhất của người lạ) không có nơi nào
khác để chuyển request — rút chỉ làm Ctrl-C chậm đi; ai chạy nhiều máy chủ đặt theo
`docs/DEPLOY.md`.

Đã cân nhắc: mặc định khác 0 (vd 5 giây) — người chạy một máy chủ chờ vô ích mỗi lần dừng, và
vẫn không đúng cho bộ cân bằng tải nào cụ thể; giữ `health_routes(database)` và thêm hàm thứ hai
nhận cờ — hai cách cho một việc; `preStop` sleep của Kubernetes — chỉ có trên Kubernetes, còn
`/health/ready` vẫn báo 200 trong lúc tắt.

## D-196 · Bản WebP của ảnh thu nhỏ: thư viện `webp` (libwebp), địa chỉ `&format=webp`, trường `webp_url` — 2026-11-13 — agent quyết

Quyết định (thực hiện D-176):

- Thư viện: `webp` 0.3 (bọc `libwebp` C qua `libwebp-sys`, biên dịch kèm), không bật feature
  `img`. Mã hoá nén mất dữ liệu, chất lượng 80 (`WEBP_QUALITY`), ảnh có kênh trong suốt giữ kênh
  đó (`alpha_compression`). Gọi qua `Encoder::from_rgb` / `from_rgba(…).encode_simple(false, q)`
  — trả `Result`, không dùng `encode()` vì nó `unwrap` bên trong.
- Mỗi cỡ hai bản: JPEG / PNG như cũ (cho client không đọc WebP) và WebP. Bảng `media_variant`
  thêm dòng có `content_type = 'image/webp'`; migration `m0007` đổi unique index thành (item,
  phiên bản, chiều rộng, `content_type`). Khoá nơi lưu của bản WebP là khoá cũ thêm `.webp`;
  job dọn mồ côi hiểu đuôi này. Hai bản lưu cùng transaction.
- Website nhận WebP bằng địa chỉ riêng: `…/content?v=…&w=<rộng>&format=webp`, ETag riêng
  (`-webp`), cache một năm như bản JPEG. Mỗi mục `variants` (REST của `media`, `MediaVariant` của
  `api`, `PublicVariant` của `content` cho `featured_media` / `body_media`) thêm `webp_url` (và
  `webp_size` ở REST của `media`), `null` khi chưa làm. Xin `format=webp` khi bản WebP chưa có
  thì nhận bản JPEG / PNG cùng cỡ, cache ngắn — địa chỉ trong `<source type="image/webp">` luôn
  ra ảnh. `format` khác `webp` là lỗi `validation_failed` (422).
- Ảnh đã có: `autoxide task media.refresh_variants` xếp làm lại cả ảnh chỉ thiếu bản WebP.

Số đo (`render` trên ảnh chụp thật 4000 × 2250 — ảnh mẫu `lake.jpg` đi kèm crate `webp` — và
chính ảnh đó phủ mặt nạ trong suốt hình elip để giống ảnh cắt nền; build release, máy Linux
4 CPU, một thread):

| Ảnh, rộng      | JPEG / PNG        | WebP            | Nhẹ hơn | Mã hoá JPEG / PNG → WebP |
| -------------- | ----------------: | --------------: | ------: | ------------------------ |
| ảnh chụp, 320  | JPEG 8,6 KB       | 4,0 KB          | 54%     | 1,4 ms → 4,7 ms          |
| ảnh chụp, 640  | JPEG 26,9 KB      | 11,5 KB         | 57%     | 5,2 ms → 16 ms           |
| ảnh chụp, 1280 | JPEG 94,7 KB      | 38,8 KB         | 59%     | 20 ms → 64 ms            |
| ảnh chụp, 1920 | JPEG 208 KB       | 85,9 KB         | 59%     | 45 ms → 143 ms           |
| cắt nền, 320   | PNG 88 KB         | 5,2 KB          | 94%     | 1,3 ms → 11 ms           |
| cắt nền, 1280  | PNG 1,37 MB       | 38 KB           | 97%     | 17 ms → 168 ms           |
| cắt nền, 1920  | PNG 3,31 MB       | 80 KB           | 98%     | 43 ms → 399 ms           |

Với bốn cỡ mặc định, bản WebP thêm khoảng 0,23 s (ảnh chụp) đến 0,6 s (ảnh cắt nền) cho job
nền của mỗi ảnh; request không chậm đi.

Lý do: `image-webp` (bộ mã hoá của `image`) chỉ có kiểu không mất dữ liệu — ảnh chụp không nhẹ
đi; bộ mã hoá thuần Rust có kiểu mất dữ liệu (`zenwebp`) mang license AGPL, không hợp với repo
MIT / Apache. Địa chỉ riêng thay vì đọc header `Accept`: CDN nào cũng cache đúng (không cần
`Vary: Accept`), website chọn bằng `<picture>`, và `url` cũ không đổi hành vi. Trường mới thay
vì thêm mục WebP vào `variants`: client cũ dựng `srcset` từ mọi mục sẽ thấy hai mục cùng chiều
rộng.

Đã cân nhắc: theo `Accept` trên cùng địa chỉ — nhiều CDN bỏ qua `Vary` hay cache theo cả chuỗi
`Accept`, ảnh sai loại đến trình duyệt cũ; chỉ làm WebP, bỏ JPEG — email và vài client cũ không
đọc WebP; WebP không mất dữ liệu cho ảnh trong suốt — nặng gấp nhiều lần bản có mất dữ liệu;
AVIF — nhẹ hơn nữa (D-168) nhưng mã hoá chậm hơn nhiều và cần thêm thư viện C lớn, để sau.

## D-197 · Workflow GitHub tự chạy chỉ khi repo công khai; image hai kiến trúc, chứng thực và SBOM; quy tắc ứng xử — 2026-11-13 — agent quyết

Quyết định (thực hiện D-175):

- `ci.yml` (push lên `main`, pull request), `audit.yml` (lịch hằng ngày) và `release.yml` (tag
  `vX.Y.Z`) có lại trigger tự chạy. Mọi job mang `if: ${{ !github.event.repository.private }}`:
  repo riêng tư thì job bị bỏ qua trước khi xin runner, không tốn phút nào; chủ dự án công khai
  repo là chúng tự chạy, không cần sửa file. Payload của sự kiện `schedule` có thể không mang
  `repository`: khi đó `audit.yml` vẫn chạy (khoảng 1–2 phút `cargo deny` mỗi ngày) cho tới lúc
  công khai — chấp nhận được, đổi lại không có nguy cơ job lặng lẽ không bao giờ chạy.
- `release.yml`: gate → build image `linux/amd64` trên `ubuntu-24.04` và `linux/arm64` trên
  `ubuntu-24.04-arm` (máy gốc, không giả lập QEMU; miễn phí cho repo công khai), đẩy theo digest
  → job `publish` ghép thành một image đa nền tảng với tag `X.Y.Z` và `X.Y`, rồi `actions/attest`
  ký chứng thực nguồn gốc build (SLSA) và SBOM (SPDX do `anchore/sbom-action` tạo), đẩy cả hai
  lên registry; SBOM đính vào bản phát hành GitHub. BuildKit cũng gắn SBOM và provenance của nó
  vào từng image. Mọi việc bằng `GITHUB_TOKEN`, không token cá nhân. `docs/RELEASING.md`: agent
  chỉ đẩy tag; đổi gói image sang Public lần đầu là việc của chủ repo (GitHub không cho đổi
  bằng `GITHUB_TOKEN`). `docs/DEPLOY.md` có lệnh `gh attestation verify`.
- `CODE_OF_CONDUCT.md`: nguyên văn Contributor Covenant 3.0 (bản mới nhất), chỉ điền cách báo
  cáo (email do chủ dự án chọn) và bỏ ghi chú mẫu ở phần xử lý. Bản 3.0 chưa có bản tiếng Việt
  chính thức: `CODE_OF_CONDUCT.vi.md` là bản dịch của dự án, ghi rõ bản tiếng Anh có giá trị,
  cùng license CC BY-SA 4.0 như bản gốc (bản phỏng theo phải giữ license đó).
- Lịch sử commit mang email cá nhân của chủ dự án: chủ dự án chọn giữ nguyên, không viết lại
  lịch sử.

Lý do: D-107 tắt CI vì runner của repo riêng tư tính phí; điều kiện theo `private` giữ đúng lý
do đó mà không bắt ai nhớ bật lại. Máy build ARM gốc nhanh ngang amd64, còn giả lập là lý do
D-078 để ARM64 lại. Chứng thực ký qua Sigstore cho người cài kiểm image đúng do repo này build;
attestation của GitHub chỉ có cho repo công khai trên gói Free / Pro / Team, nên cũng chờ công
khai như phần còn lại.

Đã cân nhắc: bật trigger không điều kiện — tốn phí hoặc chạy đỏ khi repo còn riêng tư; một job
đầu tiên gọi API để hỏi repo có công khai không — chính job đó cũng tốn phút; build đa nền tảng
bằng QEMU trên một máy — chậm gấp nhiều lần (D-078); `cosign` tự cài thay cho `actions/attest`
— thêm một công cụ, còn `gh attestation verify` người dùng đã có cùng GitHub CLI.

## D-198 · Đặc tả từng bước trong `docs/steps/`, luật chung giữa các bước; giữ nguyên lịch sử git — 2026-11-13 — chủ dự án quyết hướng, agent quyết nội dung

Quyết định: mỗi bước 30–49 có một file đặc tả trong `docs/steps/` (bảng dữ liệu, API, permission, event,
mã lỗi, thứ tự commit, test bắt buộc, tiêu chí xong, chỗ dễ sai, căn cứ pháp lý); `docs/steps/README.md`
giữ luật chung giữa các bước và thắng khi một đặc tả lệch. Luật chung chốt:

1. Hàm chạm database nhận `&TenantContext` (nguồn duy nhất của `Database`); hàm không chạm database nhận
   `TenantId`. Khách mặc định có ID cố định — ngoại lệ có chủ đích của luật UUIDv7; `NO_TENANT` cho job
   của cả database; header `Autoxide-Tenant`.
2. Cây tổ chức (`kernel_org_units`, `kernel_org_closure`) ở kernel, vì lọc theo nhánh cần subquery
   trong câu SQL của mọi module (luật kiến trúc 1); nút gốc của khách có ID bằng tenant ID.
3. Thanh toán nằm trong `ledger`, không có module `payments`; chuỗi băm sổ cái không băm `tenant_id`;
   `kernel_sequences` là bảng cấp khách hàng ở mọi database dữ liệu.
4. Thứ tự đăng ký: `address` → `organizations` → `auth`…; `tax` → `ledger` → `invoices` → `compliance`
   → `notifications` → `newsletter`, `comments` → `plans` (sau mọi module nền, module trước hỏi qua
   `EntitlementsHandle`) → thương mại → `reporting`.
5. Module đăng ký trước cần gọi module sau thì công bố "điểm cắm" trong `api` của mình; nhận event của
   module có thể chưa cài bằng `Subscription::optional` (dựng ở commit chuẩn bị của bước 41, cùng
   `autoxide_kernel::csv` và `::slug`). Tên event do module phát quyết.
6. API mới của kernel có bảng "bước nào dựng" trong README; bước cần trước thì dựng, bước sau dùng lại.
7. **Chủ dự án quyết: giữ nguyên lịch sử git khi công khai repo** (email tác giả commit, dòng
   `Claude-Session:`) — không viết lại, agent bước 30 không hỏi lại.

Lý do: chủ dự án yêu cầu (13/11/2026) lộ trình đủ chi tiết để teammate làm đầy đủ và đúng, không dùng
tài liệu ngoài repo. Đặc tả do tám agent viết song song theo một bảng tên chung; agent chính đối chiếu
chéo, sửa chỗ lệch tên event giữa bước 44 và 45, và chốt các điểm các agent đề xuất khác nhau ở trên.
Ghi chú: D-177 viết "agent làm bước 32 quyết" theo số cũ — theo bảng đánh số lại của D-178 đó là bước 31
(SDK TypeScript).

Đã cân nhắc: viết chi tiết thẳng vào `ROADMAP.md` — file đã hơn 900 dòng, agent đọc một bước phải nạp cả
lộ trình; truyền khách ngầm bằng `task_local` — mất khi `tokio::spawn`, quên khách không bị trình biên
dịch bắt; cây tổ chức trong module `organizations` — module khác không được viết subquery vào bảng của nó.

## D-199 · Repo tạm giữ riêng tư; phần cần repo công khai tách thành bước 50 — 2026-11-13 — chủ dự án quyết

Quyết định: chủ dự án giữ repo `autoxide` **riêng tư** cho tới khi tự quyết công khai. Thay phần
"chuyển sang công khai" của D-175 trong thời gian đó; mục tiêu lâu dài "mã nguồn mở, công khai
toàn bộ" trong `CLAUDE.md` không đổi, và file trong repo vẫn viết với giả định người lạ sẽ đọc.

Hệ quả:

- Các mục của bước 30 chỉ làm được khi repo công khai (bấm công khai, CI chạy xanh trên GitHub,
  workflow `Release` đăng image amd64 + arm64 kèm chứng thực và SBOM, bật secret scanning / push
  protection / private vulnerability reporting, thử cài bằng image đã đăng) chuyển sang **bước 50
  "Công khai repo"** ở cuối lộ trình. Agent không làm bước 50 khi chủ dự án chưa nói đã công khai.
- Workflow giữ nguyên như D-197: tự chạy khi repo công khai, còn riêng tư thì bỏ qua, không tốn
  phí. Không cần sửa gì khi công khai.
- Trong lúc riêng tư, phát hành từ máy (`docs/RELEASING.md`, phần "Khi repo còn riêng tư"): agent
  đẩy commit release và tag, tạo bản phát hành GitHub bằng tay; image Docker chỉ đăng khi chủ dự
  án đưa token có quyền `write:packages` (amd64, không chứng thực).
- Bước 30 xong với phần trong repo, và mục "thử cài như người lạ" làm theo đường build từ mã nguồn
  trong README / `docs/DEPLOY.md`.

Lý do: chủ dự án chưa muốn công khai. Tách phần phụ thuộc ra một bước riêng để lộ trình không kẹt
ở bước 30 (agent chọn bước có số nhỏ nhất còn mục chưa xong), và các tính năng đã làm (WebP, khoảng
rút khi tắt) được phát hành.

Thay: phần công khai của D-175 (tới khi chủ dự án quyết công khai).

## D-200 · Thiết kế giao diện 2: quản trị đa phân hệ, mặt tiền bán hàng; bước 51 cho phần core còn thiếu — 2026-11-13 — chủ dự án giao hướng, agent quyết nội dung

Quyết định: giao diện (repo riêng, D-097) theo thiết kế 2 (`docs/research/ui-design.md`):

1. Trang quản trị có mười hai phân hệ trên một thanh dọc; quản trị nội dung chỉ là một phân hệ. Menu
   con là hàng tab dưới thanh tiêu đề. Ngữ cảnh "khách › nút tổ chức" ở góc trên trái quyết phạm vi mọi
   số liệu. Ctrl + K tìm xuyên phân hệ. Mỗi phân hệ có trang tổng quan; trang Tổng quan chung có hộp
   việc gom từ mọi phân hệ.
2. Một mục menu hiện khi module đã bật cho khách và người dùng có permission ở nút đang chọn; thiếu
   tính năng của gói thì **vẫn hiện, kèm nhãn khoá**.
3. Website công khai là mặt tiền bán hàng của một khách (trang chủ, danh mục, sản phẩm, giỏ và đặt
   hàng, gian hàng người bán, bài viết có gắn sản phẩm, tài khoản người mua); blog thành một mục.
4. Phần core còn thiếu thành **bước 51** (sổ điều hướng, hộp việc, tìm xuyên phân hệ, số liệu tóm
   tắt, hoạt động gần đây, nhúng bản ghi trong nội dung) và bốn mục thêm vào bước 34 (hồ sơ công khai
   của nút), 42 (tình trạng hàng công khai theo cửa hàng), 43 (giá khi áp mã cho từng sản phẩm), 44
   (ước tính phí giao công khai). Đặc tả: `docs/steps/51-ui-contracts.md` và mục "Bổ sung theo D-200"
   ở cuối đặc tả 34, 42, 43, 44.
5. Kernel không có danh sách phân hệ viết cứng: nhóm menu do module khai (core trung lập với nghiệp vụ);
   việc trong hộp việc tính từ dữ liệu của module, không có bảng việc riêng.

Lý do: chủ dự án thấy thiết kế 1 coi trang quản trị là CMS, trong khi lộ trình đã mở sang nền đa khách
và thương mại (13/11/2026). Menu, khoá theo gói và hộp việc phụ thuộc dữ liệu chỉ core có (module bật,
permission theo nút, quyền dùng); viết cứng ở giao diện thì mỗi mô hình kinh doanh cần một giao diện.
Đối chiếu với đặc tả 33–47 cho thấy phần lớn API của mặt tiền đã có (giỏ hàng, tách đơn theo người bán,
đồng ý, chấp nhận văn bản, trang người bán công khai); chỉ thêm những mục ở trên.

Đã cân nhắc: gom menu, hộp việc vào module `notifications` — hộp thư thông báo là tin đã gửi cho một
người, còn hộp việc là trạng thái hiện tại của dữ liệu, xử lý xong thì tự hết; một endpoint `/public/site`
gom cấu hình website từ nhiều module — phải trả JSON không kiểu, mất mô tả OpenAPI; để giao diện gọi ba
endpoint có cache (cửa hàng, menu, logo Bộ Công Thương) là đủ. Đánh số lại để bước 51 đứng trước 41 —
đổi số của 11 bước và 11 file đặc tả; thay bằng ghi trong bảng phụ thuộc rằng 51 nên làm trước 41.

## D-201 · Áp kết quả nghiên cứu các sản phẩm hàng đầu vào thiết kế 2; thêm mục bước 51 và bước 52 — 2026-11-13 — chủ dự án giao, agent quyết nội dung

Quyết định: chủ dự án yêu cầu tham khảo các website và trang quản trị hàng đầu thế giới rồi mang điều
tốt nhất về (`docs/research/ui-design.md` mục 5 ghi nguồn và bằng chứng). Áp như sau:

1. Bước 51 thêm P7–P13: schema bộ lọc công bố trong OpenAPI và view đã lưu; thao tác hàng loạt chung
   (`ids` hoặc `filter` + `expected_count`, job, kết quả từng bản ghi); tìm có toán tử, phạm vi và mở thẳng
   theo mã; hộp việc theo người (khoá ổn định, lý do, hoãn, giao) và việc thiết lập; `/me/can` giải thích
   quyền; agent là người dùng thật (loại API key, `actor_kind`, lệnh rủi ro trả `202 approval_required`
   chờ người duyệt, như nhau cho REST và MCP); dòng hoạt động và ghi chú nội bộ dùng chung cho mọi bản ghi.
   Mục P10 thay phần "có thêm bỏ qua việc hay không" mà đặc tả 51 để agent tự quyết.
2. Bước 52 mới (sau 51): bàn kỹ thuật (nhật ký request, lần giao webhook, chạy lại job), `restore` và
   `dry_run`, lọc bằng lời thường qua AI (chỉ ra tham số đã kiểm), thông báo có lý do và trạng thái, ghim,
   sức khoẻ người bán và tuân thủ.
3. Mặt tiền: thêm mục vào bước 41 (gợi ý tìm hai tầng dấu, từ đồng nghĩa, sắp xếp bán chạy / đánh giá, số
   đếm bộ lọc, thẻ có biến thể và điểm, dữ liệu JSON-LD), 42 (báo khi có hàng lại theo cửa hàng), 43 (mã
   dùng được cho giỏ kèm lý do; số lượt còn lại chỉ khi là số thật), 44 (ngày giao tuyệt đối, giờ chốt,
   lịch là dữ liệu; giờ sẵn sàng khi nhận tại cửa hàng), 45 (mua không cần tài khoản trọn vẹn, trạng thái
   thanh toán công khai bằng token, tra và gắn đơn vãng lai bằng OTP, người nhận hộ, cách trả hàng, đánh
   giá có ảnh và tóm tắt AI chờ duyệt).
4. Để sau, ghi ở "Sau core": khách thử nghiệm cách ly, khách duyệt từng lần nhân sự nền tảng xin vào dữ
   liệu, nhóm nút ngoài cây, quản trị trên điện thoại.

Lý do: những điều trên có bằng chứng (Baymard, tài liệu chính thức) hoặc là mẫu chung của nhiều sản phẩm
đầu ngành, và cần dữ liệu chỉ core có (quyền theo nút, quyền dùng, audit, job). Agent hay retry và cần
người duyệt cho lệnh rủi ro, nên "lệnh chờ duyệt" đi vào core thay vì từng giao diện tự làm.

Đã cân nhắc: đối chiếu đơn vị hành chính cũ sang mới (nghiên cứu đề xuất) — trái D-193 do chủ dự án quyết,
không làm; chuyển menu con sang thanh bên như Vercel 2026 — giữ hàng tab của D-200, xem lại khi có số đo;
dồn mọi mục vào bước 51 — bước quá lớn, tách phần "nên có" sang bước 52.

## D-202 · API còn thiếu cho thiết kế giao diện 2: hỏi đáp, `alt` theo ngôn ngữ, trả hàng, đơn vãng lai, nhật ký lệnh MCP — 2026-11-13 — chủ dự án giao, agent quyết cách làm

Quyết định: năm khoảng trống rà ra từ thiết kế 2 (đề bài `docs/prompts/ui-api-gaps.md`) làm như sau.
Phần A và B có code ngay (module `comments`, `media` đã có); C, D, E chỉ vào đặc tả, làm khi lộ trình tới
bước đó. Mọi thay đổi API là **thêm** (D-059): endpoint, tham số, trường mới; không đổi hành vi hay hình
dạng của endpoint đang có.

**A. Hỏi đáp sản phẩm dựng trên `comments`** (không module mới): câu hỏi là bình luận gốc, trả lời là
trả lời trong luồng.

1. Kernel thêm ba điểm nối trung lập (theo mẫu sổ đối tượng D-123, mặc định rỗng — D-038):
   - `Module::resource_events` → `ResourceEvent { event, kind, change: Closed | Removed }`: module sở hữu
     khai event nào nghĩa là "thôi nhận tương tác" / "đã gỡ". Kernel kiểm lúc khởi động (event do chính
     module khai, loại thuộc module đó, không trùng) và đưa cho mọi module qua
     `StartContext::resource_events`. `comments` bỏ hằng số viết cứng `targets::OWNER_EVENTS`; `content`
     khai `entry_archived → Closed`, `entry_deleted → Removed` (giữ đúng hành vi cũ). Bước 41 khai lưu
     trữ / xoá sản phẩm là `Removed` (đóng và ẩn hỏi đáp).
   - Bộ tính nhãn tác giả `BadgeResolver` trong `ResourceDirectory`: `context.badge_source(kind, codes,
     resolver)` — module nào cũng đăng ký được cho loại của module khác (`orders` tính `verified_buyer`
     cho `catalog.product`), mỗi nhãn của một loại chỉ một module tính, nhãn ngoài danh sách khai bị bỏ.
     `comments` gọi lúc ghi (ngoài transaction), lưu mã vào cột `badges`, đường đọc không gọi chéo module.
     Response công khai trả `badges: ["seller"]` — mã ổn định, giao diện tự dịch.
   - `access::PostingVerifier` + chỗ giữ `PostingVerification` (một module đăng ký qua
     `StartContext::posting_verifier`, thấy cả khi module đó khởi động sau): "tài khoản đã xác thực để
     đăng nội dung chưa". **Bước 37 chưa có**, nên chưa module nào đăng ký: khi đó mọi tài khoản đăng nhập
     tính là đã xác thực — giữ hành vi trước D-202. Bước 37 (mục NĐ 147/2024) đăng ký bộ hỏi thật:
     `compliance` cài `PostingVerifier` dựa trên `PostingGate` / `AccountVerification` của đặc tả 37 (tên
     đó thuộc `compliance`, nên điểm nối của kernel mang tên khác). Phần C2 của đặc tả 37 đổi theo:
     `comments` không `require::<dyn PostingGate>` và không phải dời thứ tự đăng ký.
2. `comments`: cấu hình `AUTOXIDE_COMMENTS_VERIFIED_ONLY_TYPES` (mặc định rỗng — không đổi gì): ở loại
   đích này khách bị từ chối và người dùng phải qua bộ hỏi xác thực, lỗi `comments.verification_required`
   (403). Căn cứ: NĐ 147/2024/NĐ-CP về xác thực tài khoản đăng nội dung — **cần đối chiếu văn bản gốc**
   xem hỏi đáp trên website bán hàng có thuộc phạm vi không; vì vậy là cấu hình theo loại đích.
3. Tìm `q` trên `GET /public/threads`: chỉ bình luận gốc, không phân biệt hoa thường và dấu; khớp đúng dấu
   (cột mới `match_key` = `search::exact_key`, NFC + chữ thường) xếp trước khớp không dấu (`search_key`).
   Hạng phụ thuộc chữ gõ nên không lưu thành cột được: kernel thêm `pagination::fetch_page_tiers` (con trỏ
   mang tầng và ID; mỗi trang tối đa một câu truy vấn mỗi tầng) — một cách làm cho mọi "xếp hai tầng"
   về sau (gợi ý tìm của bước 41).
4. Số đếm: `comments_targets` thêm `published_thread_count` / `pending_thread_count` (đổi cùng
   transaction, cùng câu `UPDATE n = n + k` với số cũ); `GET /public/counts` thêm `thread_count`, đích thêm
   `thread_count` và `pending_thread_count` (quản trị).
5. MCP: `comments.unanswered` (`comments.read`, như `GET /?status=published&unanswered=true` — tham số mới)
   và `comments.answer` (`comments.post`, gọi chính `CommentsService::post` như `POST /public/comments`).
   "Chưa trả lời" là chưa có trả lời đang hiện (`reply_count = 0`), không phân biệt ai trả lời.

**B. `alt` theo ngôn ngữ** (`media`): bảng mới `media_alt_translations` (`media_id`, `locale`, `alt`,
`updated_at`; khoá (`media_id`, `locale`)) bằng migration mới; chưa có `tenant_id` vì `media_item` chưa
theo khách — bước 33 thêm cho cả hai. `media_item.alt` giữ nghĩa bản ngôn ngữ mặc định. Item trả thêm
`alts`; `PUT` / `DELETE /{id}/alts/{locale}` (bắt buộc `If-Match`, event `media.item_updated` kèm `alts`);
tool `media.set_alt`; ngôn ngữ kiểm theo `AUTOXIDE_MEDIA_ALT_LOCALES` (mặc định `en,vi`), sai thì
`media.unsupported_locale`. Trait `Media` thêm `find_many_in(ids, locale)` có cài đặt mặc định; `content`
tra ảnh của bản công khai theo ngôn ngữ của bài. Gợi ý bằng AI: endpoint sẵn có
`POST /{id}/alt-suggestions` đã nhận `locale` và chỉ trả bản nháp (D-109) — không thêm endpoint. Xoá hẳn
ảnh thì xoá bản dịch cùng transaction.

**C, D** vào đặc tả bước 45 (mục "Bổ sung theo D-202"): ảnh kèm yêu cầu trả hàng (kiểm chủ sở hữu qua trait
của `media`), lý do bắt buộc ảnh là chính sách theo nút, `POST /me/orders/{id}/returns/preview` không ghi;
thao tác bằng token xem đơn (huỷ, đổi địa chỉ có OTP lần hai, mở hỗ trợ) qua cùng service, giới hạn là
cấu hình, che số điện thoại và địa chỉ. Tạo tài khoản từ số đã xác thực thuộc **`auth` (bước 34)** — danh
tính và đăng nhập là việc của `auth`; `orders` chỉ gọi `claim` sau đó.

**E** vào đặc tả bước 52: lệnh MCP ghi `mcp:<tool>` vào đường dẫn mẫu, không ghi tham số; lọc theo
`actor_kind` và key; giữ `approval_id` khi lệnh nhận `202 approval_required`.

Lý do: dùng lại `comments` (đã có kiểm duyệt, chống spam, luồng, số đếm, tool) rẻ và đúng hơn một module
hỏi đáp mới; nhãn và event vòng đời là dữ liệu do module sở hữu khai nên thêm loại đích không phải sửa
`comments`. Nhãn tính lúc ghi vì trang sản phẩm đọc nhiều gấp bội ghi và đường đọc không được gọi chéo
module. Bộ hỏi xác thực là chỗ giữ của kernel thay vì `provide` / `require` vì thứ tự đăng ký không được
lặng lẽ làm mất kiểm tra pháp lý (module đăng ký sau vẫn có hiệu lực).

Đã cân nhắc: module `questions` riêng — lặp lại toàn bộ kiểm duyệt của `comments`; cột JSON cho `alt` theo
ngôn ngữ trên `media_item` — khó lọc, khó xoá một ngôn ngữ, đổi hình dạng bảng đang có; xếp hạng tìm
bằng `CASE WHEN … LIKE` trong `ORDER BY` — con trỏ theo ID không giữ được thứ tự, phải dùng `OFFSET`
(luật cấm); kiểm `verified_only` bằng tên role — trái luật kiến trúc 7.

## D-203 · Nút "Đánh giá này hữu ích" cho đánh giá sản phẩm (bước 45) — 2026-11-13 — chủ dự án giao, agent quyết cách làm

Quyết định: đặc tả bước 45 thêm phiếu "hữu ích" cho đánh giá. Bảng `orders_review_votes` (mỗi người một
phiếu cho mỗi đánh giá); cột `orders_reviews.helpful_count` đổi cùng transaction với phiếu; `PUT` / `DELETE
/public/reviews/{id}/helpful` (đăng nhập, gửi lại không đếm thêm); `sort=helpful` phân trang bằng con trỏ
(`fetch_page_by`). Mặc định vẫn sắp theo mới nhất. Không có phiếu "không hữu ích". Người viết đánh giá không
tự bỏ phiếu cho mình; người quản lý đánh giá hay sản phẩm ở nút bán không bỏ phiếu cho sản phẩm của mình.
Không phát event cho từng phiếu.

Lý do: đặc tả D-201 đã có `sort=helpful` nhưng chưa có dữ liệu để sắp. Số đếm lưu sẵn để trang sản phẩm
không phải đếm lại mỗi lần mở. Chặn người bán bỏ phiếu và bỏ phiếu "không hữu ích" để thứ tự này không thành
cách đẩy đánh giá xấu xuống (Luật 19/2023 cấm chặn hay ẩn đánh giá trái quy định — cần đối chiếu văn bản gốc
xem sắp xếp có thuộc phạm vi không).

Đã cân nhắc: cho khách vãng lai bỏ phiếu theo IP băm — dễ dồn phiếu, mất ý nghĩa; nút "không hữu ích" như
một số sàn — dễ bị dùng để hạ đánh giá xấu; tính "hữu ích" bằng đếm `COUNT(*)` lúc đọc — chậm trên sản phẩm
nhiều đánh giá, không dùng được cho con trỏ.

## D-204 · Thêm module `support` (bước 53), hàng đặt trước, nguồn cấp sản phẩm cho Google / Meta — 2026-11-13 — chủ dự án quyết

Quyết định: chủ dự án đồng ý cả ba chỗ còn hở của phần thương mại mà agent rà ra:

1. **Module `support` — hỗ trợ khách hàng, bước 53 mới** (đặc tả `docs/steps/53-support.md`): phiếu hỗ
   trợ và tin nhắn, người mua có tài khoản hay vãng lai (OTP), phạm vi theo nút tổ chức, hạn trả lời theo
   giờ làm việc, phân công, câu trả lời mẫu, tool MCP, gợi ý AI chỉ là bản nháp. Bắt đầu sau bước 37, song
   song với 41–49. `CLAUDE.md` thêm `support` vào danh sách module được dựng. Ranh giới: khiếu nại có hạn
   luật định vẫn là vụ việc `consumer_complaint` của `compliance` (bước 37), tranh chấp vẫn ở `marketplace`
   (bước 46), trả hàng vẫn ở `orders` — `support` chỉ chuyển sang, không làm lại. Nút "mở hỗ trợ" của đơn
   (D-202) gọi `SupportIntake` thay cho event `orders.support_requested`. Thứ tự đăng ký: `support` cùng
   nhóm với `newsletter`, `comments` (sau `notifications`, trước `plans`).
2. **Hàng đặt trước** (mục mới ở bước 42 và 45, bổ sung đặc tả 41, 42, 45): biến thể có `sale_mode =
   preorder` (thay cờ `allow_backorder` bằng `sale_mode` gồm `in_stock` / `backorder` / `preorder`), ngày dự
   kiến có hàng, giới hạn suất giữ bằng `UPDATE` có điều kiện; hàng về chia theo thứ tự đặt; người mua thấy
   ngày giao và đồng ý điều kiện trước khi đặt; dời ngày thì được huỷ miễn phí. Thanh toán: trả hết lúc đặt
   hay khi hàng về; **chưa làm tiền cọc một phần** (cần nghĩa vụ cọc trong sổ cái).
3. **Nguồn cấp sản phẩm cho Google Merchant Center và danh mục của Meta** (mục mới ở bước 48, bổ sung đặc
   tả 48): tệp XML / CSV dựng bằng job theo cửa hàng, ngôn ngữ, tiền tệ, phục vụ bằng link có token, báo
   lỗi từng sản phẩm; nằm trong `channels` vì cùng việc "đưa sản phẩm ra kênh bán", không cần module mới.

Lý do: Luật 19/2023 buộc có kênh tiếp nhận và giải quyết khiếu nại, đặc tả 45 đã có nút "mở hỗ trợ",
nhưng chưa module nào giữ yêu cầu hỗ trợ thường ngày (ai nhận, trạng thái, hạn); dồn vào `compliance` thì
lẫn việc vận hành hằng ngày với hồ sơ pháp lý. Hàng đặt trước và nguồn cấp sản phẩm là việc hằng ngày của
cửa hàng ở Việt Nam (ra mắt sản phẩm, chạy quảng cáo Google / Facebook) mà lộ trình chưa có.

Đã cân nhắc: chat thời gian thực thay cho phiếu — vẫn để sau (D-194), phiếu đủ cho kênh luật định và làm
nền cho chat về sau; dùng `comments` làm phiếu — phiếu là riêng tư, có phân công và hạn, khác bình luận
công khai; module `feeds` riêng — trùng việc với `channels`; tiền cọc đặt trước ngay bây giờ — cần thiết
kế nghĩa vụ trong sổ cái, để khi làm bước 45 hay sau.

## D-205 · Phân quyền chuẩn hàng đầu: bước 54 mới, hợp đồng quyền và test độ phủ ở bước 31 — 2026-11-13 — chủ dự án giao, agent quyết cách làm

Quyết định: chủ dự án yêu cầu phân quyền theo chuẩn các nền tảng lớn và vượt họ. Mô hình chốt: **RBAC theo
cây tổ chức có độ sâu (bước 34) + chia sẻ bản ghi một bước + deny và rào + điều kiện theo ngữ cảnh**, luật
là dữ liệu có kiểu do module khai — không ngôn ngữ chính sách tự do, không thư viện chính sách ngoài.

1. Bước 31 thêm hai mục làm ngay: hợp đồng quyền đọc được bằng máy (`x-permissions` / `x-public` trên mỗi
   operation OpenAPI, `Permission` có mức rủi ro) và test độ phủ trong gate (mọi operation không công khai
   trả 401 / 403 đúng; thiếu khai báo là đỏ). Rà soát ngày 13/11/2026 cho thấy 224 endpoint, 69 tool MCP
   đều kiểm quyền, nhưng chỉ `media` có test cho từng endpoint — endpoint mới quên kiểm quyền sẽ lọt.
2. Bước 54 mới (đặc tả `docs/steps/54-authorization.md`), sau bước 34: một hàm quyết định duy nhất và thu
   hồi tức thì (quyền không nằm trong token, cache theo `kernel_access_version`); test so khớp lọc danh sách
   với kiểm từng bản ghi; deny thắng allow; rào quyền tối đa cho khách con; API key chi tiết (hạn bắt buộc,
   tài nguyên, IP, duyệt); xác thực lại theo RFC 9470; chia sẻ bản ghi có hạn; quyền theo trường dữ liệu
   (che dữ liệu cá nhân, audit đọc dữ liệu nhạy cảm); nâng quyền đúng lúc; lệnh hai người; mô phỏng, "ai có
   quyền", quyền không dùng, rà soát định kỳ; endpoint theo OpenID AuthZEN Authorization API 1.0 (chuẩn cuối
   01/2026); agent hành động thay người dùng có vết.
3. Để sau: OAuth 2.0 cho ứng dụng bên thứ ba, SCIM 2.0.

Lý do: mỗi phần lấy từ một nền tảng đã chứng minh ở quy mô lớn — Dynamics 365 (độ sâu), Google Zanzibar
(quan hệ, "ai có quyền"), AWS IAM (deny, SCP, mô phỏng, quyền không dùng), GitHub (token chi tiết, xác thực
lại), Microsoft Entra PIM (nâng quyền đúng lúc, rà soát), Salesforce (quyền theo trường). Phần vượt trội là
thứ họ không có cùng lúc: thu hồi tức thì ở mọi đường, lọc danh sách được chứng minh khớp kiểm từng bản ghi,
độ phủ được gate chứng minh, giải thích được mọi quyết định, và sinh ra cho AI agent — trên cả hai database.

Đã cân nhắc: đồ thị quan hệ đầy đủ kiểu Zanzibar / OpenFGA — mạnh nhưng bài toán "liệt kê bản ghi được
xem" chậm và cần dịch vụ riêng; ngôn ngữ chính sách Cedar (`cedar-policy`) hay OPA — người thường khó đọc,
điều kiện tự do không dịch được thành một câu SQL chạy trên cả hai database, thêm thư viện lớn; nhét phạm
vi vào JWT — mất thu hồi tức thì.

## D-206 · Mẫu mail và thông báo động: bộ dựng mẫu dùng chung, không module `email` riêng — 2026-11-13 — chủ dự án đồng ý, agent quyết cách làm

Quyết định: không viết module `email`. Thêm `autoxide_kernel::template` — bộ dựng mẫu an toàn cho mẫu do
khách tự soạn (biến, điều kiện, vòng lặp; không đọc tệp / mạng; giới hạn kích thước và số bước chạy; luôn
thoát HTML; kiểm biến theo JSON Schema mà loại thông báo khai) — và khung mail HTML viết bằng MJML (dựng
bằng `mrml`), luôn kèm bản chữ thuần. `notifications` (bước 37) dùng nó cho mẫu theo khách (nháp, xuất bản,
quay lại phiên bản, xem trước, gửi thử tới địa chỉ của chính người gọi, thương hiệu theo nút); `newsletter`
dùng nó để cá nhân hoá chiến dịch. Thư viện ngôn ngữ mẫu chọn giữa `liquid` và `minijinja` bằng đo khi làm,
ghi `STACK.md`; chỉ một ngôn ngữ mẫu trong dự án. Mục mới ở bước 37, đặc tả ở cuối `docs/steps/37-compliance.md`.

Lý do: gửi mail (`mail`), mẫu theo khách (`notifications_templates`, bước 37) và chiến dịch (`newsletter`)
đã có chỗ; module mới sẽ thành hai nơi giữ mẫu. Phần thiếu thật là ngôn ngữ mẫu an toàn và HTML chạy đúng
mọi trình đọc mail — đặt ở kernel để mọi module dùng một cách. Làm cùng bước 37 vì sổ đồng ý (tin quảng
cáo luôn có chân thư huỷ nhận) và mẫu theo khách (cần `tenant_id`, bước 33) nằm ở đó.

Đã cân nhắc: module `email` riêng có trình soạn kéo thả — giao diện thuộc repo riêng (D-097), core chỉ cần
mẫu và API xem trước; Handlebars / Tera — Tera không có vùng cách ly cho mẫu không tin cậy, Handlebars không
có giới hạn chạy rõ ràng; tự viết ngôn ngữ mẫu — vòng lặp, điều kiện, giới hạn và báo lỗi theo dòng cột vượt
xa "vài chục dòng"; gửi thử tới địa chỉ tuỳ ý — thành công cụ gửi thư rác.

## D-207 · API cho trợ giúp trong giao diện: cờ theo người dùng, định nghĩa số liệu — 2026-11-14 — agent quyết

Quyết định: làm theo đề bài `docs/prompts/ui-help-api.md`.

1. **Cờ theo người dùng — đã làm.** Bảng `kernel_user_flags` của module nội bộ `kernel` (migration
   `m0002_create_user_flags` có `down`): `id` (UUIDv7), `user_id`, `key` (1–100 ký tự `a-z0-9._-`),
   `value` (≤ 200 ký tự), `updated_at`; duy nhất (`user_id`, `key`). API `GET /api/v1/me/flags?prefix=`,
   `PUT /api/v1/me/flags/{key}` (ghi đè bằng `ON CONFLICT`, gọi lại an toàn), `DELETE …/{key}` (204 kể cả
   khi không có). Chỉ cần đăng nhập, không permission, chỉ cờ của chính người gọi; không tool MCP (việc của
   giao diện). Hạn mức `AUTOXIDE_USER_FLAGS_MAX` (mặc định 500) → 409 `kernel.flag_limit_reached`; hai
   request tạo cờ mới cùng lúc có thể vượt mức vài cờ — chấp nhận, đây là hạn mức chống lạm dụng.
   - `/api/v1/me` là tiền tố dành riêng cho endpoint "của tôi" do kernel phục vụ (sau này `/me/views`,
     `/me/can`, `/me/work` của bước 51 và `/me/pins` của 52 cũng ở đây): `Modules::routes` thêm mục
     `("me", …)`, tên module `me` bị cấm như `kernel` và `antispam`.
   - Dữ liệu cá nhân: dùng cơ chế đã có của kernel (`PersonalData` trong bản xuất, nghe
     `privacy::USER_DELETED`), không cần chờ bước 37.
   - **Chưa có `tenant_id`**: chưa bảng nào có (bước 33 thêm cho mọi bảng cấp khách hàng cùng lúc). Test
     `flag_cua_khach_nay_khong_lo_sang_khach_khac` chuyển sang danh sách test của bước 33.
2. **Định nghĩa số liệu — chỉ vào đặc tả.** Trait `SummaryMetric` (P4 bước 51) chưa có code, nên
   `fn definition(&self) -> Option<&'static str>` (mặc định `None`, D-038), trường `definition` của
   `GET /api/v1/kernel/summary` và bài kiểm gate "mọi số liệu khai định nghĩa và có bản dịch" ghi vào đặc tả
   51 mục P4; làm cùng P4.

Lý do: lưu "đã xem gợi ý" ở trình duyệt thì đổi máy là hiện lại; lưu ở máy chủ cần một chỗ chung thay vì
mỗi module tự làm. Câu định nghĩa số liệu nằm cạnh code tính số để giao diện không nói sai cách tính.

Đã cân nhắc: đặt endpoint dưới `/api/v1/kernel/me/flags` — đúng quy tắc tiền tố module nhưng đường dẫn
"của tôi" rải rác theo module; đặc tả 51 đã dùng `/api/v1/me/…` nên gom về một tiền tố; lưu cờ trong
`auth` — kernel không được phụ thuộc module `auth`, và các endpoint `/me` khác của bước 51 là của kernel.

## D-208 · Nội dung và media theo website, theo nút tổ chức (H1) — 2026-11-14 — agent quyết

Quyết định: `content` và `media` có `org_id` (nút sở hữu); `forms` có `org_id` tuỳ chọn. Quyền theo độ sâu
(bước 34) áp cho chúng. Đọc công khai lọc theo website: tên miền → nút qua `kernel_tenant_domains.org_id`
(bước 33); menu và ngôn ngữ bật theo website. Bộ lọc quản trị `owner_org`, `site`. Thay câu "`org_id` chỉ
thêm vào `notes`; `content`, `media`, `forms`… giữ dữ liệu cấp khách" ở `docs/steps/34-organizations-identity.md`
bằng mục "Bổ sung theo D-208"; `newsletter`, `comments` vẫn cấp khách.

Lý do: mô hình chuỗi và sàn của D-185 cần cửa hàng và người bán có bài, trang, ảnh riêng (A-b1, A-c3 trong
`docs/research/ui-trace/A-nen-tang.md`). Câu cũ do agent quyết ở đặc tả 34, không phải chủ dự án.

Đã cân nhắc: một khách riêng cho mỗi website — mất báo cáo và quản trị chung của chuỗi; thẻ "website" tự do
trên bài — không có quyền theo nhánh.

## D-209 · "Chờ xác nhận" đơn là chính sách theo nút (H2) — 2026-11-14 — agent quyết

Quyết định: `orders_policies.acceptance_hours` (mặc định 0 = tự xác nhận như đặc tả 45 hiện tại). Khác 0
thì đơn đã thanh toán / COD chờ cửa hàng hay người bán bấm nhận; quá hạn tạo việc trong hộp việc (bước 51)
rồi huỷ theo chính sách, hoàn tiền tự động, tính vào `cancelled_by_seller` của bước 46. Sàn đặt mức cho
người bán (kế thừa theo cây).

Lý do: sàn ở Việt Nam thường buộc người bán nhận đơn; chỉ số `cancelled_by_seller` cần mốc này (B-b6).

## D-210 · Tách nhánh thành khách riêng, chuyển database: chỉ nhân sự nền tảng (H3) — 2026-11-14 — agent quyết

Quyết định: khách chỉ "gửi yêu cầu" tách / chuyển. Mặt điều khiển có API job cho nhân sự nền tảng:
`POST /api/v1/kernel/placements/migrations`, `POST /api/v1/kernel/tenants/{id}/moves`, `dry_run` mặc định
`true`, tiến độ qua job API chung (D-213). CLI (`autoxide tenant split|merge|move`) vẫn là cách chính.

Lý do: việc không hoàn tác được trên dữ liệu nhiều khách; đặc tả 38:286 đã để CLI là chính (A-c6).

## D-211 · Khách tự thu hồi quyền vào hỗ trợ (H4) — 2026-11-14 — agent quyết

Quyết định: giữ `AUTOXIDE_AUTH_SUPPORT_ACCESS_MAX_HOURS` mặc định 8, khoảng 1–72 như đặc tả 34. Thêm: quản
trị của khách tự thu hồi một lần cấp đang còn hạn (`DELETE /api/v1/auth/support-access/{id}`, permission
`auth.support_access.revoke` ở khách), event `auth.support_access_revoked` mang `revoked_by_tenant`. Lần cấp
theo yêu cầu cơ quan có thẩm quyền (`legal_request`) không thu hồi được từ phía khách.

Lý do: khách phải kiểm soát được ai đang vào dữ liệu của mình (Luật 91/2025 — cần đối chiếu văn bản gốc).

## D-212 · Hộp việc không có nút "Đã xử lý" (H5) — 2026-11-14 — agent quyết

Quyết định: việc tự hết khi bản ghi gốc xử lý xong; người dùng chỉ Hoãn và Giao (P10 bước 51). Không có
trạng thái "đã xử lý" hay "bỏ qua" lưu riêng. Thiết kế đã sửa màn hình theo.

Lý do: việc tính từ dữ liệu (51:99); đánh dấu tay làm hộp việc lệch dữ liệu thật.

## D-213 · Job API chung, xuất danh sách chung, yêu cầu duyệt đầy đủ (bước 51) — 2026-11-14 — agent quyết

Quyết định: kernel có `GET /api/v1/kernel/jobs?status=&kind=`, `GET …/{id}` (`progress`, `result`, `file`),
`POST …/{id}/retry` (audit), permission `kernel.jobs.read` / `kernel.jobs.manage`; file kết quả giữ có hạn,
tải bằng link ký. Xuất danh sách dùng chung (bộ lọc theo schema P7 → job → CSV qua `autoxide_kernel::csv`
→ báo xong qua `notifications`); module khai cột xuất được, không tự làm xuất riêng. Yêu cầu duyệt (P12) có
`GET /api/v1/kernel/approvals?status=`, `GET …/{id}`, `POST …/reject {reason}`, event
`kernel.approval_decided`, tool MCP tương ứng.

Lý do: P8 (51:239) đã dẫn tới job API mà chưa định nghĩa; forms, catalog (41:237), orders đều cần xuất (A-b2,
A-b8, A-c1, A-c2, B-c6).

## D-214 · Không chat với người bán trong core; huy hiệu chỉ từ dữ liệu có thật (H6) — 2026-11-14 — agent quyết

Quyết định: "Chat với người bán" vẫn ở "Sau core" (D-194). Huy hiệu người bán tin cậy (bước 46, 52) không
dùng "tỉ lệ trả lời tin nhắn" — chỉ chỉ số có dữ liệu trong core (giao đúng hạn, huỷ do người bán, đánh giá,
tranh chấp thua, thời gian trả lời phiếu hỗ trợ của bước 53 khi có).

Lý do: không có tin nhắn thì chỉ số đó là bịa (B-b11).

## D-215 · Thời điểm lập hoá đơn mặc định khi giao xong (H7) — 2026-11-14 — agent quyết

Quyết định: `orders_policies.invoice_timing` thêm `on_delivered` và lấy làm mặc định; `on_complete` chỉ dùng
khi khách có căn cứ; giữ `on_confirm`, `on_request`. Căn cứ: NĐ 123/2020/NĐ-CP Điều 9 (thời điểm lập hoá đơn
khi bán hàng hoá là thời điểm chuyển giao quyền sở hữu hay quyền sử dụng), sửa bởi NĐ 70/2025/NĐ-CP — **cần
đối chiếu văn bản gốc**.

Lý do: giao xong là lúc chuyển giao hàng hoá với đa số đơn thương mại điện tử (B-b5).

## D-216 · COD và nhiều chế độ dòng tiền trong một checkout (H8, H9) — 2026-11-14 — agent quyết

Quyết định: (1) tiền COD **không bao giờ** về tài khoản của nhà vận hành sàn — về tài khoản thu hộ của đối
tác có giấy phép, hoặc thẳng về người bán. Người bán tự thu COD thì phí sàn và khấu trừ thay thu bằng hoá đơn
phí định kỳ hoặc công nợ. Cột `ledger_flow_settings.cod_remittance`; lỗi `marketplace.cod_flow_unsupported`;
event `shipping.cod_settled` mang `seller_org_id`. Căn cứ: NĐ 52/2024/NĐ-CP (thanh toán không dùng tiền
mặt), NĐ 117/2025/NĐ-CP (khấu trừ thuế của nền tảng) — **cần đối chiếu văn bản gốc**. (2) Một giỏ có nhiều
nút: chỉ một mã QR khi mọi nút cùng đối tác thu hộ; khác chế độ thì checkout trả `payment_actions[]`, mỗi
chế độ một mục, giao diện hiện từng bước.

Lý do: D-187 — core không giữ tiền người mua (B-b1, B-b2).

## D-217 · Yêu cầu dữ liệu cá nhân người dùng tự làm không cần duyệt (H10) — 2026-11-14 — agent quyết

Quyết định: xuất và xoá dữ liệu mà người dùng tự bấm trong tài khoản chạy ngay, không qua duyệt. Sổ yêu cầu
của chủ thể dữ liệu (`compliance_subject_requests`, có hạn trả lời là dữ liệu có ngày hiệu lực) chỉ dành cho
yêu cầu qua kênh khác (email, thư, cơ quan). Căn cứ hạn trả lời: Luật 91/2025/QH15, NĐ 356/2025 — **cần đối
chiếu văn bản gốc**.

Lý do: người dùng đã xác thực tự làm thì không cần người trung gian (A-b3).

## D-218 · Thống nhất tiền giữa bước 36, 45, 46: tên tài khoản, tạm giữ, hạn thanh toán — 2026-11-14 — agent quyết

Quyết định: theo luật "đặc tả của module cung cấp hợp đồng thắng" (`docs/steps/README.md`):

1. Bộ tên tài khoản của đặc tả 36 (`clearing.gateway`, `clearing.carrier_cod`, `payable.seller_held[S]`,
   `payable.seller_available[S]`, `revenue.*`, `tax.*`…) là bộ duy nhất; bảng bút toán của đặc tả 46
   (`buyer_prepayments`, `seller_payable:<seller>`…) viết lại theo tên đó khi làm bước 46.
2. Tiền thu hộ ghi lúc thu vào `clearing.*` của đối tác; phí sàn và khấu trừ thay ghi lúc
   `orders.order_completed` (như đặc tả 46).
3. Một cơ chế tạm giữ: `ledger_holds` của 36. `marketplace_settlements.hold_until` bỏ; trạng thái giữ / được
   rút đọc từ `ledger_holds` qua `api` của `ledger`.
4. Một hạn thanh toán: `AUTOXIDE_LEDGER_PAYMENT_EXPIRY_MINUTES` (36, mặc định 30) là nguồn duy nhất cho mọi
   thanh toán `pending`, kể cả VietQR; đặc tả 45 bỏ giá trị 1440 riêng và đọc hạn từ `ledger`. Chuyển khoản
   chậm hơn hạn vẫn được khớp vào hàng chờ sai lệch của 36, không mất tiền.

Lý do: ba đặc tả viết song song nên lệch (B-b3, B-b10, B-3.2d, B-c12); để lệch thì bước 46 sẽ ghi hai bút
toán cho cùng một đồng tiền.

## D-219 · Giao diện cửa hàng qua API (bước 55) và chuyển dữ liệu từ nền tảng khác (bước 56) — 2026-11-13 — agent quyết

Quyết định: thêm bước 55 — vị trí hiển thị và banner trong `content` (lịch hiển thị, theo cửa hàng /
nhánh, link tới đối tượng của module đăng ký sau qua điểm cắm `ContentLinks`, tự ẩn khi chương trình
khuyến mại kết thúc), bộ sưu tập sản phẩm trong `catalog` (chọn tay hay theo quy tắc, sắp bán chạy từ
event tuỳ chọn của `orders`), thông tin cửa hàng công khai là cài đặt kế thừa của nút trong
`organizations`. Thêm bước 56 — module `imports`: nhập file xuất của Shopify, WooCommerce, Haravan, Sapo,
KiotViet bằng hồ sơ ánh xạ cột là dữ liệu có phiên bản, chạy thử trước, chạy lại không trùng, chuyển
hướng 301 từ đường dẫn cũ, không mang đồng ý quảng cáo và mật khẩu cũ. Đo hành vi người xem để sau (cần
đồng ý cookie; hiện dùng công cụ ngoài).

Lý do: chủ dự án hỏi hệ thống đã có biến thể, bảng giá, khuyến mại, voucher, banner và các module hỗ
trợ như hệ thống lớn chưa (13/11/2026). Rà lộ trình thấy banner, bộ sưu tập, thông tin cửa hàng và chuyển
dữ liệu từ nền tảng khác chưa có ở đâu, trong khi các nền tảng thương mại điện tử đều có và mọi khách
chuyển nhà đều cần. Banner đặt trong `content` vì website không bán hàng cũng dùng; bộ sưu tập trong
`catalog` vì là truy vấn trên sản phẩm.

Đã cân nhắc: module `storefront` riêng — chỉ gom vài bảng của ba module, thêm hợp đồng mà không bớt phụ
thuộc; nhập qua API của nền tảng cũ — cần tài khoản đối tác, để làm như bộ nối (D-191); trình dựng trang
kéo thả — trái D-026 (core chỉ có API).

## D-220 · Thêm `analytics` (bước 57), `automations` (bước 58), tìm kiếm thay được, đẩy di động — 2026-11-14 — chủ dự án quyết

Quyết định: chủ dự án đồng ý các chỗ hở agent rà ra khi soát lộ trình:

1. **Module `analytics`, bước 57 mới** (sau 37): số liệu truy cập tự lưu, mặc định không cookie (khoá phiên
   HMAC xoay và xoá mỗi ngày, không lưu IP hay user agent đầy đủ), chỉ giữ số gộp lâu dài; theo dõi theo người
   chỉ khi đồng ý qua sổ đồng ý của bước 37. Cấm dấu vân tay trình duyệt.
2. **Module `automations`, bước 58 mới** (sau 51): luật "khi … thì …" — kích hoạt bằng event / lịch, điều kiện
   là cây toán tử cố định, hành động là tool MCP chạy **với quyền hiện tại của người tạo luật**; hành động rủi
   ro chờ duyệt (P12), chống vòng lặp, khoá chống chạy trùng, `Idempotency-Key` mỗi bước.
3. **Nhập dữ liệu:** không làm khung nhập ở kernel — dùng module `imports` của bước 56 (D-219) làm cách nhập
   duy nhất, mở rộng thêm nguồn `autoxide` (cùng cột với xuất chung P16 bước 51) và đích `content`, `media`,
   `newsletter` (người đăng ký chỉ khi có bằng chứng đồng ý).
4. **Bộ máy tìm kiếm thay được** (P7 bước 40): trait `SearchBackend`, mặc định trong database (chạy như nhau trên
   hai database), backend ngoài là module riêng làm khi đo thấy cần.
5. **Kênh đẩy di động** (`MobilePush`) cho `notifications` ở bước 37; bộ nối Firebase / Apple cần khoá thật —
   khi đó dừng lại hỏi.

Thay một phần D-219: câu "đo hành vi người xem để sau (cần đồng ý cookie; hiện dùng công cụ ngoài)" — chủ dự
án đã duyệt làm `analytics` không cookie, không công cụ ngoài. D-219 do agent quyết nên thay được.

`CLAUDE.md` thêm `analytics`, `automations` vào danh sách module được dựng. Thứ tự đăng ký: `analytics` cùng nhóm
với `newsletter`, `comments`, `support`; `automations` đăng ký cuối cùng (sau `imports`, `reporting`) (nghe event và gọi tool của mọi module).
Đặt lịch dịch vụ và ký hợp đồng điện tử vẫn chờ chủ dự án quyết hướng sản phẩm.

Lý do: chủ cửa hàng nào cũng cần số liệu truy cập và tự động hoá; chuyển từ nền tảng khác cần nhập dữ liệu;
sàn lớn cần tìm kiếm vượt giới hạn "chứa chuỗi" của kernel. Thiết kế giữ luật cũ: không gửi dữ liệu người đọc
cho bên thứ ba, không đường thứ hai làm điều REST / MCP không làm được, mọi thứ chạy trên cả hai database.

Đã cân nhắc: nhúng Google Analytics hay công cụ ngoài — gửi dữ liệu người đọc ra ngoài, cần đồng ý cookie; luật
tự động hoá bằng ngôn ngữ kịch bản (Lua, JavaScript) — không kiểm được lúc lưu, khó giải thích, rủi ro bảo mật;
chỉ dùng webhook cho tự động hoá — cần máy chủ ngoài, khó cho người không biết lập trình; tìm kiếm Postgres
full-text — không có trên SQLite, trái luật số một.

## D-221 · Đưa tám mảng "để sau" lên lộ trình: bước 59–66 — 2026-11-14 — chủ dự án quyết

Quyết định: chủ dự án đưa cả tám mảng đang ở mục "Sau core" lên lộ trình, mỗi mảng một bước có đặc tả:

| Bước | Mảng | Module | Bắt đầu sau |
|---|---|---|---|
| 59 | Bán sỉ B2B: công ty mua, giá sỉ, báo giá, công nợ, hạn mức | `wholesale` | 45, 47 |
| 60 | Đặt lịch dịch vụ: giờ trống, chống chồng giờ, cọc qua đối tác | `bookings` | 34, 36 |
| 61 | Chat chăm sóc khách qua SSE, chuyển thành phiếu hỗ trợ | `conversations` | 40, 53 |
| 62 | Sản phẩm bán theo kỳ (người mua trả cửa hàng; khác `plans`) | `recurring` | 45, 36 |
| 63 | Đa tiền tệ, bán xuyên biên giới (mở rộng `ledger`, `tax`, `catalog`, `orders`, `shipping`) | — | 36, 41, 44, 45 |
| 64 | Quảng cáo trong sàn, luôn gắn nhãn, phí là doanh thu dịch vụ của sàn | `ads` | 46, 57 |
| 65 | Chợ ứng dụng qua OAuth 2.0 (PKCE bắt buộc, phạm vi là permission) | `apps` (+ `auth`) | 54, 33 |
| 66 | Đồng bộ nhân viên qua SCIM 2.0 | trong `auth` | 34, 40 |

Thay: mục "Để sau (D-194)" về B2B, đặt lịch, chat, bán theo kỳ, đa tiền tệ, quảng cáo trong sàn, và mục "Để sau
(D-205)" về OAuth, SCIM. Câu "đo hành vi người xem… để Google Analytics làm" còn sót trong mục "Sau core" bỏ, vì
đã là bước 57 (D-220). `CLAUDE.md` thêm `wholesale`, `bookings`, `conversations`, `recurring`, `ads`, `apps` vào
danh sách module được dựng.

Những ràng buộc giữ từ luật cũ, ghi trong từng đặc tả:
- tiền đi qua đối tác có giấy phép (D-187): cọc đặt lịch, trừ tiền định kỳ, thanh toán ngoại tệ; quảng cáo
  không có "ví" giữ tiền trả trước của người bán;
- chống ghi đồng thời bằng câu lệnh có điều kiện hay khoá duy nhất chạy trên cả hai database (hạn mức tín dụng,
  ô giờ đặt lịch, ngân sách quảng cáo);
- người mua trong nước chỉ VND — **cần đối chiếu** Pháp lệnh Ngoại hối 2005 (sửa 2013) và Thông tư
  32/2013/TT-NHNN trước khi bật đa tiền tệ; trích tiền định kỳ — **cần đối chiếu** NĐ 52/2024 và quy định của NHNN
  về ủy quyền trích nợ; nhãn quảng cáo — **cần đối chiếu** Luật Quảng cáo và văn bản sửa đổi hiện hành.

Còn để sau: bán ứng dụng trả phí trên chợ (chia doanh thu), tiền cọc một phần cho hàng đặt trước, đấu thầu và
hợp đồng khung B2B, gọi điện / tổng đài trong chat. Ký hợp đồng điện tử vẫn chờ chủ dự án quyết hướng.

Lý do: Autoxide phục vụ mọi mô hình kinh doanh (D-185); tám mảng này là thứ các nền tảng lớn đều có, và người
dùng hỏi tới khi chọn nền tảng.

Đã cân nhắc: gộp bán theo kỳ vào `plans` — khác người trả tiền và khác cách ghi sổ, chỉ dùng chung bộ tính lịch
kỳ; module `fx` riêng cho đa tiền tệ — tiền tệ đã có trong `money`, tỉ giá thuộc `ledger`; WebSocket cho chat —
SSE của bước 40 đủ cho tin đến, gửi tin vẫn là REST, một cách làm.

## D-222 · Bảng giá và khuyến mại theo các nền tảng lớn; chương trình, coupon, voucher, thẻ quà tặng — 2026-11-14 — chủ dự án quyết

Quyết định: chủ dự án đồng ý toàn bộ đề xuất sau khi đối chiếu đặc tả 41 và 43 với commercetools (Product
Discount / Cart Discount / Discount Code, chế độ cộng dồn, nhóm "lợi nhất"), Shopify (ba lớp giảm giá,
`combinesWith`), Stripe (Coupon là luật, Promotion Code là mã trỏ tới luật), Salesforce B2C (lớp, độc quyền
theo lớp / toàn giỏ, thứ hạng) và Shopee (voucher shop / voucher sàn, kho voucher, voucher vận chuyển).

**1. Bốn khái niệm, mỗi tên một nghĩa trong toàn hệ thống** (tên kiểu, bảng, API, tài liệu đều theo):

| Tên | Là gì | Ai dùng được |
|---|---|---|
| Chương trình khuyến mại (`promotion`) | **Luật**: điều kiện, lợi ích, thời gian, ngân sách, cộng dồn, ai trả tiền. Đến tay người mua bằng một trong ba cách: `automatic`, `code`, `voucher` | — |
| Coupon — mã giảm giá (`code`) | Chuỗi mã người mua gõ hay chọn để mở một chương trình `code`. Mã chung (`SALE50`) hay lô mã riêng mỗi mã một lượt | **Ai có mã cũng dùng được**, kể cả khách vãng lai |
| Voucher — phiếu ưu đãi đã cấp (`voucher`) | Một quyền dùng một lần **nằm trong ví của một tài khoản**, có hạn riêng; có được do bấm "Lưu", nhập mã nhận, được tặng (sinh nhật, đổi điểm, sau khi mua, bù lỗi, giới thiệu bạn) | **Chỉ đúng tài khoản được cấp**; không chuyển, không đổi ra tiền |
| Thẻ quà tặng (`gift card`, bước 47) | Tiền trả trước, là **cách thanh toán**, không phải khuyến mại; tắt mặc định (NĐ 52/2024) | — |

Luật phân định: mã gắn với một người thì là voucher, không làm coupon "chỉ một người dùng". Coupon và voucher
đều là khoản **giảm giá** (không phải tiền): cùng chịu trần pháp luật, cùng ghi "ai trả tiền", cùng hiện trên
hoá đơn là khoản giảm. Mã dùng để **nhận** voucher (`claim`) khác mã dùng để **giảm** (`redeem`), cùng bảng mã.

**2. Bảng giá (đặc tả 41):** thứ tự chọn bảng giá chung (nút gần nhất → công ty B2B → nhóm khách → kênh →
thị trường → `priority` → `id`), cột cho bước 48 / 59 / 63 có sẵn từ migration đầu; bảng giá suy ra theo
công thức (một tầng, ghi thành dòng giá thật bằng job); quy tắc làm tròn giá đẹp dùng chung; chặn / cảnh báo
giá dưới sàn (giá tối thiểu, biên lợi nhuận trên giá vốn lấy qua event của `inventory`); đổi giá hàng loạt có
xem trước, duyệt, hẹn giờ, giá tạm thời.

**3. Khuyến mại (đặc tả 43):** ba lớp `product` / `order` / `shipping`, `exclusivity` `none` / `class` /
`global` thay cờ `exclusive`, `combines_with`, `stop_after`; chế độ cộng dồn `priority` (mặc định) hay
`best_for_buyer` (so tập cộng dồn với từng chương trình độc quyền, có giới hạn, vẫn tất định); ai trả tiền
(`platform` / `seller` / `brand` / `payment_partner`, chia theo basis point); ngân sách tiền; lượt theo ngày,
khung giờ; loại mới `tiered`, `bundle_price`, `add_on`, `gift_choice` và mua X tặng Y mở rộng; điều kiện mở rộng
qua một bảng điều kiện chung; chống lạm dụng theo nhiều khoá người mua; ví voucher và `grant_voucher` cho
module khác; đăng ký tham gia chương trình của sàn; chạy thử, kiểm xung đột, lịch, số liệu chương trình; mục
tiêu tốc độ đo được.

**4. Module khác:** 45 (dữ liệu giỏ mới, chia tiền giảm khi trả hàng, quà khi trả, hoá đơn), 46 (quyết toán theo
người trả tiền giảm, gốc tính phí sàn), 47 (cấp voucher sinh nhật / đổi điểm / sau khi mua / giới thiệu; phân
khúc và hạng vào giỏ), 48 (giá theo kênh là bảng giá suy ra), 53 và 58 (cấp voucher bù lỗi / theo quy trình).

Nhẹ và nhanh (ràng buộc thiết kế, kiểm bằng số đo):
- trang danh sách không chạy luật khuyến mại: giá sau giảm tự động ghi sẵn vào `catalog_storefront_entries`
  theo event; giá suy ra ghi sẵn thành dòng giá;
- `evaluate` chạy trong bộ nhớ trên chỉ mục chương trình đang chạy theo (khách, cửa hàng), nạp lại theo số phiên
  bản; chỉ xét chương trình có mục tiêu trùng giỏ. Mục tiêu: giỏ 50 dòng, 500 chương trình đang chạy, p99 < 5 ms
  phần tính, ghi số đo vào `docs/BENCHMARKS.md`;
- mọi bộ đếm (lượt, ngân sách, suất, ví voucher, lượt theo ngày) đổi bằng `UPDATE` có điều kiện trong một
  transaction, khoá theo thứ tự cố định; chạy được và có test đồng thời trên cả SQLite và Postgres.

Căn cứ pháp lý — **cần người có chuyên môn pháp lý xác nhận trước khi chạy thật** (D-186), mọi con số là dữ
liệu có ngày hiệu lực:
- Luật Thương mại 36/2005/QH11, Điều 92: các hình thức khuyến mại, trong đó "bán hàng, cung ứng dịch vụ có kèm
  theo phiếu mua hàng, phiếu sử dụng dịch vụ" — voucher cấp sau khi mua thuộc hình thức này; quy tắc thông báo
  theo hình thức là dữ liệu `promotions_notice_rules`.
- Thông tư 39/2025/TT-BCT và NĐ 239/2026 (sửa NĐ 81/2018): trần giảm giá 50% giá ngay trước khuyến mại. Trên sàn,
  khi voucher sàn và voucher của người bán cùng áp, mặc định **cộng mọi khoản giảm trên một đơn vị rồi so trần**
  (cách an toàn) — chưa rõ trần tính riêng cho từng thương nhân hay gộp.
- NĐ 123/2020 (sửa bởi NĐ 70/2025) về hoá đơn: cách ghi khoản giảm giá, hàng khuyến mại, phần giảm do sàn hay đối
  tác thanh toán trả — bước 45 làm theo dữ liệu, cần đối chiếu.
- Luật 19/2023/QH15: điều kiện của coupon / voucher hiện rõ trước khi dùng; số lượt / suất còn là số thật.
- Luật Bảo vệ dữ liệu cá nhân: ví voucher là dữ liệu cá nhân — có trong xuất dữ liệu, xoá tài khoản thì thu hồi
  voucher chưa dùng; khoá chống lạm dụng chỉ lưu bản băm.

Lý do: người dùng thường lẫn "voucher" và "coupon"; mỗi nền tảng gọi một kiểu. Chốt theo **ai dùng được** (ai có
mã hay chỉ người được cấp) thì dễ hiểu, khớp thói quen ở Việt Nam ("kho voucher", "nhập mã") và khớp mô hình
Stripe / commercetools (luật tách khỏi cách đưa tới người mua).

Đã cân nhắc:
- Gọi mọi thứ là "voucher" như nhiều sàn Việt Nam — mất phân biệt mã ai cũng dùng được và quyền của một người,
  khiến hạn mức, chống lạm dụng và dữ liệu cá nhân xử lý lẫn lộn.
- Chọn bảng giá "rẻ nhất" (như một số nền tảng B2B) — khó đoán, người bán không kiểm soát; giữ "cụ thể nhất".
- Tìm tổ hợp khuyến mại tối ưu toàn cục — bài toán tổ hợp, chậm và khó giải thích; `best_for_buyer` chỉ so có
  giới hạn.
- Bảng giá suy ra tính lúc đọc — nhanh viết nhưng chậm đường nóng và mất lịch sử giá thật (cần cho trần 50%);
  ghi sẵn thành dòng giá.

## D-223 · Danh sách lý do theo khách: một cách làm cho mọi module — 2026-11-14 — agent quyết

Quyết định: mọi danh sách "lý do" khách cần chọn (điều chỉnh tồn, ẩn đánh giá, từ chối trả hàng, lý do trả hàng
cho phép, chênh lệch tiền két, khớp dòng lệch COD…) đi qua một tiện ích của kernel, mục P18 của bước 51:
- module khai danh sách và bộ mã mặc định qua `Module::reason_lists()` (cài đặt mặc định trả rỗng — D-038); tên
  danh sách `<module>.<việc>`, mã `snake_case`, nhãn mặc định là i18n key `<module>.reason.<việc>.<mã>`;
- khách thêm mã, đổi nhãn, tắt mã mặc định, sắp thứ tự (`kernel_reason_codes`: `tenant_id`, `list`, `code`,
  `labels` JSON, `active`, `position`, `builtin`, `updated_at`; unique (`tenant_id`, `list`, `code`));
- mã mặc định đánh dấu `legal` (vd lý do ẩn đánh giá theo gói `vn` của bước 37) không tắt được;
- service kiểm bằng `ReasonCodes::ensure(tenant, list, code)`; dòng dữ liệu lưu mã, không lưu nhãn — đổi nhãn
  không sửa lịch sử; mã đã dùng không xoá được, chỉ tắt.

Lý do: đợt 2 của thiết kế lộ ra ít nhất sáu danh sách lý do ở bốn module; mỗi module tự làm bảng riêng thì sáu
cách làm, sáu bộ endpoint. Một tiện ích chung giữ "mỗi việc một cách làm" và giao diện học một lần.

Đã cân nhắc: viết cứng danh sách trong code — khách không thêm được lý do của mình; mỗi module một bảng — lặp và
lệch nhau.

## D-224 · `ActorKind` thống nhất cho audit, nhật ký lệnh và event — 2026-11-14 — agent quyết

Quyết định: kernel có một enum `ActorKind` (`#[non_exhaustive]`): `human`, `agent` (API key loại agent), `service`
(API key loại dịch vụ), `system` (job, lịch), `support` (nhân sự hỗ trợ của nền tảng vào khách theo D-211),
`app` (ứng dụng OAuth, bước 65), `scim` (bước 66). `Principal` mang nó; audit, `kernel_request_log` (bước 52) và
envelope event (bước 40) ghi cùng giá trị; danh sách audit lọc `actor_kind=` (nhiều giá trị). Thay các tập con
đang ghi rải rác ở 36, 51, 52, 65, 66.

Lý do: khách cần thấy việc nào do người, việc nào do AI agent hay nhân sự nền tảng làm; một tập giá trị cho mọi
nơi thì lọc được xuyên suốt.

## D-225 · Khuyến mại: thời hạn bắt buộc, thông báo theo nơi nhận, kiểm trước khi bật, "Lưu mã", nhắc flash sale — 2026-11-14 — agent quyết

Quyết định:
- **`ends_at` vẫn bắt buộc.** Luật Thương mại 2005 Điều 97 yêu cầu công bố thời gian bắt đầu, kết thúc khuyến mại;
  NĐ 81/2018 (sửa bởi NĐ 128/2024) giới hạn tổng số ngày giảm giá cho một loại hàng trong năm. "Miễn phí giao
  không thời hạn" là **chính sách phí giao** của bước 44 (miễn phí từ một giá trị đơn), không phải chương trình
  khuyến mại. `promotions_legal_limits` thêm `max_days_per_program?`, `max_days_per_year?` (dữ liệu, có ngày hiệu
  lực); `activate` kiểm.
- **Thông báo theo nơi nhận:** bảng `promotions_notices` (một dòng mỗi cơ quan nhận: tỉnh theo `address`, trạng
  thái, số tiếp nhận, ngày nộp) thay ba cột `notice_*`; `notice_status` của chương trình suy ra (mọi nơi đã nộp).
- **Kiểm trước khi bật:** `POST /promotions/{id}/activation-check` (không ghi) trả danh sách điều kiện, mỗi điều
  kiện `code`, `ok`, `params`; `activate` lỗi thì trả cùng danh sách trong `errors[]` của `Problem` — không lưu
  "lỗi lần bật cuối".
- **"Lưu mã" của website = lưu voucher (D-222):** chương trình muốn có nút "Lưu" là chương trình `voucher` có
  `claimable`; coupon công khai đã hiện sẵn ở bộ chọn mã của giỏ (D-201). Không làm `/me/saved-promotions`.
- **Nhắc khi flash sale bắt đầu:** người mua bấm "Nhắc tôi" (`promotions.use`) tạo một dòng
  `promotions_reminders`; job gửi qua `notifications` trước giờ mở `REMINDER_LEAD_MINUTES` (10). Lần bấm là đồng
  ý riêng cho đúng một lần nhắc đó (ghi bằng chứng đồng ý theo bước 37), không thành đồng ý nhận quảng cáo.

Căn cứ — **cần đối chiếu văn bản gốc / người có chuyên môn pháp lý**: Luật Thương mại 36/2005 Điều 97; NĐ 81/2018
sửa bởi NĐ 128/2024 (thời hạn giảm giá, nơi nhận thông báo, miễn phí giao có phải thông báo không); NĐ 239/2026;
Luật 91/2025 về đồng ý cho mục đích cụ thể.

## D-226 · Che số điện thoại, email của khách; cơ sở tính điểm thưởng — 2026-11-14 — agent quyết

Quyết định:
- Danh sách và chi tiết khách (`customers`, cả REST, MCP và xuất danh sách) trả số điện thoại, email **đã che**
  (`09•• ••• 412`, `n•••@gmail.com`). Xem đủ: `GET /customers/{id}/contact` với permission
  `customers.contact.reveal`; mỗi lần xem ghi audit (người xem, hồ sơ, lý do tuỳ chọn). Xuất danh sách có cột liên
  hệ đầy đủ cũng cần permission này. Người mua xem dữ liệu của chính mình không bị che.
- Điểm thưởng tính trên **số tiền người mua thực trả cho hàng hoá**: sau mọi khoản giảm (kể cả coupon, voucher),
  gồm thuế (giá bán lẻ gồm thuế), không gồm phí giao, không gồm phần trả bằng điểm hay thẻ quà tặng; trừ phần
  hoàn khi trả hàng. Cố định, ghi trong tài liệu; không là tuỳ chọn của khách.

Căn cứ: Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 (tối thiểu hoá, kiểm soát truy cập) — **cần đối chiếu văn bản
gốc**. Lý do điểm: một định nghĩa duy nhất thì người mua, nhân viên và sổ cái cùng số.

## D-227 · Chênh lệch tiền két khi đóng ca — 2026-11-14 — agent quyết

Quyết định: đóng ca (bước 45) tính `variance = counted − expected`; khác 0 thì bắt buộc `variance_reason` (danh
sách lý do `orders.cash_variance`, D-223) và ghi chú. Chênh lệch ghi sổ cái ngay khi đóng ca vào tài khoản **chờ
xử lý** của cửa hàng: thiếu → `suspense.cash_shortage`, thừa → `suspense.cash_overage` (ánh xạ sang hệ thống tài
khoản kế toán theo bước 36 — tài sản thiếu chờ xử lý / tài sản thừa chờ giải quyết); người quản lý xử lý sau bằng
bút toán từ sổ cái. Vượt ngưỡng `cash_variance_approval_minor` (cài đặt kế thừa) thì đóng ca cần người duyệt
(P12 bước 51).

Căn cứ: chế độ kế toán doanh nghiệp (TT 200/2014, TT 133/2016 — tài khoản 1381 / 3381) — **cần đối chiếu**.
Lý do: tiền két lệch là sự thật kế toán, phải vào sổ; quyết nguyên nhân là việc của người, không để phần mềm đoán.

## D-228 · Lộ trình xếp theo thứ tự làm; bước đã xong tách file; đặc tả một phiên bản — 2026-11-14 — chủ dự án quyết

Quyết định: chủ dự án đồng ý bốn việc dọn sau khi rà bước 31–66:
1. **Số bước là mã cố định, không phải thứ tự.** `docs/ROADMAP.md` xếp các bước theo thứ tự làm, chia giai
   đoạn: A hợp đồng và nền lõi (31, 32, 33, 34, 35–37, 40) → B lớp cho giao diện và phân quyền (51, 54, 52) →
   C vận hành (38, 39) → D module nền dùng chung (53, 57, 58, 66, 65) → E thương mại (41–49, 55, 56, 59–64);
   bước 50 ở mục "Chờ chủ dự án". Bảng phụ thuộc của D-192 giữ nguyên (bỏ dòng 30). Bước kế tiếp là bước đầu
   tiên từ trên xuống còn mục `[ ]` mà phụ thuộc đã xong — skill `next-step` sửa theo.
2. **Bước 0–30 đã xong** chuyển nguyên văn sang `docs/roadmap/done.md`; lộ trình chỉ còn việc đang mở.
3. **Một đặc tả, một phiên bản đúng.** Các mục "Bổ sung theo …" nối ở cuối đặc tả được gộp vào thân (bảng, API,
   test… đúng chỗ), câu bị thay thì xoá, ghi số quyết định trong ngoặc. Làm ngay cho đặc tả có từ ba mục bổ sung
   (34, 37, 41–45, 47, 48, 51, 52); đặc tả còn lại gộp ở commit đầu tiên của bước đó. Từ nay không nối mục
   "Bổ sung" mới — luật ghi ở `docs/steps/README.md`.
4. **Danh sách việc của mỗi bước gom theo chủ đề**, không theo nguồn ("Đối chiếu thiết kế 2", "D-202"…); nguồn
   vẫn nằm trong quyết định và đặc tả.

Thay: câu "thứ tự và phụ thuộc ở bảng ngay trước bước 30" (D-192) — bảng nay ở đầu lộ trình; cách chọn bước
kế tiếp "số nhỏ nhất còn mục chưa xong" của skill `next-step`.

Lý do: số bước đã không còn khớp thứ tự làm (51 phải trước 41; 53, 57, 58, 65, 66 thuộc phần dùng chung), đặc tả
bị vá chồng tới bảy lớp với chỗ lớp sau đảo lớp trước, và gần 30% lộ trình là việc đã xong mà mỗi phiên agent
vẫn đọc lại.

Đã cân nhắc: đánh số lại các bước — số bước được nhắc khoảng 110 lần trong nhật ký quyết định, nhật ký chỉ thêm
không sửa, đổi số thì quyết định cũ trỏ sai; gộp các bước nhỏ 59–66 — mỗi bước là một module, gộp thì ranh giới mờ.

## D-229 · Trang dựng từ khối và AI tạo trang (bước 67); tính năng AI hỗ trợ người dùng trong CMS — 2026-11-14 — chủ dự án quyết

Quyết định: chủ dự án đồng ý thêm bước 67 và các tính năng AI / tự động hoá đi kèm, sau khi đối chiếu cách làm
của Builder.io (khối đăng ký sẵn, trang là JSON, AI chỉ dùng khối đã đăng ký), Shopify Horizon (trang ghép từ
theme blocks), Wix Harmony (AI dựng rồi người sửa kéo thả) và Webflow App Gen / Base44 (AI sinh code rồi deploy).

- **Một nền cho hai cách dùng.** Trang của `content` có thân `layout` (cây khối JSON). Tạo bằng trò chuyện với AI
  và kéo thả cùng sửa bản dữ liệu đó. Giao diện kéo thả ở repo giao diện (D-026, D-097); core có sổ khối, kiểm
  bố cục, nháp / lịch sử / xem trước / hẹn giờ, khoá sửa, đọc công khai kèm dữ liệu khối.
- **AI chỉ ghép khối, không sinh code.** Kết quả AI là bố cục hay JSON Patch qua cùng bộ kiểm từ chối mặc định
  (như D-102), thành bản nháp `ai_assisted` (D-109). Không có khối "mã tuỳ chỉnh" ở bước này.
- **Module tự khai khối** qua `Module::blocks()` (mặc định rỗng — D-038) và trả dữ liệu qua `BlockData` theo lô
  (không N+1). Chức năng thật (form, đăng ký nhận tin, giỏ hàng, lưu voucher) là khối của module sở hữu.
- **Không deploy:** đăng là công khai ngay ở tên miền của website + đường dẫn trang; chân trang pháp lý tự gắn.
- **Đi kèm:** bộ giao diện chung của website (token, kế thừa), khu dùng chung, trang mẫu, kiểm tra sức khoẻ website
  (SEO, alt, link gãy, 404 → gợi ý chuyển hướng), bộ nháp chiến dịch từ một chương trình khuyến mại.
- **Thêm vào bước có sẵn:** tạo sản phẩm nháp từ ảnh (41); cảnh báo bất thường bằng thống kê đơn giản, tất định
  (58). Hỏi số liệu bằng câu thường đã có ở trợ lý của bước 52 (tool chỉ đọc với quyền người hỏi), không thêm.

Thay: câu "trình dựng trang kéo thả — trái D-026" trong phần "Đã cân nhắc" của D-219 — chỉ phần kéo thả là giao
diện (ở repo riêng); dữ liệu trang là API nên không trái D-026.

Để sau: thử A/B trang (sau bước 57); AI tạo ảnh (cần loại nhà cung cấp AI cho ảnh); khối mã tuỳ chỉnh (cần quyết
định riêng: tắt mặc định, HTML / CSS đã lọc, iframe `sandbox`, CSP chặt).

Lý do: người không biết code làm được trang đích ngay, an toàn (AI không đưa được mã vào trang, không làm lộ dữ
liệu người mua), nhẹ (trang là dữ liệu, không build, không deploy), và agent bên ngoài cũng dựng trang qua MCP.

Đã cân nhắc: AI sinh code như Webflow App Gen / Base44 — mạnh nhưng phải deploy, khó kiểm, rủi ro bảo mật, nặng
cho mục tiêu "hỗ trợ nhẹ"; module `pages` riêng — lặp nháp / lịch sử / đường dẫn / xem trước mà `content` đã có.

## D-230 · AI tự trả lời câu hỏi đơn giản đã có câu trả lời chính xác; câu phức tạp chỉ soạn nháp — 2026-11-14 — chủ dự án quyết

Quyết định: chủ dự án chọn — **câu hỏi phức tạp hay mơ hồ: AI chỉ soạn nháp cho nhân viên; câu hỏi đơn giản, đã
biết chính xác, không mơ hồ: tự động trả lời.** Đây là ngoại lệ có giới hạn của câu "kết quả AI không bao giờ tự
đăng" trong D-109, cho kênh hỗ trợ (`support`, bước 53) và chat (`conversations`, bước 61):

- **Chữ gửi tự động không do mô hình viết.** Chỉ hai nguồn: câu trả lời mẫu được người có quyền **duyệt cho tự
  động** (kèm ngày phải rà lại), gửi nguyên văn; hoặc mẫu câu i18n của `FactSource` dựng từ dữ liệu chính xác
  (trạng thái đơn, hành trình vận đơn, giờ mở cửa) — chỉ khi người hỏi chứng minh được là chủ dữ liệu. Mô hình chỉ
  **phân loại** câu hỏi và chọn nguồn.
- **Quyết định trong Rust, không tin mô hình:** gửi khi độ tin cậy ≥ ngưỡng của nút (mặc định 90%), không có cờ rủi
  ro, chưa quá số lần tự động liên tiếp (mặc định 2). Không bao giờ tự động cho khiếu nại, hoàn tiền / trả hàng /
  huỷ đơn, thanh toán lỗi, dữ liệu cá nhân, pháp lý, lời lẽ bức xúc, người mua đòi gặp người; từ khoá chặn theo nút
  là dữ liệu.
- **Tắt mặc định**, khách bật theo nút; tin tự động gắn nhãn "Trả lời tự động" và có nút "Gặp nhân viên"; người
  mua báo "không đúng" đủ ngưỡng thì câu trả lời tự thôi dùng; mọi lần tự động ghi nhật ký.
- Một bộ quyết duy nhất: `AutoResponder` của `support`; `conversations` gọi nó, không làm bộ riêng.

Căn cứ — **cần đối chiếu văn bản gốc / người có chuyên môn pháp lý**: Luật Trí tuệ nhân tạo 134/2025/QH15 và văn
bản hướng dẫn (cho người dùng biết đang tương tác với hệ thống tự động); Luật 19/2023/QH15 (thông tin cho người
tiêu dùng chính xác); Luật 91/2025/QH15 (không trả dữ liệu cho người chưa chứng minh là chủ).

Lý do: phần lớn câu hỏi lặp lại ("đơn tôi đâu", "mấy giờ mở cửa", "đổi trả mấy ngày") có câu trả lời chắc chắn;
trả ngay giúp người mua và nhân viên. Giới hạn nguồn câu trả lời là cách giữ "chính xác, không mơ hồ" mà không
phải tin mô hình tự viết đúng.

Đã cân nhắc: để mô hình tự viết câu trả lời rồi gửi khi "tự tin" — mô hình có thể bịa (chính sách, ngày giao) mà
vẫn tự tin; chỉ soạn nháp cho mọi câu — an toàn nhưng bỏ lợi ích chính chủ dự án muốn.

## D-231 · Thử A/B trang, AI tạo ảnh, khối mã tự viết (bước 67) — 2026-11-14 — chủ dự án quyết

Quyết định của chủ dự án cho ba việc D-229 để sau:

1. **Thử A/B hai phiên bản trang: làm** (sau bước 57). Chia **cố định theo người xem**, không ngẫu nhiên theo
   lần tải: `hash(thí nghiệm, mã người xem) mod 10000` so với tỉ lệ — cùng người F5 vẫn thấy cùng phiên bản. Mã
   người xem: đã đăng nhập → tài khoản (mọi thiết bị, mọi ngày); chưa đăng nhập → khoá phiên ẩn danh trong ngày
   của bước 57 (không cookie: hôm sau hay đổi mạng có thể đổi phiên bản — tài liệu ghi rõ). Bot luôn thấy bản gốc,
   cùng URL. Chỉ thử bố cục và nội dung, **không thử giá / khuyến mại**. Người chọn bản thắng; hệ thống chỉ báo số
   liệu và độ tin cậy.
2. **AI tạo ảnh: làm theo khuyến nghị.** Trait `ImageModel` ở kernel, adapter ở module `ai`, mặc định tắt. Chỉ cho
   banner, ảnh nền, ảnh minh hoạ; ảnh mang `ai_generated` và nhãn; **không bao giờ làm ảnh sản phẩm** (`catalog`
   chặn); không tạo ảnh người thật nhận ra được hay thương hiệu của người khác.
3. **Khối mã tự viết: chọn C, D bật được nếu muốn.** Ba mức `off` (mặc định) / `sandboxed` (C: HTML + CSS +
   JavaScript trong iframe `sandbox` không `allow-same-origin`, CSP theo danh sách host — chủ cửa hàng bật) /
   `unrestricted` (D: chèn thẳng vào trang — cần **cả** người cài máy chủ bật `AUTOXIDE_CONTENT_UNSANDBOXED_CODE`
   **và** chủ cửa hàng bật có xác nhận lại danh tính; không bao giờ trên trang thanh toán, giỏ, tài khoản, đăng
   nhập; người bán trên sàn không có mức D). Mọi mức: audit mỗi lần sửa, sổ script để rà, AI không bao giờ tạo hay
   sửa khối này, agent sửa thì chờ duyệt.

Căn cứ — **cần đối chiếu văn bản gốc / người có chuyên môn pháp lý**: Luật 91/2025/QH15 và NĐ 356/2025 (chia người
xem không cookie); Luật Trí tuệ nhân tạo 134/2025/QH15 (nhãn ảnh AI); Luật 19/2023/QH15 (ảnh sản phẩm, giá trung
thực); luật an ninh mạng hiện hành; PCI DSS 4.0 yêu cầu 6.4.3 / 11.6.1 (chuẩn ngành về script trên trang thanh toán).

Thay: dòng "để sau" về ba việc này trong D-229.

Lý do: A/B cố định theo người là cách các công cụ lớn làm (người xem thấy trang ổn định, số liệu đúng); ảnh AI có
ích cho banner nhưng gây hiểu sai nếu làm ảnh sản phẩm; mã tự viết cần cho tích hợp mà khối nhúng chưa có, mức C
giữ an toàn cho người mua, mức D cho người chấp nhận rủi ro nhưng phải bật hai lớp và tránh trang thanh toán.

Đã cân nhắc: chia A/B ngẫu nhiên theo lần tải — trang nhảy khi F5, số liệu sai; đặt cookie A/B cho khách chưa đồng ý
— trái hướng không cookie của bước 57; cho mức D mặc định — rủi ro lộ dữ liệu người mua, rơi vào mọi khách dùng
Autoxide.

## D-232 · Đưa 11 mục "để sau / chờ quyết" vào các bước có sẵn — 2026-11-14 — chủ dự án trao quyền, agent quyết

Chủ dự án yêu cầu viết đặc tả cho 11 mục ở phần "Sau core" của lộ trình và **trao quyền quyết định cho agent**, kể
cả ba mục trước đây "chờ chủ dự án quyết". Agent quyết: **không mở bước mới**; mỗi mục gắn vào bước cùng chủ đề,
đặt cuối danh sách của bước đó (làm sau phần chính của bước), để giai đoạn A không bị đẩy lùi.

| # | Mục | Bước | Quyết định chính |
|---|---|---|---|
| 1 | Khách thử nghiệm (sandbox) | 38 | Sao cấu hình, không sao dữ liệu cá nhân; mọi đối tác buộc dùng bộ giả; mail chỉ tới nhân viên đã xác minh; `noindex`; tự hết hạn; đẩy sang khách thật thành **bản nháp** |
| 2 | Khách duyệt từng lần khi nhân sự nền tảng xin vào | 54 | `support_access_mode` `disabled` / `per_request` (**mặc định**) / `standing`; duyệt qua P12; tối đa 4 giờ; `legal_request` như D-211 |
| 3 | Nhóm nút ngoài cây cho báo cáo | 38 | Nhóm tự do trong `organizations`; **nhóm không cấp quyền** — báo cáo là giao của nhóm với phạm vi người xem |
| 4 | Quản trị trên điện thoại | 52 | Gợi ý `mobile` trên sổ điều hướng (tối đa 5 mục thanh đáy); tóm tắt cuối ngày tự bật, theo giờ của người dùng |
| 5 | Bán ứng dụng trả phí, chia doanh thu | 65 | Miễn phí hay thuê bao tháng; nền tảng là bên bán, thu qua hoá đơn thuê bao (35), chi nhà phát triển qua `PayoutGateway` (36); nhà phát triển nước ngoài chờ đối chiếu thuế |
| 6 | Trả trước một phần cho hàng đặt trước | 45 | Gọi là "trả trước một phần", **không** mặc định là "đặt cọc" theo BLDS 2015 Điều 328; người bán huỷ hoàn đủ; người mua huỷ theo điều kiện công bố trước |
| 7 | Đấu thầu và hợp đồng khung B2B | 59 | Hợp đồng khung có cam kết trừ nguyên tử; mời thầu nhiều người bán chỉ trên sàn, giá niêm phong tới hạn; không dùng cho mua sắm công |
| 8 | Gọi điện / tổng đài trong chat | 61 | Trait `VoiceProvider` + bộ giả; yêu cầu gọi lại vào hộp việc; ghi âm tắt mặc định, phải thông báo trước |
| 9 | Đăng nhập thay người dùng | 54 | Thay bằng **"xem như" chỉ đọc**, không bao giờ làm thay; người mua phải cấp phép (mã 30 phút); không xem người quyền cao hơn; audit hai id |
| 10 | Ảnh AVIF | 39 | **Đo trước**: chỉ thêm khi nhẹ hơn WebP ≥ 20% ở chất lượng tương đương; sinh bằng job, tắt mặc định; không đạt thì ghi "không làm" kèm số đo |
| 11 | Ký hợp đồng điện tử | 59 (trong `compliance`) | Trait `SignatureProvider` + bộ giả; ký `simple` (OTP + đồng ý, lưu bằng chứng) hay `digital` (chữ ký số qua nhà cung cấp được cấp phép); core không dựng PDF; mã băm chống sửa tệp |

Bộ nối đối tác thật (tổng đài, chữ ký số) vẫn theo luật cũ: cần tài khoản thử và secret thật thì dừng hỏi chủ dự án.

Căn cứ — **cần đối chiếu văn bản gốc / người có chuyên môn pháp lý**: Luật 91/2025/QH15, NĐ 356/2025 (sandbox, xem
như, ghi âm, truy cập hỗ trợ); Bộ luật Dân sự 2015 Điều 328 (đặt cọc); NĐ 123/2020 sửa bởi NĐ 70/2025 (hoá đơn khoản
thu trước); Luật Đấu thầu 22/2023/QH15 (phạm vi mua sắm công); Luật Giao dịch điện tử 20/2023/QH15 và nghị định về
chữ ký điện tử, dịch vụ tin cậy (ký hợp đồng); quy định thuế nhà thầu (chi cho nhà phát triển nước ngoài).

Thay: các dòng "Để sau (D-201…)", "Để sau: bán qua ứng dụng trả phí…" và "Chờ chủ dự án quyết: đăng nhập thay người
dùng · AVIF" ở mục "Sau core" của lộ trình; câu "Ký hợp đồng điện tử vẫn chờ chủ dự án quyết hướng" của D-221.

Lý do: cả 11 mục là tính năng các nền tảng lớn đều có; gắn vào bước cùng chủ đề thì dùng lại hạ tầng sẵn có (phân
phối cha → con, yêu cầu duyệt, sổ điều hướng, `PayoutGateway`, luồng đặt trước, báo giá B2B), không thêm module.

Đã cân nhắc: mở bước 68+ cho từng mục — thêm bước mà không thêm giá trị, phần lõi chờ lâu hơn; "đăng nhập thay người
dùng" có quyền ghi như một số nền tảng — rủi ro lạm quyền và dữ liệu cá nhân, "xem như chỉ đọc" đủ cho việc gỡ lỗi;
module `contracts` riêng cho ký điện tử — `compliance` đã sở hữu văn bản pháp lý có phiên bản.

## D-233 · Viết đặc tả cho 6 việc "để sau" còn rải trong đặc tả; dọn câu "để sau" đã cũ — 2026-11-14 — chủ dự án trao quyền, agent quyết

Chủ dự án yêu cầu viết đặc tả luôn cho các việc còn ghi "để sau" trong đặc tả (ngoài 11 mục của D-232), quyền quyết
giao cho agent. Như D-232: không mở bước mới, gắn vào bước cùng chủ đề, cuối danh sách của bước.

| # | Việc | Bước | Quyết định chính |
|---|---|---|---|
| 1 | Hợp đồng tool MCP | 31 (A6) | `docs/mcp-tools.json` sinh từ code; `api-diff` chặn thay đổi phá vỡ của tool như của endpoint; tool cũ khai deprecated (mở rộng D-059 cho tool) |
| 2 | `cargo semver-checks` | 31 (A7) | Chạy trong quy trình phát hành cho kernel, testkit; **chỉ báo cáo** khi còn `0.x`, chặn từ `1.0` hay khi công bố framework — giữ lý do của D-167, không đưa vào gate |
| 3 | Báo khi sản phẩm yêu thích có hàng lại / giảm giá | 47 | Người mua tự bật, đồng ý theo mục đích riêng; chỉ báo giảm thật ≥ 5% so với giá mốc; tối đa 1 lần / 7 ngày mỗi sản phẩm; gom lô theo khung giờ |
| 4 | Banner trong gian hàng người bán | 55 | Người bán tự quản trong nút của mình; sàn duyệt theo chính sách dữ liệu; link chỉ tới hàng của chính người bán; không phải quảng cáo trong sàn (64) |
| 5 | Người bán nước ngoài trên sàn | 63 | Tắt mặc định; điều kiện (giấy tờ, đại diện, thuế) là gói luật có ngày hiệu lực, chưa có gói thì không bật được; công bố quốc gia người bán; chi trả ngoại tệ; hàng nhập khẩu cho người mua ngoài phạm vi |
| 6 | EDI cho B2B | 59 | Chuẩn mở UBL 2.1 (OASIS); vào qua API key `service`, ra qua `webhooks`; EDIFACT / X12 là bộ chuyển đổi riêng khi có khách cần |

Dọn câu "để sau" đã cũ (việc đã có bước riêng): sản phẩm bán theo kỳ (62), bán sỉ B2B (59), chat thời gian thực
(61), ứng dụng OAuth (65), thử A/B (67), quảng cáo trong sàn (64) — trong đặc tả 35, 45, 46, 47, 53, 54, 57 và
ROADMAP.

Căn cứ — **cần đối chiếu văn bản gốc / người có chuyên môn pháp lý**: Luật 91/2025/QH15 và luật tin quảng cáo của
bước 37 (báo yêu thích); Luật 19/2023/QH15 (không báo giảm giá ảo; công bố người bán nước ngoài); Luật Thương mại
điện tử 122/2025, NĐ 85/2021, NĐ 117/2025, Thông tư 80/2021/TT-BTC (người bán nước ngoài, thuế).

Lý do: chủ dự án muốn phủ hết việc đã biết; gắn vào bước có sẵn dùng lại hạ tầng (api-diff, sổ đồng ý, banner,
webhooks, KYC, đa tiền tệ) và không làm giai đoạn A dài thêm.

Đã cân nhắc: đưa `cargo semver-checks` vào gate ngay — vẫn như D-167, không có ích khi còn `0.x`; EDIFACT / X12 trong
core — nặng, ít khách Việt Nam dùng, để thành bộ chuyển đổi riêng; cho người bán nước ngoài gửi hàng nhập khẩu thẳng
tới người mua — kéo theo hải quan, thuế nhập khẩu, cần đối chiếu riêng.

Ghi thêm cho D-233 (lúc gộp kết quả): EDI của bước 59 nhận và trả **thân XML (UBL)** ở `/api/v1/wholesale/edi/*` —
ngoại lệ có chủ đích của luật "ba hình dạng response" (`Data` / `Page` / `Problem`), cùng loại với tải tệp (hoá đơn
điện tử XML của 36, nguồn cấp sản phẩm của 48): chuẩn của chứng từ là XML, không bọc trong envelope JSON; lỗi giao
thức (xác thực, quá cỡ, sai kiểu nội dung) vẫn trả `Problem`.

## D-234 · Chủ đề theo mùa / ngày lễ / khung giờ và đầu trang, chân trang của website — 2026-11-14 — chủ dự án đồng ý đề xuất, agent quyết chi tiết

Chủ dự án hỏi CMS đã đổi được màu chủ đạo, logo, thanh, đầu trang, chân trang và chủ đề theo mùa, ngày lễ, giờ đặc
biệt chưa — "trong giới hạn cho phép của hệ thống", và không được làm mất sự nhanh, mượt. Đặc tả 67 (D-229) đã có
theme gốc, khu dùng chung, thanh thông báo; 55 có thông tin cửa hàng. Còn thiếu hai thứ, thêm vào **bước 67** (không
mở bước mới):

1. **Bản chủ đề có lịch** (`content_theme_variants`): ghi đè một phần theme gốc (token, font trong danh sách, logo,
   ảnh nền), hiệu ứng trang trí tuỳ chọn, đầu / chân trang riêng tuỳ chọn; lịch `once` (UTC) / `daily` / `weekly`
   (giờ địa phương theo `organizations.timezone` của website, công cụ múi giờ IANA của bước 34). Trùng giờ: ưu
   tiên cao hơn → bắt đầu muộn hơn → `id`; chỉ một bản áp tại một lúc. Thư viện chủ đề mẫu nhúng trong binary (lễ
   âm lịch kèm ngày gợi ý theo năm là dữ liệu); chép sang năm sau; xem trước ở một thời điểm.
2. **Đầu trang và chân trang của website** (`content_site_regions`): bố cục khối, nháp / đăng, theo ngôn ngữ, kế thừa
   theo nút; chân trang pháp lý luôn gắn dưới, không nằm trong bố cục. Khối mới: `content.logo`, `content.menu`,
   `content.search_box`; `orders.cart_link` (bước 45); `organizations.store_info` (bước 55).
3. **`GET /api/v1/content/public/site`** — một lời gọi cho khung website, thay `GET /public/theme` (chưa code): cache
   trong bộ nhớ tới mốc đổi chủ đề kế tiếp (trần 30 giây cho tiến trình khác), `Cache-Control: max-age` cắt đúng tại
   `next_change_at` (trần 300), ETag theo phiên bản hiệu lực; token phẳng để giao diện gán thành biến CSS. Đo p99
   khi cache nóng / nguội trên cả hai database.

Giới hạn của hệ thống: chủ đề chỉ chứa token, ID `media`, tên hiệu ứng — không CSS, HTML, script; hiệu ứng chỉ trong
danh sách core khai, giao diện vẽ, tải sau, tôn trọng `prefers-reduced-motion`, không chạy ở giỏ hàng / thanh toán /
tài khoản / đăng nhập; số bản chủ đề theo hạn mức gói `content.theme_variants` và trần máy chủ
`AUTOXIDE_CONTENT_THEME_VARIANTS_MAX`; logo thương hiệu đặt một chỗ (`storefront.logo_media_id` khi bước 55 có),
theme chỉ ghi đè.

Lý do: khách đổi giao diện theo dịp mà không cần người làm giao diện; mọi thứ là dữ liệu đã kiểm nên trang vẫn đúng
thương hiệu và an toàn; giá trị chọn sẵn ở máy chủ và cache tới đúng mốc đổi nên đường đọc công khai không thêm query.

Đã cân nhắc: chồng nhiều bản chủ đề cùng lúc — kết quả khó đoán, khó cache; cho khách viết CSS tự do — phá thương hiệu,
mở đường chèn mã, đã có khối mã tự viết cách ly (D-231) cho nhu cầu đặc biệt; để giao diện tự tính lịch — mỗi giao
diện tính một kiểu, lệch múi giờ, cache CDN không biết lúc nào hết hạn; bảng riêng cho chủ đề mẫu — dữ liệu đi theo
bản phát hành, nhúng binary đủ; lịch theo âm lịch tự tính — thêm thư viện cho vài ngày mỗi năm, ngày cụ thể đủ.

## D-235 · Khách tự chỉnh không cần code: trường tự định nghĩa, nhãn, đổi câu chữ, loại nội dung, mã theo dõi và đồng ý cookie — 2026-11-14 — chủ dự án đồng ý đề xuất, agent quyết chi tiết

Chủ dự án hỏi còn gì hỗ trợ "động" cho khách và người dùng CMS. Rà đặc tả thấy đã có: gói và tính năng (35), cây tổ
chức và cài đặt kế thừa (34), tên miền, ngôn ngữ (33, 34), role tự tạo, menu, form, webhook (đã code), giao diện và
chủ đề theo lịch (67, D-234), mẫu mail (37), số chứng từ (36), thuộc tính sản phẩm (41), bộ lọc đã lưu và thao tác
hàng loạt (51), tự động hoá (58). Còn thiếu năm thứ, thêm vào bước có sẵn (không mở bước mới):

| # | Việc | Bước | Quyết định chính |
|---|---|---|---|
| 1 | Trường tự định nghĩa trên bản ghi | 51 (P19) | Kernel giữ định nghĩa và bộ kiểm; module chọn tham gia bằng `Module::custom_field_targets()`; giá trị là cột JSON trên bản ghi của module (không nối bảng khi đọc); chỉ trường `filterable` vào bảng `<module>_custom_index` ghi cùng transaction, lọc bằng subquery thường — không toán tử JSON riêng của database; mức hiện `internal` / `public` / `buyer_input`; không nhận regex của khách |
| 2 | Loại nội dung tự định nghĩa | 67 | `content_types` + mục `kind = custom`; trường là P19 (không có bộ trường thứ hai); dùng lại nháp, lịch sử, bản dịch, SEO; trang chi tiết dựng sẵn, khối `content.type_list` |
| 3 | Mã theo dõi quảng cáo + đồng ý cookie | 37 (A6) | Nhà cung cấp cho sẵn, khách chỉ điền mã ID (không dán script); không tải gì trước khi đồng ý đúng nhóm; từ chối dễ như đồng ý; bằng chứng đồng ý ẩn danh; nhà cung cấp tự vào danh sách bên nhận dữ liệu; đi kèm `/public/site` qua điểm cắm `SiteExtensions` |
| 4 | Nhãn gắn bản ghi | 51 (P20) | Bảng của kernel (lọc cần subquery, như `kernel_org_closure`); loại bản ghi khai `taggable`; gắn cần quyền ghi bản ghi; nhãn nội bộ, không công khai. Thẻ công khai của `content` và nhãn của `media` (D-094) giữ nguyên |
| 5 | Đổi câu chữ hiển thị theo khách | 51 (P21) | Chỉ key module khai đổi được và không gian `ui.*` cho giao diện; giữ đúng chỗ điền; mã lỗi và câu `legal` không đổi được; đọc qua cache bộ nhớ theo khách |

Kèm: `orders.order` hỗ trợ `buyer_input` để khách thêm ô người mua nhập lúc đặt hàng (45). Thêm vào bảng "API mới
của kernel" ở `docs/steps/README.md`.

Căn cứ của mục 3 — **cần đối chiếu văn bản gốc / người có chuyên môn pháp lý**: Luật BVDLCN 91/2025/QH15, NĐ
356/2025/NĐ-CP (đồng ý, chuyển dữ liệu ra nước ngoài), Luật BVQLNTD 19/2023/QH15. Chưa chắc: cookie đo lường có cần
đồng ý hay dùng căn cứ khác; hồ sơ đánh giá tác động khi bật pixel của nền tảng nước ngoài. Thiết kế chọn mức chặt
(hỏi trước) — không chặn công việc.

Lý do: mọi nền tảng lớn đều cho khách tự làm năm việc này; đặt ở kernel phần dùng chung (định nghĩa trường, nhãn,
câu chữ) thì mỗi module không tự làm một kiểu; giữ tốc độ bằng giá trị nằm trên bản ghi, index riêng chỉ cho trường
lọc, cache theo khách cho câu chữ và định nghĩa.

Đã cân nhắc: kiểu "entity–attribute–value" (mỗi giá trị một dòng) cho trường tự định nghĩa — đọc bản ghi phải nối và
gom nhiều dòng, chậm; cột JSON có index của database — Postgres có, SQLite khác hẳn, vi phạm luật số một; cho khách
dán script theo dõi — đã có khối mã tự viết cách ly (D-231) cho nhu cầu đặc biệt, còn mã theo dõi phổ biến cần gắn
với đồng ý nên phải có cấu trúc; chuyển nhãn của `media` sang P20 — phá hợp đồng API đã phát hành, không đem lại gì.

## D-236 · Kiểu vị trí banner, bộ vị trí chuẩn, chọn trang hiện, banner dạng chữ, khối vùng banner — 2026-11-14 — chủ dự án đồng ý đề xuất, agent quyết chi tiết

Chủ dự án muốn banner có loại theo vị trí (trên, trái, phải, chính…). Đặc tả 55 đã có vị trí là dữ liệu khách tự
đặt tên, nhưng vị trí chỉ có tên và số banner tối đa — giao diện phải tự đoán cách vẽ, người soạn không biết ảnh
cỡ nào. Thêm vào **bước 55** (không mở bước mới):

1. **Kiểu vị trí:** `display` (`single` / `carousel` / `grid` / `stack` / `strip` / `popup`) với cài đặt có schema
   (trượt tự động ≥ 4 giây hay tắt; lưới 2–4 cột; popup có tần suất, độ trễ); `accepts` (`image` / `text` / `any`);
   tỉ lệ và cỡ ảnh tối thiểu (máy tính, điện thoại) kiểm khi lưu bằng kích thước `media` đã lưu, sai lệch tỉ lệ
   ≤ 2%; `devices`; `page_kinds` (loại trang vị trí dùng được).
2. **Bộ vị trí chuẩn** gieo cho khách mới qua `Module::seed_tenant`: `top_strip`, `home_hero`, `home_middle`,
   `category_top`, `sidebar_left`, `sidebar_right`, `inline`, `above_footer`, `popup` — khách sửa, tắt, xoá được;
   không gieo lại.
3. **Chọn trang hiện:** banner có `page_kinds` (tập con của vị trí) và `page_targets` (`{kind, id}` theo sổ đối
   tượng, tối đa 50); `catalog` đăng ký thêm `catalog.category`, `catalog.collection`. Lọc trong bộ nhớ trên tối đa
   50 banner đang hiệu lực của vị trí — câu truy vấn và index không đổi.
4. **Banner dạng chữ:** `kind = text`, màu là tên token của theme (không mã màu tự do).
5. **Đọc nhiều vị trí một lần:** `GET /api/v1/content/public/placements?keys=` (tối đa 10), `max-age` là mốc gần nhất.
6. **Khối `content.placement`** cho trang dựng từ khối và đầu trang (D-234), khai ở `content` tại bước 55 khi sổ khối
   của bước 67 đã có; một lần đọc cho mọi vùng banner của trang.

Thứ tự banner cố định theo `position` khách sắp, không đảo ngẫu nhiên. Khối `announcement_bar` / `popup` của bước 67
giữ nguyên vai trò: chữ cố định trong một trang (popup đó để thu email); vị trí banner là nội dung có lịch, quản lý
một chỗ.

Lý do: giao diện biết vẽ gì từ dữ liệu, người soạn biết ảnh cỡ nào, đổi banner không làm vỡ trang; khách mới có ngay
bộ vị trí dùng được; giữ tốc độ vì một lời gọi cho cả trang và cache tới đúng mốc banner đổi.

Đã cân nhắc: danh sách vị trí cố định trong code (`left`, `right`…) — mỗi giao diện bố cục một kiểu, cố định là bó
khách; xoay vòng ngẫu nhiên hay theo trọng số — phá cache, F5 nhảy, cần thì làm bằng thử A/B của bước 67; banner
chữ có màu tự do — phá thương hiệu, lệch theme; ghép vị trí banner vào khối `announcement_bar` — hai việc khác
nhau (cố định so với có lịch), gộp làm mỗi khối nửa vời.

## D-237 · Rà lộ trình lần hai trước bước 31: tỉ giá về `ledger`, một chỗ cho thông tin cửa hàng, nhãn và ghi chú của `customers`, báo có hàng lại không trùng, sửa bảng phụ thuộc — 2026-11-14 — agent quyết

Chủ dự án yêu cầu rà toàn bộ lộ trình một lượt trước khi code bước 31. Kiểm máy: mọi đường dẫn đặc tả tồn tại, mọi
mã D-… có trong nhật ký, đủ 37 bước 31–67 trong bảng thứ tự. Đọc từng bước và đối chiếu đặc tả thì thấy năm chỗ
mâu thuẫn hay làm một việc hai nơi; sửa thẳng vào thân đặc tả (một đặc tả, một phiên bản — D-228):

1. **Tỉ giá thuộc `ledger`** (thay chỗ để ngỏ ở D-233 / đặc tả 63). Đặc tả 36 đặt bảng tỉ giá ở `invoices` nhưng
   `ledger` (đăng ký **trước** `invoices`) lại gọi `dyn Invoices::exchange_rate` khi ghi bút toán ngoại tệ —
   không làm được theo luật thứ tự đăng ký. Đặc tả 63 thì đặt bảng khác (`ledger_fx_rates`, dạng phân số). Chốt:
   bảng `ledger_fx_rates` (số thập phân chuẩn hoá dạng chữ như mọi số thập phân của 36), `Ledger::exchange_rate`,
   endpoint `/api/v1/ledger/exchange-rates`, lịch `ledger.fetch_exchange_rates`; `invoices`, báo cáo gộp (38),
   chi trả ngoại tệ và đa tiền tệ (63) đọc qua đó.
2. **Thông tin cửa hàng một chỗ.** Hồ sơ công khai của nút (34) và `storefront.*` (55) cùng giữ tên, logo, địa chỉ,
   giờ mở cửa. Chốt: **địa điểm** (tên hiển thị, `Address`, giờ mở cửa, ngày đặc biệt, nhận tại quầy, ảnh bìa) chỉ
   ở hồ sơ nút của 34; **thương hiệu** (tên thương hiệu, logo, biểu tượng, liên hệ, mạng xã hội) chỉ ở cài đặt kế
   thừa `storefront.*` của 55; `/public/storefront` ghép hai phần. Bỏ `logo_media_id` khỏi hồ sơ nút.
3. **`customers` dùng chỗ chung của bước 51.** Đặc tả 47 còn cột `tags` và bảng `customers_notes` riêng, trái với
   P13 (ghi chú chung) và P20 (nhãn chung, D-235). Chốt: `customers.customer` khai `taggable`, đích trường tự định
   nghĩa; ghi chú qua `kernel_record_notes`.
4. **Báo có hàng lại không gửi trùng.** Đăng ký báo có hàng lại của `inventory` (42) và báo yêu thích của
   `customers` (47) cùng nghe một event. Chốt: `inventory_restock_alerts` ghi `user_id` khi người đăng ký đã đăng
   nhập; `Inventory::restock_alerts_of` cho `customers` bỏ những biến thể người đó đã tự đăng ký — một lần có hàng,
   một tin.
5. **Bảng phụ thuộc.** 38 ghi "song song với 39" trong khi 39 bắt đầu sau 38 (39 rà index phủ cả bảng của 33–38) —
   bỏ 39 khỏi cột song song của 38. Thêm phụ thuộc còn thiếu: 55 (khối vùng banner cần 67), 59 (ký điện tử: 37; mời
   thầu: 46), 61 (37), 63 (người bán nước ngoài: 37, 46), 67 (34; P19 của 51). Mục 45 và 47 của lộ trình ghi thêm
   phần của D-234 / D-235; mục 63 bỏ "phân số nguyên"; `SiteExtensions` vào danh sách điểm cắm.

Không đổi thứ tự làm (D-228) và không đổi phạm vi bước nào.

Lý do: sửa trên giấy rẻ hơn sửa sau khi code; ba chỗ đầu nếu để tới lúc code sẽ thành hai bảng cùng một dữ liệu
hay một lời gọi trái luật thứ tự đăng ký.

Đã cân nhắc: giữ tỉ giá ở `invoices` và cho `ledger` nhận tỉ giá do bên gọi truyền vào — mọi bên gọi `post` phải tự
tra tỉ giá, dễ lệch; gộp hẳn báo có hàng lại vào `inventory` (bỏ phần của `customers`) — báo yêu thích có luật riêng
(giảm giá, 7 ngày, gom lô) đã chốt ở D-233, chỉ cần chống trùng.

## D-238 · Module `menus`: menu của website tách khỏi `content`, mục menu trỏ tới đối tượng của mọi module (bước 68) — 2026-11-14 — chủ dự án quyết

Chủ dự án quyết (14/11/2026, đề bài `docs/prompts/menus-module.md`): menu **trên website** (thanh danh mục, bảng
xổ lớn, menu chân trang, menu phụ) là một module riêng, `menus`. Menu trỏ tới bản ghi của mọi module (bài viết,
danh mục nội dung, danh mục sản phẩm, khuyến mại, gian hàng, cửa hàng), nên đặt trong `content` là sai chỗ. Thiết
kế "kết hợp" (chủ dự án đồng ý): người quản trị dựng menu, toàn quyền thứ tự và lối vào đặc biệt; mục menu trỏ
tới một đối tượng của sổ đối tượng và có thể tự lấy nhóm con. **Không gộp** với sổ điều hướng của bước 51
(`Module::navigation`), là menu của trang quản trị.

Quyết định thay phần menu của `content` ở bước 17: thiết kế và endpoint menu của D-096, D-102, D-105. Giữ luật từ
chối link `//host` của D-149 ở module mới.

Quyết định:

1. **Kernel, sổ đối tượng (D-123), theo D-038** (hàm mới đều có mặc định, module cũ không sửa):
   - `Resource.image: Option<Uuid>` (ID media), làm luôn phần này của P6 bước 51;
   - kiểu `ResourceRef { kind, id }`;
   - `ResourceResolver::resolve_many(ids, locale)`: tra nhiều đối tượng một lần, theo ngôn ngữ. Mặc định gọi
     `resolve` từng cái;
   - `ResourceResolver::children(id, locale, limit)`: mặc định rỗng;
   - `ResourceResolver::has_children()`: mặc định `false`; loại có cây trả `true`.

   Thêm hai hàm ngoài đề bài, vì hai lý do. Menu 200 mục mà tra từng cái là hàng trăm câu query. Tên danh mục
   lại theo ngôn ngữ, trong khi `resolve(id)` không nhận ngôn ngữ.
2. **`content` đăng ký `content.term`** (danh mục, thẻ):
   - tên lấy theo ngôn ngữ, thiếu thì dùng ngôn ngữ mặc định;
   - `children` trả danh mục con;
   - đường dẫn công khai `/<AUTOXIDE_CONTENT_POST_PATH_PREFIX>/category/<slug>` (thẻ: `/…/tag/<slug>`). Website
     hiện trang danh sách bằng `GET /api/v1/content/public/terms/{taxonomy}/{slug}`. Trước đây danh mục chưa có
     đường dẫn công khai nào; đây là đường dẫn chuẩn từ nay.

   `content.entry` trả thêm `image` (ảnh đại diện của bản công khai) và tra nhiều bài bằng một câu query.
3. **Module `menus`, bảng và API.**
   - Bảng `menus_menus`, `menus_items`. Mục menu có đúng một trong ba kiểu đích:
     - `url`: `https://`, `http://`, `mailto:` hay đường dẫn `/…`; từ chối `//host`;
     - `ref_kind` + `ref_id`;
     - không có đích: nhóm tiêu đề.
   - Trình bày:
     - `label`: bỏ trống thì lấy tiêu đề của đối tượng;
     - `auto_children` (1 tới 50 mục con);
     - `icon`, `highlight`: tập cố định;
     - nhãn: `badge_text`, `badge_tone` (`sale`, `brand`, `new`, `neutral`), `badge_from`, `badge_until`;
     - ô gợi ý `feature_kind` + `feature_id`.
   - Giới hạn: 4 cấp, 200 mục.
   - API quản trị `/api/v1/menus` (`menus.read`, `menus.manage`): danh sách, đọc, tạo (201), thay cả cây (`PUT`,
     bắt buộc `If-Match`), xoá. Tool MCP gọi cùng service.
   - Lỗi lưu gom một lần (D-083): `menus.ref_unknown_kind`, `menus.ref_not_found`, `menus.children_unsupported`.
4. **API công khai `GET /api/v1/menus/public/{key}?locale`.**
   - Trả cây đã giải. Bỏ mục trỏ tới đối tượng không còn công khai, kèm mục con; bỏ nhóm không còn mục con. Mở
     `auto_children`. Nhãn đã lọc theo hạn. Thiếu ngôn ngữ thì dùng ngôn ngữ mặc định.
   - Mỗi mục kèm `ref` (loại, ID) để giao diện gọi thêm khi cần. Khối "Bán chạy" của bảng xổ **không** thuộc
     module này: giao diện gọi danh sách sản phẩm công khai của `catalog` (lọc theo danh mục, sắp `best_selling`).
   - `ETag` là dấu vân tay của cả câu trả lời; kernel tự trả 304.
   - **Làm mới cache bằng thời hạn ngắn, không nghe event.** Bản đã giải nằm trong bộ nhớ, tối đa
     `AUTOXIDE_MENUS_PUBLIC_CACHE_SECONDS` (mặc định 60; 0 là tắt). Cùng số đó đi vào `Cache-Control: max-age`.
     Hai mốc rút ngắn thời hạn: menu được sửa thì xoá cache ngay; nhãn sắp tới hạn bật hay tắt thì hết hạn đúng
     lúc đó. Đối tượng đổi tên hay đổi đường dẫn thì hiện sau tối đa một thời hạn cache.
   - Ngôn ngữ: `AUTOXIDE_MENUS_LOCALES`, `AUTOXIDE_MENUS_DEFAULT_LOCALE`, giống cách của `content`.
5. **Làm trước bước 33 theo lệnh của chủ dự án.**
   - Chưa có `tenant_id` / `org_id` và tham số `site`:
     - bước 33 thêm `tenant_id` cho bảng `menus_*` cùng mọi module, và thêm khách vào `resolve`, `resolve_many`,
       `children`;
     - bước 34 thêm `org_id` (website hay nút sở hữu, H1) và `?site=`.
   - Các module sau đăng ký loại đối tượng trong bước của mình:
     - `catalog.category` (41, có `children`);
     - `promotions.promotion` (43);
     - `marketplace.seller` (46);
     - cửa hàng công khai (34).
6. **Chuyển từ menu của `content` (D-059).**
   - Bốn endpoint menu của `content` khai deprecated từ 2026-11-14, sunset 2027-05-14 (sáu tháng), kèm endpoint
     thay thế; hành vi không đổi.
   - `content` công bố trait `autoxide_content::api::LegacyMenus` để đọc menu cũ. Lệnh
     `autoxide task menus.import_content_menus` nhập menu mà `menus` chưa có cùng (`key`, `locale`); chạy lại không
     nhân đôi, không đè menu đã sửa. Mục trỏ bài thành `content.entry`, mục trỏ danh mục / thẻ thành `content.term`.
   - `menus` không đọc bảng của `content`. Không có `content` thì `menus` vẫn chạy, chỉ lệnh nhập báo không có
     nguồn.
   - Qua ngày sunset thì gỡ endpoint, và gỡ bảng `content_menus*` bằng migration mới.
   - Khối `content.menu` của bước 67 (D-234) đổi thành `menus.menu`, do module này khai.

Lý do:
- Menu của sàn thương mại trỏ tới danh mục sản phẩm, khuyến mại, gian hàng; `content` không được biết những thứ đó.
  Sổ đối tượng cho `menus` trỏ tới loại của mọi module mà không phụ thuộc chúng.
- Cache theo thời hạn ngắn đủ đúng (đổi tên danh mục hiện sau một phút) và không buộc `menus` phải biết event
  đổi tên của từng module.

Đã cân nhắc:
- Giữ menu trong `content` và thêm cột cho mỗi loại đích: mỗi module thương mại phải sửa `content`.
- Menu sinh hoàn toàn tự động từ cây danh mục: mất thứ tự tay và lối vào đặc biệt (chủ dự án loại).
- Nghe event đổi tên của mọi module để xoá cache: `menus` phải biết tên event của module khác, và dễ sót.
- Một trait chung cho mọi module có menu cũ: chỉ `content` có, nên dùng trait riêng của `content` là đủ.

Thay: phần menu của D-096, D-102, D-105 (bước 17).

## D-239 · `cargo xtask api-diff`: luật phá vỡ của `/api/v1`, chỉ `x-sunset` trong tài liệu (bước 31 A1) — 2026-11-15 — agent quyết

Quyết định:
- `cargo xtask api-diff [--base <ref>]` (`xtask/src/api_diff.rs`) so `docs/openapi.json` với bản ở tag phát hành
  gần nhất là tổ tiên của `HEAD` (`git describe --tags --abbrev=0 --match 'v*.*.*'`). Chạy trong `./gate.sh` ở cả
  hai mức, ngay sau `cargo xtask check`; job gate của CI lấy cả lịch sử (`fetch-depth: 0`).
- Bật mọi luật trong bảng của đặc tả 31 A1, kể cả các luật ngoài chữ của ROADMAP: `parameter.removed`,
  `request.constraint_tightened`, `response.status_removed`, `response.media_type_removed`,
  `schema.composite_changed`.
- Thêm một luật ngoài bảng: `request.property_removed`. Áp khi bản mới bỏ một trường của thân request **và** thân
  đó có `additionalProperties: false`. Thân request của Autoxide dùng `deny_unknown_fields`, nên client cũ gửi
  trường đó sẽ bị từ chối.
- Enum:
  - bỏ giá trị là phá vỡ ở cả hai chiều;
  - thêm giá trị trong request là `note`;
  - thêm giá trị trong response là `warning` (client chặt có thể vỡ), không đỏ;
  - request từ tự do thành danh sách cố định là `request.constraint_tightened`.
- `oneOf` / `anyOf` khác số biến thể là `schema.composite_changed`. Cùng số biến thể thì so từng cặp theo thứ tự.
- Tài liệu OpenAPI chỉ thêm **một** extension: `x-sunset`. Không thêm `x-deprecated-since`, `x-successor`: hai
  thông tin đó đã có trong câu của `description`. `Extensions` của utoipa là `HashMap`, nhiều khoá thì thứ tự trong
  `openapi.json` đổi giữa các lần sinh. Bản cũ chưa có `x-sunset` thì đọc câu "removed on YYYY-MM-DD".
- Ngoại lệ có chủ đích nằm ở `xtask/api-breaking-allow.json`. Mỗi mục cần `decision` và chỉ có hiệu lực với đúng
  `base`.

Lý do:
- Mỗi luật thêm đều là trường hợp client cũ hỏng mà không có lỗi rõ.
- Thử với các tag cũ: so với v0.3.0, lệnh bắt đúng một thay đổi phá vỡ thật (thân của
  `POST /api/v1/auth/me/mfa/totp` thành bắt buộc). Từ v0.8.0 tới nay không có thay đổi phá vỡ nào.

Đã cân nhắc:
- Dùng `oasdiff` (Go): thêm công cụ ngoài, và không biết hai cách viết nullable của utoipa.
- Báo đỏ khi thêm giá trị enum trong response: chặn cả việc thêm trạng thái mới hợp lệ; cảnh báo là đủ để người
  phát hành xem lại.

## D-240 · Hợp đồng tool MCP: `docs/mcp-tools.json`, `Deprecation::tool`, cảnh báo trong kết quả (bước 31 A6) — 2026-11-15 — agent quyết

Quyết định (chi tiết D-233 giao cho agent):
- **Lưu deprecation của tool:** thêm trường `tool: Option<&'static str>` và hàm tạo `Deprecation::tool(name, since)`
  vào `Deprecation` sẵn có, không đổi trường công khai cũ (hợp đồng module, D-167). Với tool, `method` / `path` không
  dùng.
  - `Modules::deprecations()` chỉ trả endpoint: lớp HTTP và mục `deprecations` của danh mục module không thấy tool.
  - `Modules::tools()` gắn deprecation vào tool (`Tool::deprecation()`). Tool hay tool dùng thay không có trong
    `tools()` của chính module thì là lỗi khởi động.
  - Danh mục module (`ModuleTool`) có thêm trường `deprecation` (thêm trường response, không phá vỡ).
- **Báo cho agent** (`rmcp` 3.9: `CallToolResult` có `meta` và `content` công khai):
  - `tools/list` nối câu "Tool `x` is deprecated since …; It will be removed on …; Use `y` instead.";
  - kết quả giữ envelope `{"data": …}`, thêm `_meta` `{"autoxide/deprecation": {since, sunset, successor}}` và một
    đoạn văn bản "Warning: …";
  - số đo `mcp_deprecated_tool_calls_total{tool}`. Tên tool là tập cố định do code khai, nên dùng làm nhãn được.
- **Manifest:** `autoxide mcp-manifest` in `docs/mcp-tools.json`.
  - Tool sắp theo tên; khoá tự sắp.
  - `requires_approval_for_agent` luôn `false` tới bước 51.
  - `output_schema` luôn `null`, vì `Tool` chưa khai schema kết quả.
  - Test giữ file khớp code như `docs/openapi.json`.
- **`cargo xtask api-diff`** so cả manifest với tag trước, mã `tool.*` theo bảng của đặc tả 31 A6. Phần so schema
  dùng lại bộ so của A1, ánh xạ mã `request.*` / `response.*` sang `tool.*`. Tag chưa có manifest (v0.13.0 trở
  về trước) thì bỏ qua.

Lý do: không thêm hàm mới vào `Module` (D-038) và không có hai bộ so schema. Đặt cảnh báo ngoài `{"data": …}` để
client đang đọc `data` không vỡ.

Đã cân nhắc:
- Gom `method` / `path` vào một enum đích: đổi trường công khai của `Deprecation`, phá vỡ hợp đồng Rust.
- Thêm `Module::tool_deprecations`: thêm một hàm vào trait cho việc mà hàm sẵn có làm được.

## D-241 · Response thật khớp hợp đồng: bộ kiểm tự viết trong testkit, media type của lỗi, ngoại lệ đầu tiên của `api-diff` (bước 31 B4) — 2026-11-15 — agent quyết

Quyết định:
- `TestRequest::send` (testkit) kiểm **mọi** response với tài liệu OpenAPI của chính app:
  - mã trạng thái có khai (không có thì dùng `default`);
  - `Content-Type` là một khoá trong `content`;
  - thân JSON đúng schema;
  - lệch thì `send()` trả lỗi, nêu chỗ lệch, vd `GET /api/v1/x 200: $.data.id: expected string (uuid)`.
- `TestRequest::unchecked()` bỏ kiểm, chỉ cho test cố ý tạo câu trả lời ngoài hợp đồng. Hiện chỉ dùng ở module giả
  trong test idempotency (trả 500 / 429 dạng chữ, panic).
- **Bộ kiểm JSON Schema tự viết** (`crates/testkit/src/contract.rs`, khoảng 250 dòng) cho đúng tập con utoipa sinh ra.
  Không dùng thư viện `jsonschema`.
  - Kiểm: `type` / nullable, `properties`, `required`, `additionalProperties`, `items`, `enum`, `const`, `oneOf`,
    `anyOf`, `allOf`, `$ref`, giới hạn số và độ dài, `format` `uuid` / `date-time` / `date`.
  - Không kiểm `pattern` (cần thư viện regex).
  - `oneOf` kiểm như "khớp ít nhất một", vì biến thể utoipa sinh ra có thể chồng nhau.
  - Tài liệu khai `application/octet-stream` thì nhận mọi loại không phải JSON (tệp của media có loại thật như
    `image/png`).
- **Media type của lỗi:** `http::openapi::finish` đổi mọi response có thân `Problem` khai dưới `application/json`
  thành `application/problem+json`, đúng thứ lỗi thật trả. Sửa 822 chỗ; `api-diff` coi hai media type này là một
  cho lỗi (D-239).
- **Lệch tìm ra khi bật kiểm:** chỉ một lỗi tài liệu thật. `answers[].value` của bài nộp `forms` khai là object,
  nhưng thực tế là chuỗi, số, mảng hay `true`; đã sửa thành mọi kiểu JSON. Theo luật, đó là "kiểu response đổi",
  nên ghi bốn ngoại lệ (cho bản gốc `v0.13.0`) vào `xtask/api-breaking-allow.json`, dẫn quyết định này. Còn lại là
  tài liệu sai của các route giả trong test của testkit, đã sửa.

Lý do:
- Tập schema utoipa sinh ra nhỏ và biết trước, nên vài trăm dòng là đủ (CLAUDE.md: không thêm thư viện khi tự viết
  được).
- `jsonschema` kéo theo regex và nhiều crate khác vào cây phụ thuộc của mọi crate dùng testkit.

Đã cân nhắc:
- Kiểm hợp đồng bằng một lớp HTTP riêng chỉ bật trong test: phải đổi `crates/app` và mọi module.
- Bật kiểm từng test một: dễ quên.

## D-242 · `cargo xtask semver`: chỉ kernel và testkit, ép so như bản patch, báo cáo khi còn `0.x` (bước 31 A7) — 2026-11-15 — agent quyết

Quyết định:
- `cargo xtask semver [--base <tag>] [--deny] [--markdown]` (`xtask/src/semver.rs`) gọi `cargo semver-checks
  check-release -p <crate> --baseline-rev <tag> --release-type patch` cho `autoxide-kernel` và `autoxide-testkit`.
  - Tag mặc định chọn như `api-diff` (tổ tiên gần nhất của `HEAD`).
  - Ghim `cargo-semver-checks` 0.51.0 (`STACK.md`).
- **Ép `--release-type patch`:** với `0.x`, công cụ coi bản minor như major, nên không báo gì. So như patch thì mọi
  thay đổi phá vỡ đều hiện. Thử tay: đổi tên `IfMatch::from_value` thì lệnh báo đúng `inherent_method_missing`.
- **Chế độ:**
  - còn `0.x`: chỉ báo cáo (thoát 0);
  - từ `1.0.0`, hay với `--deny`: có phá vỡ thì thoát 1.
- `--markdown` in mục `### Breaking (Rust)` để chép vào CHANGELOG. Đường dẫn trên máy build trong "previously in
  file …" được cắt về đường dẫn trong repo, vì repo công khai.
- Chạy ở bước 3 của `docs/RELEASING.md`, **không** trong `./gate.sh`.
- **Không** kiểm crate của module.

Lý do:
- Đo trên máy làm việc, so với v0.13.0:
  - kernel và testkit: 3 phút 11 giây, chiếm khoảng 7 GB ở `target/semver-checks` (dựng rustdoc JSON của bản cũ và
    bản mới);
  - thêm 9 module có `api` thì nhân lên nhiều lần, và đĩa đã đầy giữa chừng.
- Phần `api` của module hiện chỉ do module trong cùng repo dùng: đổi phá vỡ thì `cargo check` của workspace đỏ
  ngay.
- Khi có module ở repo riêng (D-164), xem lại.

Đã cân nhắc: chạy trong gate — lâu, cần mạng, và còn `0.x` thì chặn không có ích (D-167).

## D-243 · SDK TypeScript: bộ sinh viết bằng Rust trong `xtask`, gate kiểm khớp không cần Node, `tsc` trong CI (bước 31 A2) — 2026-11-15 — agent quyết

Quyết định (D-177 giao cho agent):
- `cargo xtask sdk` (`xtask/src/sdk.rs`) sinh `sdk/typescript/src/schema.ts` từ `docs/openapi.json`:
  - mỗi schema một `export type`, kèm mô tả làm chú thích;
  - interface `paths` với tham số theo vị trí, thân request và response theo mã của từng operation;
  - thứ tự sắp theo tên.
- Gate (cả hai mức) chạy `cargo xtask sdk --check`: file lệch hay `version` của `package.json` khác workspace thì đỏ,
  kèm lệnh sinh lại. Người chạy `./gate.sh` không cần Node.
- `client.ts` viết tay, không phụ thuộc thư viện:
  - `createClient`; `get` / `post` / `put` / `patch` / `delete` có kiểu suy từ `paths`;
  - `{data}` trả `data`, trang giữ nguyên;
  - lỗi ném `AutoxideError` (`code`, `detail`, `errors`, `retryAfter`);
  - `paginate()` theo `next_cursor`;
  - 304 trả `{ notModified: true }`.
- Gói là mã nguồn `.ts`, `private`, không đăng npm (D-177).
- CI:
  - job `sdk` chạy `npm ci && npx tsc --noEmit` trên SDK, `examples/smoke.ts` và ví dụ TypeScript trích từ
    `docs/guides` (A5);
  - Node 22.23.4 và `typescript` 5.9.3 ghim trong `package-lock.json`;
  - workflow `Release` đính kèm `autoxide-sdk-X.Y.Z.tgz` (`npm pack`);
  - khi repo còn riêng tư thì đính tay theo `docs/RELEASING.md`.

Lý do:
- Tập schema utoipa sinh ra nhỏ và biết trước; bộ sinh khoảng 300 dòng.
- Thử trên `docs/openapi.json` thật (366 schema, 225 operation): `tsc --strict` sạch ngay. Ví dụ dùng thử bắt đúng
  kiểu sai (`limit` là số).

Đã cân nhắc: `openapi-typescript` qua image Docker `node` ghim phiên bản — thêm Docker vào gate nhanh, và kiểu sinh
ra theo cách riêng của nó, khó đọc hơn.

## D-244 · Hướng dẫn theo tình huống: dạng `curl` hẹp chạy như test, bảng tool kiểm theo code (bước 31 A3–A5) — 2026-11-15 — agent quyết

Quyết định:
- `docs/guides/` gồm 7 tình huống, mỗi file có bản `.vi.md` cùng nội dung: `website`, `authentication`, `api-keys`,
  `retries`, `pagination`, `errors`, `mcp`, cộng `README` làm mục lục. Tệp mẫu (ảnh PNG nhỏ) nằm ở `files/`.
- **Ví dụ `curl` dạng hẹp**, chạy bằng `crates/app/src/e2e/guides.rs`:
  - khối `bash` có dòng `<!-- example: <tên> -->` ngay trước là một bước, gồm đúng một lệnh
    `curl -sS [-X] "$API/…" [-H …] [--json '…' | --data-binary @tệp]`;
  - chú thích `# save: TÊN=.đường.dẫn` hay `=header:<tên>`, `# expect: <mã>`, `# expect-header: <tên>: <giá trị>`;
  - biến tách theo luật shell (`'…'` không thay, `"…"` có thay), nên lệnh trong tài liệu chạy thật được trong
    terminal;
  - mỗi hướng dẫn một database trống, dựng bằng `TestApp` với mọi module của binary (`TestAppBuilder::boxed_module`,
    hàm mới của testkit), đăng nhập thật;
  - response được kiểm theo hợp đồng (D-241), trừ bước chờ `304`.
- **Khối code của bản tiếng Việt giống hệt bản tiếng Anh** (test kiểm). Chú thích trong code viết tiếng Anh.
- **Ví dụ TypeScript** trích bằng `cargo xtask sdk --extract-examples`, kiểm kiểu cùng SDK trong job `sdk` của CI.
  - Lần đầu, `tsc` bắt ba lỗi của ví dụ: `me.user` không có, `Problem` không có `request_id` (mã request nằm ở header
    `X-Request-Id` — SDK thêm `AutoxideError.requestId`), `process` cần kiểu của Node (ví dụ không đọc
    `process.env` nữa).
- **Bảng tool của `mcp.md`** nằm giữa `<!-- tools:start -->` / `<!-- tools:end -->`, mỗi dòng
  `| \`tên\` | permission | read-only / writes / destructive |`. Test so với `Modules::tools()`, sai thì in đúng
  các dòng phải có.

Lý do: tài liệu viết tay lệch code rất nhanh; khi ví dụ chạy trong gate, lệch là đỏ ngay. Bộ chạy riêng cho dạng
hẹp khoảng 300 dòng, không cần shell thật trong test (chạy được trên mọi máy, mọi database).

Đã cân nhắc: chạy khối `bash` bằng shell thật với máy chủ thật — cần cổng mạng, `curl` trên máy và dọn tiến trình,
chậm và dễ hỏng trên CI.

## D-245 · Hợp đồng quyền đọc được bằng máy và test độ phủ phân quyền (bước 31, đặc tả 54 P1) — 2026-11-15 — agent quyết

Quyết định (chi tiết D-205 / đặc tả 54 P1 giao cho bước 31):
- **Khai trên handler bằng `extensions` của `#[utoipa::path]`** (utoipa 6 có sẵn, không cần macro riêng):
  - `("x-permissions" = json!([...]))`: có một trong các permission là đủ; danh sách rỗng là mọi người đã đăng nhập;
  - hoặc `("x-public" = json!(true))`.
  - Khai cho cả 226 operation bằng script đọc thông tin từ tài liệu hiện có: `security` và câu "Missing permission"
    của response 403. Kết quả: 45 công khai, 24 chỉ cần đăng nhập, 157 có permission. Test độ phủ bên dưới kiểm lại.
- **Lint của kernel (`http::lint`):** operation thiếu cả hai là lỗi; mỗi permission khai phải được nêu trong
  mô tả hay response 403.
  - Đặc tả muốn câu "Requires …" *sinh từ* khai báo. Tôi giữ câu viết tay, vì nó giàu hơn (vd "`files.upload` cho
    file của bạn, `files.manage` cho file của mọi người"), và cho lint kiểm khớp. Mục đích "chữ không lệch khai báo"
    vẫn đạt.
- **Tài liệu OpenAPI xuất qua `serde_json::Value`** để mọi khoá sắp theo tên. Extension của operation nằm trong
  `HashMap` của utoipa; operation có cả `x-permissions` lẫn `x-sunset` thì thứ tự sẽ đổi giữa các lần sinh.
  `docs/openapi.json` sắp lại toàn bộ một lần, không đổi nghĩa (`api-diff` xác nhận).
- **`Permission` có `risk`:**
  - mặc định suy từ tên (`….read` là `read`, còn lại `write`);
  - `with_risk` đặt khác: `sensitive_data` cho xem / xuất dữ liệu cá nhân (`auth.users.read`, `comments.read`,
    `comments.export`, `forms.read`, `forms.export`, `newsletter.read`, `newsletter.export`, `files.manage`),
    `destructive` cho `media.purge`, `content.delete`, `read` cho `ai.view`;
  - danh mục module trả `risk` cho từng permission.
- **Test độ phủ** (`crates/app/src/e2e/access.rs`, mọi module của binary, cả hai database):
  - Request mẫu sinh từ tài liệu: tham số bắt buộc, `If-Match: *`, thân tối thiểu theo schema. Nhờ vậy lỗi kiểm dữ
    liệu không che bước kiểm quyền.
  - Mọi operation không công khai: không token → 401. Tài khoản thật không role nào → 403 `permission_denied` (404
    nếu operation khai `x-hides-existence`). Với `[]` thì không bị từ chối.
  - Permission khai phải có module khai thật.
- **Tool MCP:**
  - mỗi tool `.requires(…)` hoặc khai `.for_any_signed_in()` (hàm mới, tương đương `x-permissions: []`);
  - tool chỉ đọc phải gọi được bằng ít nhất một permission mức `read` / `sensitive_data` (đặc tả ghi "chỉ yêu cầu
    permission mức read"; nới thành "ít nhất một", vì tool như `get_file` cho cả `files.upload` lẫn
    `files.manage`);
  - ngoại lệ ghi trong test: `files.list_files` (`files.upload` gồm cả xem file của mình; `files` không có
    permission chỉ đọc).
- **Lỗi thật test tìm ra, đã sửa:**
  - sáu endpoint `…/data` của `comments`, `forms`, `newsletter` đọc `email` trước khi kiểm quyền (người không quyền
    nhận 422); handler giờ gọi `principal.require` trước;
  - tool `files.get_file` / `read_file` / `delete_file` và bốn tool của `auth` không khai permission; endpoint
    `files` cho `files.manage` mà tài liệu chỉ ghi `files.upload`.

Lý do: endpoint mới quên kiểm quyền (hay kiểm sau khi đọc đầu vào) làm gate đỏ; AI agent và giao diện biết trước
quyền của từng endpoint từ tài liệu.

Đã cân nhắc:
- Macro riêng bọc `#[utoipa::path]`: thêm một thứ phải học, trong khi `extensions` đã đủ.
- Sinh `x-permissions` tự động từ lời gọi `principal.require` trong code: không đọc được lúc biên dịch, và có
  service kiểm quyền theo điều kiện.

## D-246 · Test nâng cấp từ bản phát hành trước: Docker, ngoài gate (bước 31 B1) — 2026-11-15 — agent quyết

Bối cảnh: migration và dữ liệu cũ chỉ được thử trên database do **chính bản hiện tại** dựng. Lỗi chỉ lộ khi nâng
cấp thật (migration mới hỏng trên dữ liệu của bản cũ, đổi cách lưu token, đổi định dạng file) thì không test nào
bắt.

Quyết định:

- **Test** `crates/app/tests/upgrade.rs`, chạy trên database `TestDatabase` chọn (SQLite hay Postgres):
  - bản cũ chạy bằng image Docker của nó (`TEST_UPGRADE_IMAGE`): `migrate up`, `user create`, `serve`;
  - ghi dữ liệu mẫu qua HTTP thật: role, API key, ảnh PNG (chờ job ảnh nhỏ), bài nháp, bài đã đăng có ảnh đại diện
    và ảnh trong thân, form, danh sách newsletter, webhook, menu (từ 0.13.0, đọc `--version` của image);
  - dừng bằng `docker stop` (SIGTERM), kiểm container thoát 0;
  - bản hiện tại (binary vừa build) trên **cùng database và cùng thư mục `data/`**: `migrate up`, `doctor`,
    `serve`;
  - đọc lại mọi bản ghi: mọi trường bản cũ trả về, bản mới trả y hệt (bản mới được thêm trường);
  - access token, refresh token, API key và mật khẩu cũ còn dùng được;
  - vài lệnh ghi: sửa bài, tải ảnh mới, tạo role; rồi `task media.refresh_variants`.
- **Chưa đặt `TEST_UPGRADE_IMAGE` thì test in "skipped" và qua**, như test S3 với `TEST_S3_ENDPOINT`. Vì thế test
  có trong `cargo test --workspace` nhưng không làm gì khi chạy `./gate.sh`.
- **Lệnh `cargo xtask upgrade-test [--from <tag>] [--image <image>] [--database sqlite|postgres|both]`:**
  - tag cũ mặc định là tag phát hành gần nhất trước HEAD (HEAD chính là tag thì lấy tag trước nó);
  - image: thử tải `ghcr.io/<chủ>/<repo>:X.Y.Z` (và `:vX.Y.Z`); không được thì dùng `autoxide:vX.Y.Z` đã build
    trên máy, hay build từ tag (`git worktree add` → `docker build --network host`, truyền biến proxy chuẩn);
  - chạy test trên SQLite, rồi trên Postgres (`GATE_POSTGRES_URL`, không có thì dựng container như `gate.sh`).
- **Không vào `./gate.sh`.** Lệnh cần Docker, tải hay build bản release của tag cũ, mất nhiều phút. Nó chạy ở job
  `upgrade` của CI (ma trận `sqlite` / `postgres`, đăng nhập ghcr.io bằng `GITHUB_TOKEN` để tải image) và ở bước 4
  của `docs/RELEASING.md`, trước khi gắn tag: đỏ thì không phát hành.
- **Giới hạn:** container chạy với `--network host`, nên lệnh chỉ chạy trên Linux. Nhờ vậy bản cũ gọi được Postgres
  của test, và test gọi được bản cũ ở `127.0.0.1` mà không cần ánh xạ port. Container chạy bằng uid/gid của máy để
  file trong thư mục gắn vào ghi được và xoá được.
- `reqwest` (đã có trong workspace) thêm vào `[dev-dependencies]` của `autoxide` làm client HTTP của test.

Lý do: chỉ image thật của bản cũ mới đúng là thứ người dùng đang chạy. Build lại bản cũ từ mã nguồn trong cùng
workspace thì không được, vì hai bản trùng tên crate.

Đã cân nhắc:
- Giữ sẵn file database mẫu của từng bản trong repo: không thử được Postgres, và file phải sinh lại mỗi lần phát
  hành.
- Chạy trong gate: mỗi lần gate lại tốn thêm một lần build bản release và cần Docker. Trong khi đó gate phải chạy
  được trên máy chỉ có Postgres (`GATE_POSTGRES_URL`).

## D-247 · Bố cục theo tính năng của bốn module và nhóm file của kernel (bước 32 A1, A2) — 2026-11-15 — agent quyết

Bối cảnh: `newsletter`, `comments`, `media`, `forms` (6–10 nghìn dòng mỗi module) và kernel (37 file nằm phẳng)
chưa theo quy ước "File layout" của `docs/MODULE_GUIDE.md`. Bước 33 sẽ sửa mọi bảng của mọi module và thêm
khoảng tám module vào kernel; người sửa cần tìm đúng chỗ ngay.

Quyết định:

- **Module: theo cách của `content`.**
  - File tính năng chính giữ logic và khai các file con trong thư mục cùng tên: `campaigns.rs` + `campaigns/`,
    `posting.rs` + `posting/`, `moderation.rs` + `moderation/`, `forms.rs` + `forms/`.
  - Nhóm không có file chính thì là file nhóm chỉ khai module con: `audience.rs`, `submissions.rs`,
    `library.rs`, `images.rs`.
  - Tính năng chỉ có một file giữ ở trên cùng, không bọc thêm nhóm: `public.rs` của `newsletter` (cùng tên
    với `http/public.rs`), `threads.rs` của `comments`. Bảng của đặc tả đặt chúng vào `signup/`, `reading/`;
    nhóm một file không giúp tìm nhanh hơn mà còn đổi tên.
  - Bố cục cụ thể:

    | Module | Nhóm |
    | --- | --- |
    | `newsletter` | `audience/` (`lists`, `subscribers`, `bounces`); `campaigns/` (`sending`, `new_posts`, `compose`) |
    | `comments` | `posting/` (`authoring`, `targets`); `moderation/` (`reports`) |
    | `forms` | `forms/` (`fields`); `submissions/` (`public`, `inbox`, `csv`, `notify`) |
    | `media` | `library/` (`upload`, `sniff`, `folders`, `tags`); `images/` (`variants`, `refresh`, `alt`, `alts`) |

  - `tests.rs` của `forms` và `media` thành `tests/`: đồ dùng chung ở `mod.rs`, mỗi tính năng một file;
    file con viết `use super::*;` như `mod tests` thường làm.
  - Chỉ dời, không đổi tên hàm, kiểu, mã lỗi, job, event, permission hay tool. Số test và tên lá của từng
    crate trước và sau như nhau: `newsletter` 28, `comments` 40, `forms` 29, `media` 49, kernel 324 (doc test
    đổi tên vì tên của chúng mang đường dẫn file).
- **Kernel: năm nhóm riêng tư, đường dẫn công khai không đổi.**
  - Năm nhóm:

    | Nhóm | Module | Bước 33+ thêm vào |
    | --- | --- | --- |
    | `contracts/` | `ai`, `mail`, `privacy`, `resource`, `storage` | `payment`, `einvoice`, `identity`, `sms`, `entitlement` |
    | `data/` | `database`, `migration`, `pagination`, `reference`, `rows`, `search`, `transfer`, `version` | `tenancy`, `numbering` |
    | `security/` | `access`, `antispam`, `idempotency`, `ip_hash`, `secret`, `throttle`, `tls`, `tokens` | — |
    | `ops/` | `doctor`, `logging`, `metrics`, `task`, `telemetry`; riêng tư: `builtin`, `module_catalog`, `user_flags` | — |
    | `values/` | `email`, `money`, `public_url` | `decimal` |

  - Giữ trên cùng: `error`, `response`, `i18n`, `module`, `config`, `http`, `event`, `job`, `mcp`. Đây là
    phần mọi module chạm tới. `mcp` không có trong bảng của đặc tả; nó là hợp đồng như `http`.
  - `user_flags` (D-207) vào `ops/` cạnh `builtin`, vì module nội bộ `kernel` phục vụ nó.
  - `lib.rs` khai `mod <nhóm>;` riêng tư và `pub use <nhóm>::{…};`. `autoxide_kernel::ai::TextModel` và
    `crate::ai::…` vẫn chạy. Không có đường thứ hai qua tên nhóm.
  - `crates/kernel/tests/public_paths.rs` `use` mọi module công khai cùng một kiểu tiêu biểu: mất đường dẫn
    nào là lỗi biên dịch.
  - Đã sửa theo đường dẫn mới: allowlist của `.gitleaks.toml` cho `security/antispam/fixtures/altcha_v2.json`.
    Không phải sửa: `ENV_ACCESS_FILE` (`config/` giữ trên cùng), `include_str!` của `i18n` (giữ trên cùng) và của
    `antispam` (dời cả thư mục). `cargo doc` với `-D warnings` sạch.
- Tài liệu đang dùng (MODULE_GUIDE, đặc tả bước sau, ROADMAP) sửa theo đường dẫn mới. Nhật ký và đặc tả của
  bước đã xong giữ nguyên: chúng ghi lại lúc đó.

Lý do: bước 33 sửa từng module và thêm module vào kernel theo đúng một chỗ dễ đoán. Module ở repo riêng (D-167)
không phải sửa dòng nào.

Đã cân nhắc:
- Re-export trong file nhóm (`pub(crate) use lists::List`) để khỏi sửa đường dẫn bên trong module: thành hai
  cách gọi một thứ.
- Đặt tên nhóm vào đường dẫn công khai của kernel (`autoxide_kernel::contracts::ai`): phá vỡ hợp đồng module.

## D-248 · Phát hành gom thành bản lớn, không phát hành sau từng bước — 2026-11-15 — chủ dự án quyết

Bối cảnh: theo D-171, agent tự phát hành sau mỗi bước của lộ trình. Mỗi bản phát hành cần chủ dự án tự tạo
trang phát hành trên GitHub, vì phiên làm việc không đẩy được tag. Chủ dự án (15/11/2026): "Làm xong rồi
release 1 bản lớn thôi, đừng release từng cái nhỏ nữa".

Quyết định (thay D-171):

- **Xong một bước** (mọi mục `[x]`, gate đầy đủ xanh): merge vào `main` (PR, merge kiểu rebase), ghi thay đổi
  vào `## [Unreleased]` của `CHANGELOG.md`. Không đổi số phiên bản, không gắn tag.
- **Phát hành** theo `docs/RELEASING.md`, agent tự làm không hỏi, khi xong trọn một giai đoạn của lộ trình (A, B,
  C…: mọi bước của giai đoạn đã `[x]`), hay khi chủ dự án bảo. Bản đầu tiên theo luật này gồm bước 32 trở đi của
  giai đoạn A.
- Phần còn lại của D-171 giữ nguyên: không phát hành giữa bước, không phát hành khi `[Unreleased]` trống, theo
  luật SemVer của `RELEASING.md` (D-078).
- Đặc tả bước 32 ghi "merge nhóm A vào `main` ngay khi xong, để bước 33 bắt đầu". Bước 33 làm tiếp trên cùng
  nhánh này, nên nhóm A theo bước 32 vào `main` lúc bước 32 xong.

Lý do: ít lần chủ dự án phải tạo bản phát hành bằng tay; mỗi bản có đủ thay đổi để người dùng đáng nâng cấp.
`main` vẫn luôn xanh và mới sau mỗi bước.

## D-249 · `forms`: xuất CSV lớn bằng job, link tải ký, file theo dữ liệu cá nhân (bước 32 B3) — 2026-11-15 — agent quyết

Bối cảnh: `GET /api/v1/forms/{id}/submissions/export` dựng cả file trong bộ nhớ và giữ request tới khi xong. Form
có hàng chục nghìn bài thì chậm và tốn bộ nhớ. Thêm giới hạn từ chối cho endpoint cũ là thay đổi phá vỡ (D-059).

Quyết định:

- Giữ endpoint đồng bộ như cũ. Thêm ba endpoint:
  - `POST /{id}/submissions/exports`: thân không bắt buộc, `status` như bản đồng bộ; trả 201 + `Location`.
  - `GET /exports/{export_id}`: trạng thái `pending` / `running` / `ready` / `failed`, `row_count`, `size`,
    `expires_at`, và `download_url` khi `ready`.
  - `GET /exports/{export_id}/file?expires=&signature=`: `x-public`, không cần đăng nhập.
  - Bảng `forms_exports` (migration `m0007_create_exports`), job `forms.export_submissions` xếp trong cùng
    transaction với dòng.
- **Một bản đang chạy mỗi form; trùng thì 409** `forms.export_in_progress`, id bản đang chạy nằm trong thông báo.
  Không trả lại bản đang chạy, để mỗi yêu cầu có một câu trả lời rõ. Transaction khoá dòng form (`rows::lock_one`)
  trước khi xét, nên yêu cầu đồng thời ra đúng một 201 trên cả hai database (có test).
- **Cùng nội dung từng byte với bản đồng bộ.** `csv::write` tách thành `csv::BOM` và `csv::line`. Cột
  (`export_columns`) và đọc trang 200 bài (`export_page`) là hàm dùng chung. Job ghi theo luồng
  (`futures_util::stream::try_unfold`) vào `dyn Storage`, khoá `forms/exports/<id>.csv`: không giữ cả file trong
  bộ nhớ.
- **Lỗi thì `failed`, không thử lại.** Job trả `Ok`, người dùng yêu cầu bản mới. Lỗi giữa chừng mà thử lại thì
  file dở bị ghi đè và dễ lặp vô ích. Job chạy lại khi bản xuất đã `ready` / `failed` thì không làm gì.
- **Link ký:**
  - Khoá ký: HMAC-SHA256 bằng `SecretKey::derive("forms.export_download")`, trên `export_id` (16 byte) và
    `expires` (giây Unix, 8 byte big-endian); chữ ký là hex.
  - So chữ ký bằng `verify_slice` (thời gian không đổi). Sai chữ ký, sửa hạn hay đã hết hạn: cùng một mã 403
    `forms.export_link_invalid`. File hay dòng đã xoá: 404.
  - Link sống `AUTOXIDE_FORMS_EXPORT_LINK_MINUTES` (mặc định 60, 1–10080) và không quá hạn giữ file. Mỗi lần đọc
    bản xuất ra link mới.
  - Link là URL tuyệt đối khi có `AUTOXIDE_HTTP_PUBLIC_URL`, không thì đường dẫn trên API.
- **File chứa dữ liệu cá nhân:**
  - Giữ `AUTOXIDE_FORMS_EXPORT_KEEP_HOURS` (mặc định 24, 1–720) sau khi xong; bản `failed` cũng có hạn.
  - Job `forms.cleanup` xoá file trước, dòng sau; xoá file lỗi thì để dòng cho lần sau.
  - `DELETE /api/v1/forms/data?email=` và xoá form xoá mọi bản xuất của form liên quan, sau transaction (nơi lưu
    có thể ở xa).
  - Tài khoản bị xoá thì `requested_by` thành `NULL`.
  - Event mới `forms.submissions_exported` (subject là form, `after`: `export_id`, `row_count`, không email):
    audit biết ai lấy dữ liệu ra.
- `forms` cần `dyn Storage` (đăng ký sau `files` — đã đúng) và `PublicUrl`.
- `MemoryStorage` dời từ `media` sang testkit (API mới, công khai) kèm `fail_writes` để thử đường lỗi của nơi lưu;
  `media` dùng lại.
- Không có tool MCP: dữ liệu cá nhân hàng loạt, như việc xuất / xoá theo email.

Căn cứ: Nghị định 13/2023/NĐ-CP (hiệu lực 01/7/2023) và Luật Bảo vệ dữ liệu cá nhân 2025 (hiệu lực 01/01/2026) —
quyền xoá dữ liệu, nên file xuất phải bị xoá theo. Thời hạn giữ file xuất là cấu hình; cần đối chiếu văn bản gốc
hay người có chuyên môn pháp lý cho trường hợp cụ thể (D-186).

Đã cân nhắc:
- Trả bản đang chạy thay vì 409: client không biết đó là bản cũ hay bản mình vừa yêu cầu.
- Link tải cần đăng nhập: không mở được trong trình duyệt khi client dùng token. Chữ ký gắn với id và hạn là đủ.

## D-250 · `auth`: đổi email thì báo địa chỉ cũ, kèm cách báo "không phải tôi" (bước 32 B1) — 2026-11-15 — agent quyết

Bối cảnh: D-065 ghi "báo địa chỉ cũ để sau". Kẻ biết mật khẩu đổi được email sang địa chỉ của nó: chủ thật mất cả
đường đặt lại mật khẩu, mà không hay biết gì.

Quyết định:

- **Mail báo đi qua job, xếp trong transaction của `confirm`.**
  - `EmailVerifications::confirm` (khi email thực sự đổi) xếp job `auth.send_email_changed` trong cùng
    transaction. Xác nhận rollback (vd địa chỉ bị người khác lấy) thì không có mail. Request không chờ mail.
  - Payload: `user_id`, hai địa chỉ, lúc đổi, ngôn ngữ (`Accept-Language` của người xác nhận). Job queue xoá
    payload khi xong (D-079).
- **Job:** tạo token một lần (`OneTimeTokens::new("auth")`, mục đích `email_change_report`, `subject` = id,
  `data` = địa chỉ cũ, sống **7 ngày** — hằng số `REPORT_DAYS`, vì mail báo hay được đọc muộn). Mail tới **địa chỉ
  cũ** ghi địa chỉ mới **che bớt** (`n***@example.org`: đủ để chủ nhận ra, không lộ trọn địa chỉ kẻ chiếm cho ai
  đọc trộm hộp thư cũ). Mẫu `auth.mail.email_changed_link` dẫn tới Setting mới
  `AUTOXIDE_AUTH_EMAIL_CHANGE_REPORT_URL` (có `{token}`); không đặt thì `auth.mail.email_changed_code` chứa token.
- **`POST /api/v1/auth/email-change/report`** (`x-public`, thân `{ "token" }`, trả 204). Một transaction:
  1. dùng token; khoá dòng người dùng (`rows::lock_one`), nên hai lần báo đồng thời chỉ một bên được (có test);
  2. địa chỉ cũ còn trống thì trả email về đó (kiểm trước, vì lỗi unique làm hỏng cả transaction trên Postgres);
  3. khoá tài khoản bằng `UserService::lock_out_in`: bỏ mật khẩu (`NULL`: không đăng nhập bằng mật khẩu được
     nữa), thu hồi phiên và refresh token, API key, passkey, token xác minh / đặt lại / báo đang chờ, tắt TOTP
     kèm recovery code (`mfa::service::remove_in`, tách từ hàm tắt MFA sẵn có);
  4. event mới `auth.email_change_reported` (`before.email`, `after.email`, `after.restored`);
  5. khi email đã về địa chỉ cũ: xếp job `auth.send_password_reset` (D-162) tới đó.
- **Địa chỉ cũ đã bị người khác lấy:** tài khoản vẫn bị khoá nhưng không gửi mail đặt lại nào. Mail đó sẽ tới
  người khác, hay tới kẻ chiếm. Chủ tài khoản nhờ quản trị viên (`autoxide user …`).
- Token sai, hết hạn, đã dùng, hay tài khoản đã bị xoá: cùng một lỗi 400 `auth.invalid_email_change_report_token`,
  không cho dò gì.
- Không thêm giới hạn lượt riêng cho endpoint: token 256 bit ngẫu nhiên không đoán được, như
  `/password-reset/confirm`.
- `EmailVerifications::confirm` nhận thêm `Language` (cho mail báo).

Lý do: chủ tài khoản biết ngay khi email bị đổi, và lấy lại được tài khoản chỉ bằng hộp thư cũ. Kẻ chiếm mất mọi
đường vào đã tạo (phiên, API key, passkey, MFA).

Đã cân nhắc:
- Gửi mail báo trực tiếp trong request: chậm request, và gửi cả khi transaction rollback.
- Giữ mật khẩu cũ, chỉ thu hồi phiên: kẻ chiếm đã biết mật khẩu nên đăng nhập lại ngay được.

## D-251 · `auth`: quản trị viên xem và gỡ passkey của người khác; `autoxide user remove-passkeys` (bước 32 B2) — 2026-11-15 — agent quyết

Bối cảnh: người dùng mất thiết bị giữ passkey thì chỉ tự gỡ được khi còn đăng nhập. `autoxide user reset-mfa`
(D-159) chỉ tắt TOTP, không gỡ passkey, nên admin duy nhất mất thiết bị không có đường nào.

Quyết định:

- **Permission mới `auth.passkeys.manage`** (`Risk::Destructive`): "See and remove the passkeys of other users".
  Tách khỏi `auth.users.manage`, vì gỡ passkey là việc khôi phục tài khoản, nên giao riêng cho người lo việc đó.
- **Ba endpoint dưới `/api/v1/auth/users/{id}/passkeys`:**
  - `GET`: danh sách, cùng dạng `GET /me/passkeys`, không có khoá.
  - `DELETE …/{passkey_id}`: gỡ một passkey.
  - `DELETE`: gỡ hết; vẫn 204 khi người đó không có passkey nào. Có cả hai kiểu gỡ: mất một thiết bị hay mất
    hết đều là việc thường gặp.
  - Mỗi endpoint kiểm permission, người đích có tồn tại, rồi `require_covers` (không tác động lên người quyền
    cao hơn, D-079).
- **Không đổi mật khẩu, không đăng xuất phiên khác**, giống `DELETE /users/{id}/mfa`: đây là giúp người mất thiết
  bị, không phải khoá kẻ chiếm. Việc khoá kẻ chiếm có `POST /auth/email-change/report` (D-250) và đặt lại mật
  khẩu.
- **Event:** dùng lại `auth.passkey_removed`, actor là quản trị viên. Thêm `after.by_administrator: true` (như
  `auth.mfa_disabled`, D-159); người dùng tự gỡ thì `after` vắng như cũ. `passkey::remove_all_in` nhận thêm cờ
  `by_admin`.
- **CLI `autoxide user remove-passkeys --email …`** cạnh `reset-mfa`, in số passkey đã gỡ. Trait công khai
  `autoxide_auth::api::AccountRecovery` thêm `remove_passkeys(email) -> (User, u64)`, chạy với
  `Actor::system()`; không có tài khoản thì `auth.account_not_found`. Đây là thay đổi phá vỡ cho ai tự cài trait
  này (chỉ `auth` cài): ghi CHANGELOG, chấp nhận ở `0.x` (D-167).
- Hai lệnh khôi phục của CLI dùng chung một đường (`enum Recovery`, `recover`).
- `docs/DEPLOY.md` (EN + VI): cách dùng cạnh `reset-mfa`.
- **Không có tool MCP:** việc khôi phục tài khoản không giao cho agent.
