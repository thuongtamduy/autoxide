## D-20261127-ci-runner-budget · CI giữ ít máy hơn: job `plan` quyết một lần, shard test theo số test được chọn, `publish-check` trên PR phát hành — 2026-11-27 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

- `ci.yml` có job `plan` quyết một lần phần nào cần chạy: test nâng cấp, dựng thử image, MCP OAuth, luật cảnh báo, MSRV,
  chế độ `semver` (thay job `semver (plan)`) và "PR phát hành". Job không cần chạy bị bỏ qua ở mức JOB (`if:`): không xin
  máy, check báo "skipped" (được tính là đạt). Mẫu file giữ như cũ, mở rộng: ảnh hưởng tới image thêm `ci.yml`; test nâng
  cấp thêm mọi file `.rs` đã đổi có `derive(Record)` hay `DeriveEntityModel` (trước hoặc sau thay đổi) — bảng khai ngoài
  thư mục `entity` (vd `modules/auth/src/api_key/listing.rs`) trước đây lọt mẫu.
- **Push `main` và chạy tay vẫn LUÔN chạy test nâng cấp và dựng thử image** (D-20261121-ci-batch-release-faster mục 3,
  chủ dự án quyết; image dựng bản release từ mọi mã Rust nên mẫu file không đủ); chúng chạy song song với gate nên không
  làm lâu thêm lần chạy. Mẫu chỉ áp cho pull request, như trước.
- MCP OAuth, luật cảnh báo, MSRV ở push `main` cũng theo mẫu, **so với commit của lần CI xanh gần nhất trên `main`** (không
  phải commit liền trước, để thay đổi của lần chạy bị huỷ hay đỏ vẫn được tính); không có mốc (chưa có lần xanh, commit
  đó không còn là tổ tiên) thì chạy hết; chạy tay luôn chạy hết. Chạy lại cả workflow của một commit đã xanh thì mốc là
  chính HEAD và ba job này bị bỏ qua (kết quả xanh của chúng đã có).
- Gate (`gate.yml`) không đổi phạm vi: push `main`, tag, chạy tay vẫn chạy gate đầy đủ hai database, chia cố định
  SQLite 4 + Postgres 6 (+6 bản 18). Pull request: `./gate.sh --pr --only build` ghi thêm `tests.json` (`cargo nextest
  list` với filterset của PR), `cargo xtask test-shards` tính số shard theo chi phí ước lượng của test được chọn, tối đa
  như gate đầy đủ; không build hay không test nào được chọn thì job `tests` bỏ qua ở mức job (trước đây 10 máy vẫn mở
  chỉ để in một dòng). Chỉ đổi SỐ máy: `--partition count:k/n` vẫn chạy đủ mọi test đã chọn.
- Pull request: danh sách test JSON lạ (nextest đổi định dạng) hay không test nào khớp một filterset khác `none()` làm job
  `build tests` đỏ (fail-closed), không bao giờ thành matrix rỗng.
- Job `crates.io packaging (publish-check)` chạy `cargo xtask publish-check` trên PR phát hành (đổi `version`) và khi chạy
  tay CI với `publish-check` bật, song song với mọi job khác, thay bước chạy tay ~85 phút ở docs/RELEASING.md bước 5.
  auto-release.yml không kiểm job này (chỉ xem CI của push `main`): chờ nó xanh rồi mới merge PR phát hành.

Lý do: đo 20 lần chạy CI ngày 2026-11-27 (12 PR, 8 push `main`; `gh api …/runs/<id>/jobs`): mỗi lần chạy chiếm 110–240
phút-máy; giờ cao điểm job chờ máy tới 22–40 phút (lần 45977434981, 45981700243, 45983003020) vì gói miễn phí cho ~20 job
cùng lúc và mỗi PR mở ~25–28 job. Push `main` (~170–200 phút-máy) luôn dựng image release 23–34 phút, test nâng cấp 2 × 7–10
phút và MCP OAuth 6–10 phút dù thay đổi không chạm thứ đó. Hai job đầu giữ (quyết định của chủ dự án; và image release
bắt lỗi mà mẫu file không thấy); MCP OAuth, luật cảnh báo và MSRV (~10–15 phút-máy) chỉ kiểm phần mẫu của chúng mô tả và PR
của chính thay đổi đã kiểm theo cùng mẫu. Shard test: mỗi máy tải archive 1,6 GB (1–2,5 phút) rồi giải nén trước khi chạy; test của `autoxide`
chiếm phần lớn thời gian (trung bình 11,5 giây/test trên SQLite, 37 giây trên Postgres, crate khác 0,6 và 3,4 giây), nên
PR chạm một module (module + `autoxide`) cần ~7 máy chứ không 10, PR chỉ chạm công cụ cần 2. Commit phát hành luôn đổi
`Cargo.toml` và `Cargo.lock`, nên MCP OAuth và MSRV vẫn chạy trên đúng commit được phát hành (auto-release.yml dùng lại CI
xanh của commit đó, D-20261121-auto-release); luật cảnh báo chỉ chạy khi `deploy/prometheus` đổi — kết quả chỉ phụ thuộc
thư mục đó, đã kiểm ở lần đổi. D-300 không đổi.

Check bắt buộc: ruleset của `main` (chủ dự án kiểm ngày 2026-11-28) không bật "Require status checks to pass" — chỉ "Require
a pull request before merging" và "Block force pushes" — nên đổi số shard hay tên check của PR không làm PR treo; job gộp
`gate (PostgreSQL …)` vẫn là tên ổn định nếu sau này bật.

Đã cân nhắc:
- Build archive chỉ các crate bị ảnh hưởng (`-p`): cargo hợp feature theo các crate được chọn, build lại sqlx, sea-orm…
  thành bộ thứ hai và mất cache (đã đo ở `xtask/src/affected.rs`); `autoxide` lại phụ thuộc mọi module. Loại.
- Cache không phụ thuộc số phiên bản workspace: thư viện ngoài đã được rust-cache khôi phục theo tiền tố khoá khi đổi
  phiên bản; crate của workspace phải biên dịch lại vì phiên bản nằm trong `-C metadata` và `CARGO_PKG_VERSION` — không
  tránh được mà không đổi tính đúng. Không làm gì thêm.
- Gộp các job nhỏ (workflows, secrets, dco, reuse…) thành một: mỗi job dưới 1 phút, lợi ít mà đổi tên check. Loại.
- Cho test nâng cấp và dựng thử image theo mẫu cả ở push `main` (~45 phút-máy mỗi lần): đảo một phần quyết định của chủ
  dự án, và mẫu file không thấy hết thay đổi ảnh hưởng tới bản release. Loại.
- Chia `publish-check` nhiều máy: `cargo package --workspace` cần mọi crate nội bộ trong cùng registry tạm. Một máy,
  giới hạn 180 phút.

Thay: phần "push main luôn chạy" của MCP OAuth (D-20261127-mcp-sdk-ci-check); mục 2 của D-20261125-ci-skip-rust (push
`main` nay dùng phân loại "chạm Rust" cho MSRV, so với lần CI xanh gần nhất; gate vẫn luôn chạy đủ); phần "tên check giữ
nguyên" của D-20261120-release-gate-sharded khi áp cho pull request (tên shard của PR theo số máy, vd "gate / sqlite 1/3";
gate đầy đủ giữ tên cũ; check ổn định là job gộp `gate (PostgreSQL …)`)
