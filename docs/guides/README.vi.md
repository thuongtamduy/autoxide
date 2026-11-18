# Hướng dẫn

Hướng dẫn theo tình huống cho API của Autoxide. Mỗi hướng dẫn có ví dụ `curl`, chạy như test với mỗi bản build,
và ví dụ TypeScript dùng SDK ở [`sdk/typescript`](../../sdk/typescript/).

Tiếng Anh: [`README.md`](README.md).

- [Hiện bài viết và ảnh trên website](website.vi.md): endpoint công khai, `srcset` và WebP, ETag và 304
- [Đăng nhập và giữ phiên đăng nhập](authentication.vi.md): access token và refresh token, đăng xuất
- [API key cho chương trình và AI agent](api-keys.vi.md): ít permission nhất, giới hạn tốc độ, thu hồi
- [Gọi lại an toàn](retries.vi.md): `Idempotency-Key`
- [Đọc danh sách dài](pagination.vi.md): `limit`, `cursor`, `order`
- [Đọc lỗi](errors.vi.md): `code`, `errors`, `Retry-After`
- [Nối AI agent qua MCP](mcp.vi.md): key cho agent, cấu hình client, các tool
- [Nhận event theo thời gian thực](realtime-events.vi.md): Server-Sent Events, `Last-Event-ID`, giới hạn
