## D-20261125-typescript-range · Dải TypeScript hỗ trợ cho SDK: 5.0 trở lên, phát triển trên 7 — 2026-11-25 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: SDK `sdk/typescript` phát hành dạng mã nguồn `.ts`, nên cam kết dải TypeScript **≥ 5.0** cho người dùng
SDK. Devdependency `typescript` ghim bản 7 (dùng phát triển); CI job `sdk` chạy `tsc --noEmit` thêm một lần bằng
`typescript@5.0.4` (bản thấp nhất, `npx --package`). Nâng sàn lên bản cao hơn thì ghi mục mới.

Lý do: người dùng biên dịch SDK bằng TypeScript của họ, không phải bản của repo; chỉ kiểm bằng bản 7 không chứng minh
5.x hay 6.x biên dịch được. Đã chạy thử `tsc --noEmit` trên SDK + mã sinh + ví dụ hướng dẫn ở 5.0.4, 5.4.5, 5.9.3,
6.0.3, 7.0.2: đều đạt, không cần đổi `tsconfig.json`. 5.0 là sàn tự nhiên vì `moduleResolution: "Bundler"` và
`allowImportingTsExtensions` bắt đầu từ 5.0. Bản 7 kéo thêm 19 gói nhị phân theo nền tảng
(`@typescript/typescript-*`, Apache-2.0, cùng nhóm maintainer Microsoft, có integrity trong lockfile, không có
install script) — chỉ là optionalDependencies, `npm ci` chọn đúng gói cho máy.

Đã cân nhắc: ma trận CI nhiều bản (5.0.4, 5.9.3, 6.0.3, 7.0.2) — thêm thời gian CI mà sàn và bản dev đã bắt phần lớn lỗi;
chỉ ghim bản 5.9.3 (Dependabot đề xuất 7 mỗi tuần, vẫn cần kiểm dải cho người dùng).
