# Đóng góp cho Autoxide

[English](CONTRIBUTING.md) · **Tiếng Việt**

Cảm ơn bạn đã giúp. Hướng dẫn này dành cho người; nếu bạn làm cùng một AI agent viết code,
hãy trỏ nó tới [`CLAUDE.md`](CLAUDE.md) (bản tiếng Anh ngắn: [`AGENTS.md`](AGENTS.md)), nơi ghi đầy đủ luật agent
làm theo. Luật dưới đây là cùng luật đó, chỉ ngắn hơn. Mọi người tham gia đều theo
[quy tắc ứng xử](CODE_OF_CONDUCT.vi.md).

## Trước khi bắt đầu

- **Lỗi**: mở issue kèm phiên bản, database (SQLite hay PostgreSQL) và các bước tái hiện.
- **Vấn đề bảo mật**: đừng mở issue; làm theo [`SECURITY.vi.md`](SECURITY.vi.md).
- **Tính năng mới**: mở issue trước và mô tả vấn đề bạn muốn giải. Core giữ trung lập với
  nghiệp vụ (không có đơn hàng, kho hay bệnh nhân trong core — những thứ đó thuộc về module),
  và thứ tự công việc theo [`docs/ROADMAP.md`](docs/ROADMAP.md), nên trao đổi ngắn trước
  giúp bạn khỏi viết code không merge được.

## Chuẩn bị máy

Bạn cần:

