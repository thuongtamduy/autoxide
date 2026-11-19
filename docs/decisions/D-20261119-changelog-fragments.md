## D-20261119-changelog-fragments · Thay đổi cho changelog ghi bằng file mảnh trong `changelog.d/`, gộp lúc phát hành — 2026-11-19 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: PR không sửa `CHANGELOG.md` nữa. Thay đổi người dùng thấy ghi thành một file mảnh
`changelog.d/<tên>.<mục>.md` (`<tên>`: chữ thường, số, `-`; `<mục>`: `added`, `changed`, `deprecated`, `removed`,
`fixed`, `security` theo Keep a Changelog), nội dung là các gạch đầu dòng tiếng Anh đúng như sẽ nằm trong changelog.
`cargo xtask check` (trong gate) chặn tên sai, mục lạ, file rỗng, dòng không phải gạch đầu dòng, tiêu đề trong mảnh.
Lúc phát hành, bước 3 của `docs/RELEASING.md` chạy `cargo xtask changelog`: gộp mọi mảnh (theo thứ tự tên file) vào
cuối mục tương ứng của `## [Unreleased]`, thêm mục còn thiếu đúng thứ tự, rồi xoá mảnh. Dòng đã có trong
`[Unreleased]` giữ nguyên. Hướng dẫn cho người ngoài ở `changelog.d/README.md`, `CONTRIBUTING.md` + `.vi.md`, mẫu PR.

Lý do: ngày 19/11/2026 có tới 8–10 phiên agent merge song song; gần như mọi PR đều xung đột ở `CHANGELOG.md` vì ai
cũng chèn vào đầu `### Added`, mỗi lần phải gộp lại và chạy lại CI (~20 phút). File mảnh là cách chuẩn của các dự án
lớn (towncrier, changesets, `changelog.d` của GitLab / Rust): hai PR không bao giờ sửa cùng một file. Tự viết trong
`xtask` (~200 dòng, có test) thay vì thêm công cụ ngoài (towncrier cần Python, changesets cần Node).

Đã cân nhắc: thêm dòng ở CUỐI `### Added` (giảm nhưng không hết xung đột khi hai PR cùng thêm ở cuối);
`merge=union` trong `.gitattributes` (nút merge của GitHub không dùng driver này, và dễ lặp dòng).
