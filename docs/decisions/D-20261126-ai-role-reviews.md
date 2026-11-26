## D-20261126-ai-role-reviews · Rà pháp lý, kiểm thử bảo mật kiểu bên ngoài và dùng thử bản RC do phiên AI đóng vai, vài vòng — 2026-11-26 — chủ dự án quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. Ba việc "từ bên ngoài" của điều kiện ra 1.0 do phiên AI đóng vai làm, không chờ con người:
   - rà pháp lý: phiên đóng vai luật sư công nghệ / dữ liệu cá nhân và chuyên gia thuế, hoá đơn điện tử;
   - kiểm thử bảo mật kiểu bên ngoài: phiên đóng vai đội pentest độc lập, bắt đầu chỉ với tài liệu công khai (hộp đen)
     rồi mới đọc mã (hộp xám), chỉ tấn công máy chủ chạy trong chính phiên đó;
   - dùng thử bản RC: phiên đóng vai nhiều kiểu người dùng (người mới / code bằng AI, người viết module, người nối AI agent
     qua MCP, người vận hành, chủ cửa hàng dùng API), chỉ theo tài liệu công khai.
2. Mỗi việc làm vài vòng: vòng 1 trên `main` hiện tại để lộ vấn đề sớm, sửa, rồi vòng sau; vòng cuối trên bản `1.0.0-rc.N`.
   Điều kiện ra 1.0 là vòng cuối không còn điểm chặn (pháp lý), lỗi mức cao trở lên (bảo mật) hay lỗi hợp đồng mở (RC).
3. Phiên đóng vai độc lập với phiên viết code: không nhận kết luận của tác giả, mỗi vòng một phiên mới.
4. Nói thật với người dùng: tài liệu (README, `SECURITY.md`, `docs/COMPATIBILITY.md`) ghi rõ các vòng này do AI làm, không
   phải ý kiến pháp lý hay chứng nhận bảo mật; người triển khai cho khách thật nên tự thuê luật sư và kiểm thử bảo mật.
   Rà bởi luật sư và đội kiểm thử là con người được khuyến nghị nhưng không chặn 1.0.
5. Lỗi bảo mật mức cao trở lên báo kín cho phiên điều phối, chỉ công bố sau khi sửa và merge; không đưa script khai thác
   vào repo khi lỗi chưa sửa.

Lý do: chủ dự án muốn hoàn thiện tới 1.0 không phải chờ thuê người ngoài, và yêu cầu mở phiên đóng vai cho ba việc này,
vài vòng. Đợt pentest nội bộ tháng 11/2026 (docs/security/pentest-2026-11) và các lượt kiểm độc lập trong ngày đã cho thấy
phiên AI độc lập bắt được lỗi thật.

Đã cân nhắc: giữ điều kiện "luật sư thật / đội pentest thật / người ngoài dùng 2–4 tuần" (chặn 1.0 vô thời hạn vào việc
ngoài tầm của dự án); bỏ hẳn ba việc (không có lớp kiểm từ góc nhìn bên ngoài).

Thay: phần "luật sư đã rà" và "kiểm thử bảo mật bên ngoài" trong điều kiện ra 1.0 của D-20261126-road-to-1-0 (mục 5), và
"cho người ngoài dùng 2–4 tuần" trong D-20261126-release-1-0-readiness
