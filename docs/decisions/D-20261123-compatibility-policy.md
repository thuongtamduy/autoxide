## D-20261123-compatibility-policy · Bản 1.0 có cam kết: chính sách tương thích, `cargo-semver-checks` trên PR, dòng hỗ trợ dài hạn — 2026-11-23 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định (bước 70 P4):

- **Chính sách tương thích** cho người ngoài ở `docs/COMPATIBILITY.md` + `.vi.md`, áp từ `1.0.0`, cho module hạng
  core và official: REST + OpenAPI (không phá vỡ `/api/v1`; endpoint mới thay vì sửa, endpoint cũ khai qua
  `Module::deprecations` với sunset **ít nhất sáu tháng**, không bỏ ở bản patch — bổ sung D-059), mã lỗi `code`
  (không đổi tên, không dùng lại; được thêm mã mới), biến cấu hình (đổi tên thì tên cũ chạy tiếp kèm cảnh báo ít
  nhất một bản minor và sáu tháng, bỏ ở bản major; mặc định đổi hành vi chỉ ở major trừ vá bảo mật), migration
  (không sửa migration đã phát hành; nâng thẳng từ mọi 1.x; hạ cấp bằng `down` của binary mới), tool MCP, event và
  webhook (chỉ thêm trường; đổi hình dạng thì tên mới), CLI có trong tài liệu và đầu ra `--json`, Rust API
  (kernel, testkit, `autoxide::Autoxide`, `api` + `module(…)` của module core / official: phá vỡ chỉ ở major,
  `#[deprecated]` trước ít nhất một minor; thư viện lộ trong hợp đồng lên major = phá vỡ; nâng Rust tối thiểu chỉ ở
  minor), SDK TypeScript. Không cam kết: module thử nghiệm, thứ đánh dấu experimental, phần không công khai,
  `#[doc(hidden)]`, `notes`, `xtask`, log, chữ cho người đọc, bảng database, metric (ghi changelog nhưng không đóng
  băng). Bảng thư viện của `docs/MODULE_GUIDE.md` sửa theo: "Rust tối thiểu mới hơn" từ 1.0 không còn là phá vỡ.
- **`cargo-semver-checks` trên mọi crate hợp đồng**: `cargo xtask semver` mở rộng từ kernel + testkit (D-242) sang
  `autoxide` (thư viện) và crate của module core / official — danh sách đọc từ bảng sinh tự động giữa
  `<!-- modules:begin/end -->` của `docs/MODULES.md`, nên đổi hạng module thì danh sách tự đổi. Thêm `--changed
  <rev>`: chỉ kiểm crate có file thuộc phần công khai đổi (kernel, testkit cả thư mục; `crates/app/src/lib.rs`;
  `modules/<m>/src/{lib.rs,api.rs,api/}`). Job `semver` của CI chạy ở PR phát hành (mọi crate, so với tag trước)
  và ở PR chạm phần công khai (chỉ crate bị chạm, so với nhánh gốc của PR); job riêng, cài công cụ bằng
  `taiki-e/install-action` ghim SHA, không chạm job gate. Trước 1.0 chỉ cảnh báo (`::warning` + tóm tắt job), so
  như bản patch; từ 1.0 `cargo xtask semver` tự thất bại khi có phá vỡ và so như bản minor (thêm mới,
  `#[deprecated]` hợp lệ); PR phát hành tăng major so với tag (`2.0.0` vs `v1.x`) chỉ báo cáo. Bật chặn thật = thêm
  check vào branch protection lúc phát hành 1.0 (chủ dự án làm, ghi ở `docs/RELEASING.md`).
- **Dòng hỗ trợ dài hạn** (`SECURITY.md` + `.vi.md`, `docs/RELEASING.md`): trước 1.0 chỉ minor mới nhất; từ 1.0
  minor mới nhất nhận mọi bản sửa, minor trước nhận vá bảo mật **3 tháng** sau khi minor mới ra, minor cuối của
  major trước nhận vá bảo mật **6 tháng** sau major mới. Ghi rõ dự án cộng đồng, mục tiêu chứ không SLA. Vá dòng cũ
  bằng nhánh `release/X.Y` + tag đẩy tay.

Lý do: người ngoài chỉ dám xây trên core khi biết thứ gì giữ nguyên qua nâng cấp; mỗi loại hợp đồng có một luật đổi
cụ thể để agent sau khỏi tự suy. Đọc danh sách module từ `docs/MODULES.md` thay vì viết cứng: bảng đó đã có test
giữ khớp code, và hạng module chính là ranh giới cam kết. Kiểm trên PR chạm phần công khai (thay vì chỉ lúc phát
hành) báo phá vỡ cho đúng người gây ra, khi còn rẻ để sửa; chỉ kiểm crate bị chạm vì kiểm cả ~30 crate mất cỡ một
giờ. So như minor từ 1.0 để `#[deprecated]` (bước bắt buộc trước khi bỏ) không bị báo nhầm. Thời hạn 3/6 tháng
vừa sức một nhóm nhỏ mà vẫn cho người dùng thời gian nâng cấp.

Đã cân nhắc: kiểm mọi crate ở mọi PR — bỏ, quá lâu; chạy trong `./gate.sh` — bỏ, cần build bản cũ và mạng (như
D-242); cài công cụ bằng `cargo install` — bỏ, vài phút mỗi lần, action có bản dựng sẵn; hỗ trợ mọi minor 1.x —
bỏ, vượt sức nhóm nhỏ; cơ chế tên cũ cho biến cấu hình làm ngay — để lúc có lần đổi tên đầu tiên sau 1.0 (hiện chưa
có biến nào cần đổi; `Setting` thêm cách khai tên cũ khi đó, theo D-038 không phá vỡ).

Thay: phần "chỉ kernel và testkit" và "chỉ chạy ở PR phát hành" của D-242
