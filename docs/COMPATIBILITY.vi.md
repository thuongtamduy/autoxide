# Chính sách tương thích

[English](COMPATIBILITY.md) · **Tiếng Việt**

Trang này nói bạn dựa vào được gì khi nâng cấp Autoxide, và cái gì có thể đổi. Áp dụng từ bản
**1.0.0**. Trước 1.0 (`0.x`), một bản minor (`0.18` → `0.19`) được phép phá vỡ mọi hợp đồng dưới
đây, kể cả HTTP API; mỗi chỗ phá vỡ được ghi ở mục **Changed** hay **Removed** của
[`CHANGELOG.md`](../CHANGELOG.md) kèm việc cần làm thay thế. Từ 1.0.0 các luật dưới đây áp trọn
vẹn: không gì trong `/api/v1` bị phá vỡ.

Autoxide đánh số theo [Semantic Versioning](https://semver.org): một số phiên bản chung cho máy chủ,
các crate Rust và SDK TypeScript.

| Bản phát hành | Ví dụ           | Được phép chứa                                                        |
| ------------- | --------------- | --------------------------------------------------------------------- |
| Patch         | `1.4.0 → 1.4.1` | Sửa lỗi và vá bảo mật. Không tính năng mới, không biến cấu hình mới phải đặt. |
| Minor         | `1.4.1 → 1.5.0` | Tính năng mới, endpoint mới, biến cấu hình mới, deprecation. Không phá vỡ. |
| Major         | `1.9.3 → 2.0.0` | Thay đổi phá vỡ, mỗi cái ghi kèm cách làm thay thế.                    |

"Phá vỡ" là mọi thay đổi khiến một client, một bản cài đặt hay một module làm đúng tài liệu không
còn chạy được nếu không sửa.

## Hợp đồng ổn định gồm những gì

Mọi thứ trong mục này thuộc các module hạng **core** và **official**
([`docs/MODULES.vi.md`](MODULES.vi.md)). Module thử nghiệm không nằm trong đó (xem
[Phần không cam kết](#phần-không-cam-kết)).

### REST API và OpenAPI

Hợp đồng là `docs/openapi.json`, phục vụ ở `/api/v1/openapi.json`.

- Trong `/api/v1` không có gì bị phá vỡ, ở bất kỳ bản nào: không bỏ hay đổi tên endpoint, không thêm
  đầu vào bắt buộc, không bỏ hay đổi kiểu trường của response, không đổi nghĩa của status code.
- Thay đổi sẽ làm vỡ client thì làm thành **endpoint mới**. Endpoint cũ chạy y như cũ và được khai
  deprecated (`Module::deprecations`): tài liệu OpenAPI đánh dấu `deprecated`, response mang header
  `Deprecation`, `Sunset` và `Link` (RFC 9745, RFC 8594) trỏ tới endpoint thay thế.
- Endpoint deprecated chỉ bị bỏ sau **ngày sunset**, ít nhất **sáu tháng** sau bản phát hành khai
  deprecated, và không bao giờ trong một bản patch.
- Bản minor được **thêm**: endpoint, trường tuỳ chọn của request, tham số query, trường của response,
  header, và giá trị mới trong enum của response. Client phải bỏ qua trường response không biết và
  coi giá trị enum lạ là "khác".
- `/api/v2` (nếu có) chỉ xuất hiện ở bản major, và `/api/v1` vẫn chạy song song ít nhất thêm một bản
  major nữa.
- `cargo xtask api-diff` trong gate so mọi thay đổi với bản phát hành trước và đỏ khi có phá vỡ.

### Mã lỗi

Trường `code` của lỗi (`notes.title_taken`, `validation_failed`…) là hợp đồng với chương trình.

- Mã không bao giờ đổi tên, không bao giờ dùng lại với nghĩa khác.
- Request hỏng với một mã thì sau bản minor vẫn hỏng với đúng mã đó.
- Bản minor được thêm mã mới, cho lỗi mới. Client phải xử lý mã không biết theo HTTP status.
- Câu `message`, bản dịch của nó và `errors[].detail` là cho người đọc, có thể đổi ở bất kỳ bản nào.

### Cấu hình

Các biến môi trường (`.env.example` liệt kê đủ), nghĩa và giá trị mặc định của chúng.

- Không bỏ hay đổi tên biến trong bản minor. Khi buộc phải đổi tên, tên mới được thêm và **tên cũ
  vẫn chạy**, kèm cảnh báo trong log khởi động nêu tên mới, ít nhất một bản minor và ít nhất sáu
  tháng; tên cũ chỉ bị bỏ ở bản major.
- Giá trị mặc định làm đổi hành vi (vd giới hạn chặt hơn) chỉ đổi ở bản major, trừ khi để vá lỗ hổng
  bảo mật; khi đó changelog ghi ở mục **Security**.
- Biến mới luôn có mặc định, nên file `.env` sẵn có vẫn chạy.
- `AUTOXIDE_MODULES` theo cùng luật: trong một bản major, không module nào bị chuyển từ official sang
  experimental (việc đó sẽ tắt nó theo mặc định).

### Database và migration

- Bản mới luôn đọc được dữ liệu của bản cũ: chạy binary mới (hay `autoxide migrate up`), migration
  đưa database lên. Nâng từ **bất kỳ bản 1.x** lên bất kỳ bản 1.x mới hơn được ngay, không phải dừng
  ở từng bản minor. CI thử việc nâng cấp từ bản phát hành trước trên cả SQLite và PostgreSQL.
- Migration đã phát hành không bao giờ bị sửa; thay đổi đi bằng migration mới.
- Mọi migration có bước `down`, nên quay về được bản trước: dùng binary **mới**, chạy
  `autoxide migrate down <module> --steps <n>` cho các migration nó thêm (changelog và
  `autoxide migrate status` cho biết là những cái nào), rồi chạy binary cũ. Backup trước; đi xuống có
  thể bỏ dữ liệu mà chỉ bản mới lưu.
- SQLite và PostgreSQL đều được hỗ trợ ở mọi bản. Một bản major của PostgreSQL mà dự án PostgreSQL đã
  ngừng bảo trì có thể thôi được hỗ trợ ở một bản minor, báo trước trong changelog một bản minor.
- Bản thân các bảng **không** là hợp đồng: chúng thuộc module của mình và có thể đổi hình dạng ở bất
  kỳ bản nào. Đọc và ghi dữ liệu qua API, không bằng SQL.

### Tool MCP

Danh sách tool (`docs/mcp-tools.json`, do MCP endpoint phục vụ) theo luật của REST: tool giữ tên,
nhận mọi đầu vào nó từng nhận và trả mọi trường nó từng trả. Tool phải đổi thì làm tool mới và khai
tool cũ deprecated kèm ngày sunset (`Deprecation::tool`), ít nhất sáu tháng. Tool không bao giờ cần
permission rộng hơn trước.

### Event và webhook

Tên event và các trường trong payload (thứ module nhận và webhook gửi đi) được thêm trường ở bản
minor, nhưng không bao giờ mất hay đổi kiểu một trường. Event phải đổi hình dạng thì phát dưới tên
mới, và event cũ vẫn được phát tới bản major kế tiếp.

### Dòng lệnh

Các lệnh và tuỳ chọn `autoxide` có trong tài liệu, mã thoát của chúng và đầu ra của tuỳ chọn `--json`
theo luật như cấu hình: thêm ở bản minor, bỏ hay đổi chỉ ở bản major. Chữ in ra cho người đọc có thể
đổi.

### Rust API cho module

Với người viết module ở repo riêng ([hướng dẫn](guides/writing-a-module.vi.md)), hợp đồng là phần
Rust công khai của:

- `autoxide-kernel` và `autoxide-testkit`;
- `autoxide::Autoxide`, thứ công khai duy nhất của crate `autoxide`;
- `pub mod api` và `pub fn module(…)` của mọi crate module hạng core và official.

Luật, theo [hướng dẫn SemVer của Cargo](https://doc.rust-lang.org/cargo/reference/semver.html):

- Thay đổi phá vỡ (bỏ hay đổi tên thứ công khai, đổi chữ ký hàm, thêm method bắt buộc vào trait mà
  không có mặc định, …) chỉ có ở bản **major**.
- Trước khi bị bỏ, một thứ được đánh dấu `#[deprecated]` kèm ghi chú nêu thứ thay thế, ít nhất một bản
  minor.
- Tính năng mới cho module đến dưới dạng method mới của trait **có cài đặt mặc định**, nên module sẵn
  có biên dịch được mà không phải sửa.
- `cargo-semver-checks` so các crate này với bản phát hành trước ở mọi pull request chạm vào chúng và
  ở mọi pull request phát hành; từ 1.0, có phá vỡ thì kiểm tra đỏ.
- Thư viện bên thứ ba ra phiên bản lớn mới **không** buộc Autoxide ra bản major, và không buộc
  module phải sửa code. Module chạm database, HTTP, OpenAPI, kiểm đầu vào, migration và thời gian qua
  các lớp riêng của kernel (`autoxide_kernel::db`, `http`, `migration`, `time`…), và phần công khai
  hạng ổn định của kernel và testkit không có kiểu nào của các thư viện nằm dưới các lớp đó
  (`sea-orm`, `sea-query`, `sqlx`, `axum`, `http`, `tower`, `utoipa`, `validator`, `jiff`,
  `async-trait`). Một trong số đó lên phiên bản lớn mới thì chỉ kernel sửa. Chỉ nhóm nhỏ thư viện
  *từ vựng* (`serde`, `serde_json`, `uuid`, `tokio`, `tracing`, `futures-core`, `bytes`) được xuất
  hiện trong hợp đồng; một trong số đó lên phiên bản lớn mới (hiếm: `serde` 2) là thay đổi thư viện
  duy nhất tính là phá vỡ. Bảng ở
  [`docs/MODULE_GUIDE.vi.md`](MODULE_GUIDE.vi.md#độ-ổn-định-của-hợp-đồng-module).
- Tới 1.0, các module đang được chuyển sang các lớp đó và đường cũ vẫn còn; `cargo xtask check` (và
  `check-module` cho module ở repo riêng) chặn module phụ thuộc thẳng vào thư viện kernel đã bọc, trừ
  các module còn trong danh sách chờ chuyển. 1.0 chỉ phát hành khi mọi module đã chuyển xong.
- Phiên bản Rust tối thiểu theo bản Rust ổn định gần đây (`rust-toolchain.toml`). Nâng nó chỉ ở bản
  minor, không bao giờ ở bản patch, và có ghi trong changelog.

### SDK TypeScript

SDK (`sdk/typescript`) sinh từ `docs/openapi.json` và mang cùng số phiên bản. Nó theo luật của REST:
bản minor thêm method và trường tuỳ chọn; method hay kiểu chỉ biến mất ở bản major. Endpoint
deprecated vẫn ở trong SDK tới ngày sunset.

## Phần không cam kết

Những thứ sau có thể đổi ở bất kỳ bản nào, kể cả bản patch khi đó là sửa lỗi:

- **Module thử nghiệm** (hạng `experimental` trong [`docs/MODULES.vi.md`](MODULES.vi.md), tắt theo
  mặc định): endpoint, tool, event, cấu hình và Rust API của chúng có thể đổi giữa các bản minor, kể cả
  thay đổi phá vỡ. Mọi thay đổi đều có trong changelog, và việc chuyển dữ liệu của chúng sang bản mới
  được mô tả trong ghi chú nâng cấp khi cần làm tay.
- Thứ được đánh dấu **experimental** ở chỗ khác: operation OpenAPI có `"x-stability":
  "experimental"`, tool MCP có `"stability": "experimental"`, và tính năng mà tài liệu ghi
  "experimental".
- Phần Rust không công khai: `pub(crate)` và thứ riêng tư, thứ có `#[doc(hidden)]`, và mọi thứ trong
  crate module ngoài `api` và `module(…)`. Module mẫu `notes` và crate `xtask` hoàn toàn không thuộc
  hợp đồng.
- Dòng log: định dạng, câu và trường. Tên và nhãn của số đo (metric) được ghi trong changelog khi đổi
  nhưng không bị đóng băng.
- Chữ cho người đọc: `message` của lỗi, bản dịch, mô tả trong OpenAPI và MCP, đầu ra CLI không có
  `--json`.
- Bảng, cột và index của database (xem trên), thứ tự phần tử ở chỗ API không hứa thứ tự, và số đo
  hiệu năng.

## Build từ mã nguồn

### Phiên bản Rust tối thiểu (MSRV)

MSRV là `rust-version` trong `Cargo.toml` gốc của workspace (hiện là bản phát hành liền trước toolchain được ghim trong
`rust-toolchain.toml`). Chính sách:

- Repo build được và các crate thư viện biên dịch được với MSRV; CI có một job build workspace bằng đúng phiên bản đó,
  nên một tính năng ngôn ngữ mới không lọt vào mà không ai biết.
- Toolchain được ghim (`rust-toolchain.toml`, dùng bởi người phát triển, CI và image Docker) mới hơn MSRV và đi theo Rust
  stable; MSRV chỉ được nâng khi một dependency hay một tính năng cần, không bao giờ trong bản patch, và việc nâng được
  ghi trong changelog.
- Module ở repo khác cần ít nhất MSRV. Ví dụ module ngoài dùng cùng `rust-version` (`cargo xtask check` kiểm).

### Cargo feature

Bản build mặc định có đủ mọi feature mà một máy chủ production cần. Các feature tuỳ chọn:

| Crate                | Feature            | Làm gì                                                              | Trong bản phát hành | Hợp đồng |
| -------------------- | ------------------ | ------------------------------------------------------------------- | ------------------- | -------- |
| `autoxide`           | `avif`             | Ảnh thu nhỏ AVIF (`AUTOXIDE_MEDIA_AVIF`); kéo bộ mã hoá `rav1e`      | Tắt                 | Ổn định  |
| `autoxide`           | `bench-seed`       | `task <module>.bench_seed` sinh dữ liệu mẫu để đo                    | Tắt                 | Không    |
| `autoxide-kernel`    | `permission-audit` | Chỉ cho test: dựng route không có lớp permission để rà soát          | Tắt                 | Không    |
| các crate module     | `testing`          | Đồ phụ trợ test module dùng chung với `autoxide-testkit`             | Tắt                 | Không    |
| các crate module     | `bench-seed`, `avif` | Chuyển tiếp từ các feature của `autoxide` ở trên                   | Tắt                 | Như trên |

"Không" nghĩa là nằm ngoài cam kết tương thích: không bao giờ bật trong ứng dụng. Feature chỉ cộng thêm; feature mới
không bao giờ đổi bản build mặc định.

### Build tái lập

Image phát hành được build với mục tiêu cùng mã nguồn cho ra cùng binary. Đây là mục tiêu: chưa có test tự động nào build hai lần
rồi so sánh, nên đừng dựa vào việc giống từng byte. Những gì đã có:

- `Cargo.lock` được commit và mọi bản build phát hành dùng `--locked`; toolchain được ghim (`rust-toolchain.toml`,
  `RUST_VERSION` của Docker) và image nền ghim theo digest.
- Dockerfile build trong một thư mục cố định và truyền `--remap-path-prefix` để không đường dẫn build nào lọt vào
  binary; `SOURCE_DATE_EPOCH` (thời điểm commit của bản phát hành) dùng cho mọi mốc thời gian, kể cả file nén phát hành
  (xếp theo tên, chủ sở hữu và thời gian sửa cố định).
- Để so một bản phát hành với bản build của bạn: build lại từ tag bằng cùng Dockerfile, chép `/usr/local/bin/autoxide` ra khỏi
  image của bạn, giải nén file nén phát hành (`tar -xzf autoxide-X.Y.Z-<target>.tar.gz`) rồi so `sha256sum` của hai file
  `autoxide`. (`SHA256SUMS` liệt kê các file nén, không phải binary bên trong.) Khác nhau chưa phải bằng chứng bị sửa đổi: digest
  image nền hay kiến trúc đích khác cũng làm binary khác. [`docs/VERIFYING.vi.md`](VERIFYING.vi.md) chỉ cách kiểm chữ ký và
  attestation, là cách kiểm được hỗ trợ.

### Bảng kê vật liệu phần mềm (SBOM)

Mỗi bản phát hành kèm hai danh sách những gì nằm bên trong:

- **Crate Rust trong binary.** Binary được build bằng `cargo auditable`, công cụ nhúng đúng danh sách dependency vào file
  thực thi. Các công cụ như `cargo audit bin autoxide`, Syft, Trivy hay Grype đọc được, nên máy quét lỗ hổng thấy các crate
  Rust kể cả trong binary đã strip.
- **File CycloneDX.** `autoxide.cdx.json` (CycloneDX 1.5, sinh từ `Cargo.lock` bằng `cargo cyclonedx`) được đính kèm
  GitHub Release và có attestation; image còn mang một SBOM SPDX (`sbom.spdx.json`).

## Hỗ trợ

Từ 1.0 mỗi phiên bản major có hỗ trợ dài hạn:

- Bản minor mới nhất nhận mọi bản sửa; bản minor trước nhận vá bảo mật **3 tháng** sau khi bản minor kế tiếp ra; khi có
  bản major mới, bản minor cuối của major trước nhận vá bảo mật **6 tháng**. Bản cũ hơn không được vá. Bảng ở
  [`SECURITY.vi.md`](../SECURITY.vi.md#phiên-bản-được-vá).
- **Vá ngược.** Bản sửa làm trên `main` trước. Với dòng cũ còn được hỗ trợ, người duy trì cherry-pick nó vào nhánh
  `release/X.Y` cắt từ tag `vX.Y.Z` mới nhất của dòng đó, phát hành thành bản patch kế tiếp (`X.Y.Z+1`) chỉ chứa bản sửa
  đó, sau khi chạy gate đầy đủ trên cả hai database. Bản patch không bao giờ chứa tính năng mới, cài đặt mới bắt buộc
  hay thay đổi hợp đồng, nên cài riêng được an toàn.
- Tương thích **database** giữa các phiên bản (nâng từ bản nào được, bước tay nào cần) ghi trong changelog của từng bản
  phát hành.
Cách phát hành ghi ở [`docs/RELEASING.md`](RELEASING.md).
