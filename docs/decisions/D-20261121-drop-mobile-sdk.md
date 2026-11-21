## D-20261121-drop-mobile-sdk · Bỏ SDK di động (Swift, Kotlin, Dart), giữ SDK TypeScript — 2026-11-21 — chủ dự án quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: xoá SDK Swift, Kotlin, Dart và mọi thứ chỉ phục vụ chúng: `sdk/swift/`, `Package.swift`,
`sdk/openapitools.json`, `xtask/src/sdk_mobile.rs` (và phần kiểm tương ứng trong `cargo xtask sdk --check`), job CI
`Mobile SDK (swift|kotlin|dart)`, dòng SDK di động của `STACK.md`, các bước `artifactVersion` / `pubVersion` / chép hợp
đồng Swift / build tay SDK di động trong `docs/RELEASING.md`. Hướng dẫn `docs/guides/mobile.md` (+ `.vi.md`) mục "Pick an
SDK" trỏ tới SDK TypeScript (web, React Native, Node) hoặc tự sinh client từ `docs/openapi.json`.

Giữ nguyên: SDK TypeScript và job `tsc`; image Docker `linux/arm64`; phía máy chủ gửi push (FCM/APNs) — đó không phải SDK;
bước 69 các mục P2–P5 (đồng bộ ngoại tuyến, WebSocket, mua trong ứng dụng, hướng dẫn).

Lý do: chưa có ứng dụng di động nào dùng ba SDK này; mỗi lần hợp đồng API đổi, người phát triển phải gánh bản chép hợp
đồng, phiên bản gói và (trước đây) build Swift 30–60 phút. Ai làm app native vẫn tự sinh được client từ OpenAPI chuẩn
bằng công cụ của nền tảng mình, nên không mất khả năng.

Thay: phần SDK di động của D-20261119-step69-sdk, và toàn bộ D-20261121-mobile-sdk-on-demand.
