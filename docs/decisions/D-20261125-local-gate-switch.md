## D-20261125-local-gate-switch · Công tắc cho gate nhanh trên máy: tắt thì CI trên pull request là chốt chặn — 2026-11-25 — chủ dự án quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. Gate nhanh có **công tắc** là biến môi trường `GATE_LOCAL` = `on` (mặc định) | `off`, đọc bởi `gate.sh`. Bật hay tắt
   là một lựa chọn lúc chạy, không phải sửa file: chủ dự án đặt biến ở cấu hình môi trường cloud, hoặc nói "tắt gate" /
   "bật gate" trong phiên và phiên điều phối chuyển lời cho phiên con. Agent không tự tắt.
2. `GATE_LOCAL=off ./gate.sh --quick` chỉ chạy các kiểm rẻ — fmt, `cargo xtask check`, SDK, api-diff, `cargo deny`,
   gitleaks — rồi báo xanh và in rõ phần chưa kiểm. Không clippy, không kiểm file sinh, không test. Agent chạy thêm các
   bài test vừa viết hay vừa sửa (bài chạm database thêm trên Postgres, D-263).
3. Khi `off`, **CI xanh trên pull request thay "gate nhanh xanh"** ở mọi luật: không merge vào `main` khi CI của đúng
   head đó chưa xanh, không push thẳng lên `main`.
4. Không đổi: gate đầy đủ, `--pr`, `--only` không đọc công tắc; gate đầy đủ xanh trước khi phát hành (D-300); gate đỏ
   thì sửa code, không nới gate.
5. Tại thời điểm ghi mục này chủ dự án bảo **tắt**, tạm thời, cho mọi phiên đang chạy.

Lý do: gate nhanh trên máy cloud bốn nhân mất 15–30 phút cho một thay đổi thường và 40–60 phút khi chạm kernel (mọi crate
test lại), phần lớn là biên dịch test; nhiều phiên song song mỗi lần push đều chờ như vậy. CI trên pull request chạy
cùng các bước đó trên cả SQLite và Postgres và đã là điều kiện merge (D-20261121-ci-batch-release-faster), nên `main`
không kém an toàn hơn; cái giá là lỗi lộ ra muộn hơn một vòng CI. Các kiểm rẻ giữ lại vì mất một hai phút, bắt loại
lỗi hay gặp nhất, và gitleaks phải chạy trước khi secret rời máy.

Đã cân nhắc: công tắc là một dòng trong `CLAUDE.md` (phải sửa file và merge mỗi lần đổi — chủ dự án muốn một lựa chọn);
bỏ hẳn mọi kiểm trên máy (lỗi một phút cũng phải chờ một vòng CI, secret có thể rời máy); cờ dòng lệnh thay biến môi
trường (biến đặt được một lần cho cả môi trường cloud, cờ thì mỗi agent phải nhớ gõ).

Thay: phần "`./gate.sh --quick` trước mỗi commit, đủ để merge vào `main`" của D-300, trong lúc công tắc `off`
