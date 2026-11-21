# Hướng dẫn

Hướng dẫn theo tình huống cho API của Autoxide. Mỗi hướng dẫn có ví dụ `curl`, chạy như test với mỗi bản build,
và ví dụ TypeScript dùng SDK ở [`sdk/typescript`](../../sdk/typescript/).

Tiếng Anh: [`README.md`](README.md).

- [Hiện bài viết và ảnh trên website](website.vi.md): endpoint công khai, `srcset` và WebP, ETag và 304
- [Dựng trang chủ của cửa hàng từ API](storefront.vi.md): vị trí và banner, bộ sưu tập, thương hiệu và liên hệ, cache
- [Đăng nhập và giữ phiên đăng nhập](authentication.vi.md): access token và refresh token, đăng xuất
- [Đăng nhập bằng Google, Microsoft, GitHub hay OpenID Connect](oidc.vi.md): nhà cung cấp theo khách, liên kết tài khoản an toàn
- [Đồng bộ nhân viên từ hệ thống danh tính (SCIM 2.0)](scim.vi.md): Entra ID, Okta và Google Workspace tạo, sửa, khoá tài khoản nhân viên
- [API key cho chương trình và AI agent](api-keys.vi.md): ít permission nhất, giới hạn tốc độ, thu hồi
- [Gọi lại an toàn](retries.vi.md): `Idempotency-Key`
- [Đọc danh sách dài](pagination.vi.md): `limit`, `cursor`, `order`
- [Đọc lỗi](errors.vi.md): `code`, `errors`, `Retry-After`
- [Nối AI agent qua MCP](mcp.vi.md): key cho agent, cấu hình client, các tool
- [Nhận event theo thời gian thực](realtime-events.vi.md): Server-Sent Events, `Last-Event-ID`, giới hạn
- [Tải file lớn thẳng lên nơi lưu](direct-uploads.vi.md): link đã ký tới S3, xác nhận, kiểm file
- [Thử thay đổi trong sandbox](sandbox.vi.md): bản thử của khách, đối tác giả lập, đem bản nháp về
- [Tự động hoá công việc bằng luật](automations.vi.md): kích hoạt, điều kiện, tool MCP làm hành động, duyệt, cảnh báo số liệu bất thường
- [Làm app điện thoại hay app máy tính](mobile.vi.md): SDK, token và PKCE, thông báo đẩy, đồng bộ khi mất mạng, WebSocket, mua trong ứng dụng
- [Viết một bộ nối nhà cung cấp](provider-adapters.vi.md): tài khoản theo khách, secret, webhook, test không ra mạng (cho người phát triển)
