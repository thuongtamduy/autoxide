## D-20261125-decision-index-untracked · Mục lục quyết định `INDEX.md` không nằm trong git, mỗi máy tự sinh — 2026-11-25 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. **`docs/decisions/INDEX.md` thôi nằm trong git**: thêm vào `.gitignore`, bỏ khỏi `.gitattributes`. Hai PR mỗi cái thêm một
   file quyết định không còn chạm chung một file nào, nên không xung đột.
2. **Mỗi máy sinh lấy** bằng `cargo xtask decisions` (xtask chỉ phụ thuộc `jiff`, `serde`, `serde_json`: dựng trong chừng
   20 giây). `scripts/cloud-setup.sh` (hook SessionStart của máy cloud) sinh sẵn lúc mở phiên; `CLAUDE.md` và các skill dặn
   chạy lệnh khi file thiếu hay cũ.
3. **Gate**: bỏ luật `decisions-index` (không còn file đã commit để so). Giữ `decisions-id` và `decisions-ref` (ID trùng,
   tham chiếu tới ID không có). Thêm vào `decisions-id`: đoạn `Thay…` có ID ngoài ngoặc mà bộ phân tích không tách ra được
   thì đỏ.
4. **Sửa bộ phân tích dòng `Thay`**: dấu `:` đứng trước mọi ID không còn cắt cụt phần nêu mục bị thay; phần trong dấu nháy
   kép là trích dẫn nên bỏ cả đoạn (dấu `—`, `:`, `. ` và ID trong đó không tính). Chạy lại trên nhật ký hiện có tìm ra năm
   mục "thay một phần" bị bỏ sót lâu nay (D-221 bởi D-232, D-020 bởi D-022, D-219 bởi D-229, D-327 bởi D-337, và
   D-20261121-sepay-einvoice bởi D-20261122-einvoice-provider-number).

Thay: phần "`docs/decisions/INDEX.md` … `merge=generated`" của D-20261119-merge-batches, và luật `decisions-index` ở mục 4 của
D-20261118-decision-ids — mục lục không còn là file đã commit nên không có gì để so hay để gộp.

Lý do: ngày 25/11/2026 năm PR liên tiếp (#499, #500, #505, #506…) kẹt xung đột chỉ vì file mục lục này: hai PR cùng thêm một
quyết định thì cùng thêm một dòng cuối một chủ đề, và mục mới thay mục cũ còn sửa dòng của mục cũ. CI không chạy trên PR
xung đột, nên mỗi lần phải gộp `main` và sinh lại tay. File sinh được hoàn toàn từ các file quyết định nên lưu nó trong git
chỉ thêm việc.

Đã cân nhắc: giữ file nhưng đổi định dạng (mỗi mục một dòng, `merge=union`) — vẫn xung đột khi hai PR thêm sát nhau, và
dòng "thay bởi" của mục cũ bị sửa chéo; `union` còn sinh dòng trùng, gate lại phải bắt tay; chia mục lục thành nhiều file
theo chủ đề — giảm chứ không hết xung đột. Bỏ khỏi git thì không còn gì để xung đột. Đổi lại: agent phiên sau phải sinh file
trước khi đọc (một lệnh, ~20 giây lần đầu), và người đọc repo trên GitHub không thấy mục lục (README trỏ tới lệnh sinh).