- [Rust](https://rustup.rs) — `rustup` tự cài đúng toolchain ghim trong
  `rust-toolchain.toml` ở lần chạy `cargo` đầu tiên;
- một trình biên dịch C (SQLite được biên dịch từ mã nguồn);
- [Docker](https://docs.docker.com/get-docker/), cho test PostgreSQL và S3 của gate đầy đủ;
- [`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny):
  `cargo install cargo-deny --locked`;
- tuỳ chọn [`gitleaks`](https://github.com/gitleaks/gitleaks) (không có thì gate chạy nó
  trong Docker).

Trên máy Linux x86_64 mới (ví dụ phiên agent trên cloud) chạy `scripts/cloud-setup.sh` thay vì cài tay: script
cài toolchain ghim cùng `cargo-deny`, `gitleaks` và `cargo-nextest` từ bản phát hành dựng sẵn, kiểm SHA-256 đã ghim.
Chạy lại nhiều lần được.

Rồi:

```bash
git clone https://github.com/thuongtamduy/autoxide.git autoxide
cd autoxide
git config core.hooksPath .githooks
cargo test --workspace
```

Git hook kiểm định dạng và quét secret trong thay đổi đã stage trước mỗi commit.

## Kiểm thay đổi: hai mức gate

Gate là định nghĩa của "xong". Có hai mức:

```bash
./gate.sh --quick
```

Trong lúc làm, trước mỗi commit: định dạng, lint, kiểm ranh giới, thư viện, secret, và test
của các crate bạn đã sửa (cùng các crate phụ thuộc vào chúng), chỉ trên SQLite. Phần test
không cần Docker và thường mất một hai phút.

```bash
./gate.sh
```

Trước khi mở pull request: mọi thứ ở trên, và mọi test trên **cả SQLite và PostgreSQL**
(gate tự dựng container Docker dùng xong bỏ). Một thay đổi chỉ được merge khi gate này xanh.

Gate đỏ thì sửa code. Đừng nới gate, tắt lint hay bỏ qua test.

Thay đổi chỉ chạm file có trong [`scripts/rust-free-paths.txt`](scripts/rust-free-paths.txt) (tài liệu, SDK
TypeScript, file mảnh changelog, quyết định) không thể đổi kết quả Rust nào: `--quick` bỏ Clippy, và ở pull request CI
bỏ phần biên dịch và test Rust (các job vẫn báo xanh). File không có trong danh sách thì chạy đủ.

## Những luật quan trọng nhất

- **Cả hai database.** Mọi thứ chạm tới database — migration, query, transaction, index,
  test — phải chạy trên SQLite và PostgreSQL. Tính năng chỉ chạy trên một bên là chưa xong.
  Đừng dùng kiểu dữ liệu, hàm hay cú pháp chỉ một bên có.
- **Ranh giới module.** Mỗi module là một crate riêng trong `modules/` và sở hữu bảng của
  mình (tên `<module>_…`). Module không bao giờ đọc, ghi hay join bảng của module khác, không
  có foreign key tới chúng; nó chỉ gọi module khác qua trait được công bố trong `pub mod api`.
  `cargo xtask check` (một phần của gate) kiểm điều này.
- **Logic ở service.** Handler HTTP và tool MCP chỉ nhận dữ liệu vào, gọi service rồi trả
  response; REST và MCP dùng chung một service.
- **Lỗi có `code` ổn định.** Mọi lỗi trả cho client là `autoxide_kernel::error::Error` với
  mã viết `snake_case` (lỗi của module bắt đầu bằng `<module>.`) và thông báo nói rõ phải sửa
  gì. Mã là hợp đồng với client: đổi mã là thay đổi phá vỡ. Mỗi mã mới cần bản dịch tiếng
  Việt trong `locales/vi.json` của crate.
- **Hợp đồng API luôn khớp.** Mọi route có tài liệu OpenAPI. Đổi API hay cấu hình thì chạy
  `cargo xtask regen`: một lệnh sinh lại mọi file tự động (`docs/openapi.json`, `docs/mcp-tools.json`,
  `.env.example`, schema SDK TypeScript). Gate trên máy kiểm chúng sớm (`cargo xtask regen --check`, chỉ chạy trên máy; CI do các test giữ tài liệu khớp code kiểm) và in
  đúng lệnh này khi có file lệch.
  Không đổi hành vi của endpoint `/api/v1` đã có theo cách phá vỡ; làm endpoint mới và đánh
  dấu endpoint cũ là deprecated.
- **Test.** Code mới đi kèm test. Test chạm database dùng `autoxide_testkit::TestDatabase`,
  mỗi test nhận một database trống trên SQLite hay PostgreSQL tuỳ gate đang chạy bên nào.
- **Không `unsafe`, không `unwrap` / `expect` / `panic!` ngoài test.** Trả lỗi thay vào đó.
- **Không secret trong Git.** Chỉ commit `.env.example`, không bao giờ `.env`.
- **Tài liệu hai ngôn ngữ.** Tài liệu cho người dùng (`README`, hướng dẫn trong `docs/`, file
  này) có bản tiếng Anh và bản tiếng Việt (`*.vi.md`) cùng nội dung: sửa cả hai trong cùng
  một commit. Tài liệu nội bộ (`CLAUDE.md`, `STACK.md`, `docs/ROADMAP.md`,
  `docs/DECISIONS.md`) và comment trong code được viết tiếng Việt.

Danh sách đầy đủ, kèm lý do của từng luật, ở [`CLAUDE.md`](CLAUDE.md); các quyết định đã
chốt có mục lục theo chủ đề ở `docs/decisions/INDEX.md`; file này sinh tự động, không commit:
chạy `cargo xtask decisions` để tạo.

## Commit và pull request

- Một pull request chỉ làm một thay đổi. Tách các thay đổi không liên quan.
- Với mọi thứ người dùng Autoxide sẽ thấy, thêm một file nhỏ vào
  [`changelog.d/`](changelog.d/README.md) thay vì sửa `CHANGELOG.md`: đặt tên
  `<tên>.<mục>.md` (`added`, `changed`, `deprecated`, `removed`, `fixed` hoặc `security`) và viết các gạch đầu dòng
  tiếng Anh đúng như sẽ hiện trong changelog. Chúng được gộp vào `CHANGELOG.md` lúc phát hành, nên các pull request
  song song không xung đột ở đó.
- Trong pull request, nói bạn đã đổi gì, vì sao, và xác nhận `./gate.sh --quick` xanh (CI chạy phần còn lại, gồm
  PostgreSQL; `./gate.sh` đầy đủ bắt buộc trước khi phát hành).

### Commit message

Commit message theo một dạng nhẹ của [Conventional Commits](https://www.conventionalcommits.org):

```text
<type>(<scope>): <tóm tắt ngắn ở thể mệnh lệnh, tối đa 72 ký tự>

<thân tuỳ chọn: vì sao cần đổi, đã cân nhắc gì, thay đổi không làm gì>

Signed-off-by: Tên Của Bạn <ban@example.com>
```

- `type` là một trong `feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `revert`.
  `scope` không bắt buộc: tên module hay crate (`orders`, `kernel`).
- Thay đổi phá vỡ đánh dấu bằng `!` sau scope (`feat(kernel)!: …`) và giải thích trong thân. Thay đổi phá vỡ của hạng hợp
  đồng ổn định theo [`docs/COMPATIBILITY.vi.md`](docs/COMPATIBILITY.vi.md).
- Viết bằng tiếng Anh hoặc tiếng Việt. Giải thích *vì sao* trong thân khi không hiển nhiên.
- Tiêu đề pull request được dùng làm tiêu đề squash-merge, nên theo cùng dạng.

### Ký commit (DCO)

Autoxide dùng [Developer Certificate of Origin](DCO) (DCO) thay cho thoả thuận cấp phép của người đóng góp. Thêm dòng
`Signed-off-by` nghĩa là bạn cam đoan mình viết thay đổi đó hoặc có quyền gửi nó theo license của dự án. Thêm bằng `-s`:

```bash
git commit -s -m "fix(notes): reject empty titles"
```

Để ký các commit đã làm: `git rebase --signoff origin/main` rồi push lại với `--force-with-lease`. Job `DCO sign-off`
trong CI đỏ khi một commit của pull request không có dòng `Signed-off-by` (khi nhánh mà pull request nhắm tới chưa có `.github/CODEOWNERS` — như nhánh `release/X.Y` cắt từ tag cũ — thì không kiểm gì; commit merge không được kiểm, nên ưu tiên
rebase hơn là gộp `main` vào nhánh của bạn). Pull request do người duy trì mở
(login có trong `.github/CODEOWNERS` của nhánh mà pull request nhắm tới, không phải của pull request; GitHub xác thực login) được miễn, kể cả commit do phiên agent của họ viết: người
duy trì là bên cấp license và chịu trách nhiệm về những gì họ đưa vào.

### Đóng góp có AI hỗ trợ

Bản thân Autoxide được xây bằng AI agent ([`CLAUDE.md`](CLAUDE.md)), nên đóng góp có AI hỗ trợ được chào đón. Các luật này
để luôn có một người chịu trách nhiệm và để gánh nặng review công bằng:

- **Nói rõ.** Ghi trong mô tả pull request công cụ nào đã giúp và giúp việc gì. Thêm trailer như
  `Assisted-by: <công cụ hay mô hình>` vào commit nếu công cụ viết phần đáng kể.
- **Có một người chịu trách nhiệm.** Dòng `Signed-off-by` là của bạn, không phải của công cụ. Bạn đã đọc từng dòng, hiểu và
  giải thích được khi review, và tự chạy `./gate.sh --quick`.
- **Kiểm những gì công cụ bịa.** Xác nhận mọi dependency, hàm, API có thật ở đúng phiên bản đang ghim (skill
  `check-library-api` của repo chỉ cách làm). Gỡ tham chiếu bịa, và không bao giờ dán secret, dữ liệu cá nhân hay dữ
  liệu khách hàng vào công cụ.
- **Không gửi thứ bạn không bảo vệ được.** Đầu ra hàng loạt chưa review, nhiều thay đổi không liên quan trong một pull
  request, hay câu trả lời review chỉ là đầu ra của công cụ có thể bị đóng mà không review chi tiết.
- **License.** Không gửi đầu ra mà bạn biết là sao chép mã theo license không tương thích.

### Nhãn và nên bắt đầu từ đâu

Người duy trì gắn nhãn cho issue để người mới tìm được việc đầu tiên: `good first issue` (nhỏ, mô tả rõ, có người duy
trì giúp), `help wanted` (chúng tôi muốn có đóng góp, mọi cỡ) và các nhãn còn lại ở
[`GOVERNANCE.vi.md`](GOVERNANCE.vi.md#nhãn-issue-và-pull-request). Bình luận vào issue trước khi bắt đầu để hai người
không làm trùng việc.

## License của phần đóng góp

Autoxide dùng license Apache, phiên bản 2.0, hoặc license MIT, tuỳ người dùng chọn. Trừ khi
bạn nói rõ khác đi, mọi đóng góp bạn chủ ý gửi để đưa vào Autoxide, theo định nghĩa trong
license Apache-2.0, được cấp phép theo cả hai license như trên, không kèm điều khoản hay điều
kiện nào khác.
