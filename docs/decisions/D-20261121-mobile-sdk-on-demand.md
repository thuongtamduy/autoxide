## D-20261121-mobile-sdk-on-demand · Build SDK di động (Swift, Kotlin, Dart) chỉ khi chạy tay, trước mỗi lần phát hành — 2026-11-21 — chủ dự án quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: job `Mobile SDK (swift|kotlin|dart)` của CI chỉ sinh và biên dịch SDK khi workflow chạy tay
(`workflow_dispatch`: Actions → CI → Run workflow). Ở pull request và push vào `main` job vẫn chạy, báo xong ngay
(không chặn check bắt buộc) và không build gì. Trước mỗi lần phát hành, chạy CI bằng tay trên nhánh release một lần để
đồng bộ và kiểm cả ba SDK (bước 4 của `docs/RELEASING.md`).

Giữ nguyên ở mọi PR: `cargo xtask sdk --check` trong gate (cấu hình SDK khớp `docs/openapi.json`, bản sao hợp đồng
của Swift khớp) và job SDK TypeScript (`tsc`, khoảng 30 giây) — giao diện web dùng SDK TypeScript.

Lý do: hợp đồng API còn đổi liên tục; mỗi lần đổi, build SDK Swift trên Linux mất 30–60 phút (lượt CI sau #309 bị huỷ
vì hết 30 phút), Kotlin và Dart thêm vài phút mỗi bản, trong khi chưa có ứng dụng di động nào dùng các SDK này. Chủ dự
án yêu cầu dời phần này tới khi hoàn thiện, đồng bộ một lần. Build mỗi lần phát hành vẫn bảo đảm SDK đi kèm bản phát
hành sinh và biên dịch được.

Đã cân nhắc: build theo lịch hằng tuần (vẫn tốn khi không có gì để phát hành, và lỗi tìm ra không gắn với lần đổi nào);
bỏ hẳn job (mất kiểm tra trước khi phát hành, và tên check có thể đang là check bắt buộc).

Thay: phần "build SDK di động khi push vào `main` đổi hợp đồng API" của D-20261119-step69-sdk.
