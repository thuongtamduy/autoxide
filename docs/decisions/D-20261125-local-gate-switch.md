## D-20261125-local-gate-switch · Tạm tắt gate nhanh trên máy, CI trên pull request là chốt chặn — 2026-11-25 — chủ dự án quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. `CLAUDE.md` có một **công tắc gate trên máy**, ghi ở đầu mục "Gate". Hiện là **TẮT**, tạm thời, tới khi chủ dự án
   bảo bật lại.
2. Khi TẮT: không bắt buộc `./gate.sh --quick` trước commit hay push. Trước khi push chỉ chạy các kiểm rẻ —
   `cargo fmt --check`, `cargo xtask check`, `cargo xtask regen --check` khi đổi API hay cấu hình — và các bài test vừa
   viết hay vừa sửa (bài chạm database chạy thêm trên Postgres, D-263).
3. Khi TẮT, **CI xanh trên pull request thay gate nhanh** ở mọi luật nhắc tới "gate nhanh xanh": không merge vào `main`
   khi CI của đúng head đó chưa xanh, không push thẳng lên `main`.
4. Không đổi: gate đầy đủ xanh trước khi phát hành (D-300); gate đỏ thì sửa code, không nới gate; `gate.sh` giữ nguyên,
   ai muốn vẫn chạy được.
5. Bật lại bằng một quyết định mới đổi công tắc thành BẬT; không cần sửa gì khác.

Lý do: gate nhanh trên máy cloud bốn nhân mất 15–30 phút cho một thay đổi thường và 40–60 phút khi chạm kernel (mọi crate
test lại), phần lớn là biên dịch test; nhiều phiên song song mỗi lần push đều chờ như vậy. CI trên pull request chạy
cùng các bước đó trên cả SQLite và Postgres và đã là điều kiện merge (D-20261121-ci-batch-release-faster), nên `main`
không kém an toàn hơn; cái giá là lỗi lộ ra muộn hơn một vòng CI. Các kiểm rẻ giữ lại vì chúng mất một hai phút và bắt
loại lỗi hay gặp nhất (định dạng, ranh giới module, bản dịch thiếu, file sinh cũ).

Đã cân nhắc: giữ nguyên (chậm, là lý do của mục này); bỏ hẳn mọi kiểm trên máy (lỗi một phút cũng phải chờ một vòng
CI); thêm cờ vào `gate.sh` (không cần — công tắc là luật cho agent, không phải hành vi của script).

Thay: phần "`./gate.sh --quick` trước mỗi commit, đủ để merge vào `main`" của D-300, trong lúc công tắc TẮT
