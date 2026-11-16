# Bản vẽ thiết kế giao diện

Bản sao các bản vẽ thiết kế giao diện của Autoxide, lấy từ hai artifact "Design" của chủ dự án ngày
16/11/2026 (D-301). Đây là tài liệu tham khảo cho agent làm các bước có màn hình, **không phải giao diện
chạy được**: core chỉ có API (luật kiến trúc 9), giao diện thật là repo riêng (D-096, D-097). Nội dung
nghĩa của thiết kế, giao diện cần gì từ core, nằm ở [`../research/ui-design.md`](../research/ui-design.md);
đối chiếu với lộ trình ở [`../research/ui-traceability.md`](../research/ui-traceability.md).

| Thư mục  | Artifact                                  | Phiên bản         | Nội dung                                         |
| -------- | ----------------------------------------- | ----------------- | ------------------------------------------------ |
| `admin/` | Autoxide Admin — thiết kế quản trị        | `1794717563-0816` | 36 màn, `canvas.json`, CSS, `ax-shell.js`, 6 ảnh |
| `web/`   | Autoxide Web — website công khai          | `1794640016-e7e2` | 18 màn, `canvas.json`, CSS, 10 ảnh               |

Mỗi thư mục có:

- `project/` — mọi file của bản vẽ: mỗi màn một file `.dc.html`; `canvas.json` ghi vị trí và tiêu đề
  từng màn trên canvas (tiêu đề có số thứ tự màn, vd `25 · Người dùng và quyền`). Màn hiện tại dùng
  `ax-r40.css` + `ax-shell.js` (quản trị) và `shop-r38.css`, `site.css` (website); các file
  `*-rNN.css` cũ hơn là bản lưu của những lần sửa trước, giữ nguyên để khôi phục đúng.
- `_blob/` — ảnh SVG đã tải lên artifact; tên file là mã ảnh, các màn trỏ tới `/_blob/<mã>`.

Dữ liệu trong màn (tên người, email `@example.com`, `@bepmoc.vn`, số tiền…) là dữ liệu mẫu bịa ra để
vẽ, không phải dữ liệu thật.

## Xem và sửa

Các file `.dc.html` cần khung của loại artifact "Design" mới hiện đúng; mở trực tiếp trong trình duyệt
sẽ thiếu khung và ảnh. Khung này (`artifact-type/`, `index.html`, `SKILL.md`) giống nhau ở mọi artifact
Design, do nền tảng cấp, nên không chép vào đây.

Để khôi phục hay dựng lại một artifact từ bản này: tạo một artifact Design mới, đăng lại các file trong
`project/`, tải ảnh trong `_blob/` lên, rồi thay mã ảnh cũ bằng mã mới trong các màn (ảnh tải lên lại sẽ
có mã khác).

## Cập nhật

Bản vẽ gốc vẫn ở artifact; thư mục này là bản chụp tại một thời điểm. Sửa thiết kế xong thì chép lại cả
thư mục `project/` và `_blob/` của artifact tương ứng, đè lên bản cũ, ghi phiên bản mới vào bảng trên và
commit trong cùng một lần. Không sửa tay file ở đây rồi để lệch với artifact.
