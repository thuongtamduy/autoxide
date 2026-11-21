## D-20261121-pin-ci-deps · Ghim action, công cụ CI và image nền theo SHA / digest, Dependabot nâng — 2026-11-21 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. Mọi `uses:` của GitHub Actions trong `.github/workflows/` ghim theo SHA đầy đủ của commit, kèm comment phiên bản
   (`actions/checkout@<sha> # v7.0.1`); không dùng thẻ trôi (`@v7`). Workflow gọi workflow trong repo (`./…`) giữ nguyên.
2. Công cụ cài bằng `taiki-e/install-action` ghi phiên bản (`cargo-deny@0.22.0`, `cargo-nextest@0.9.152`).
3. Image nền của `Dockerfile` (`rust`, `debian`) và Postgres của `compose.yml` ghim theo digest, giữ thẻ để đọc.
4. `.github/dependabot.yml` thêm `github-actions` (gom một nhóm) và `docker-compose` bên cạnh `cargo`, `docker`: mỗi tuần
   đề xuất SHA / digest mới, CI chạy trên PR đó như mọi PR.
5. Giá trị từ ngữ cảnh GitHub do người ngoài đặt được (tên nhánh, tiêu đề…) đưa vào script qua `env:`, không chèn thẳng
   `${{ … }}` vào `run:`.

Lý do: workflow phát hành có quyền ghi (đẩy image, tạo Release, ký chứng thực); thẻ của action hay image có thể bị đẩy
lại sang mã khác. Ghim theo nội dung thì build lặp lại được, và nâng cấp đi qua PR có người / CI xem.

Đã cân nhắc: chỉ ghim job có quyền ghi (hai cách viết trong cùng repo, dễ sót); Renovate (thêm một ứng dụng, Dependabot
đã có sẵn và đủ); ghim cả image dịch vụ của job test (Postgres, moto — job chỉ đọc, đổi thường xuyên theo ma trận phiên
bản; để sau).
