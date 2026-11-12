# Bàn giao cho repo giao diện (trang quản trị + website)

Đọc cho người (và agent) làm repo giao diện của Autoxide. Core chỉ có API; giao diện là repo riêng,
công khai, chỉ dùng API của core. File này nói: API nào **đã dùng được**, API nào **đang làm**,
**quy ước chung** phía giao diện phải theo, và **việc cụ thể** cho từng module.

Ngày viết: 12/11/2026. Trạng thái thật luôn lấy từ `main` của repo core — file này có thể cũ hơn.

## 1. Nguồn sự thật

| Cần biết | Xem ở |
|---|---|
| Hợp đồng API (endpoint, kiểu dữ liệu, lỗi, permission) | `docs/openapi.json` (sinh từ code, gate kiểm luôn khớp); máy chủ phục vụ ở `/api/v1/openapi.json` |
| Stack giao diện đã chốt | Quyết định "Stack của dự án giao diện" trong `docs/DECISIONS.md` (lúc viết là D-086 trên nhánh đề bài; số có thể đổi sau khi gộp vào `main`) |
| Thiết kế `content` | `docs/prompts/step-17-content.md` + `docs/research/content.md` |
| Thiết kế `forms`, `newsletter`, `comments` | `docs/prompts/forms-newsletter-comments.md` + `docs/research/forms-newsletter-comments.md` |
| Thư viện media | `docs/MEDIA.md` |
| Cài đặt, chạy core cục bộ | `docs/DEPLOY.md` |

**Stack đã chốt** (tóm tắt): TypeScript strict; pnpm workspaces + Turborepo; Biome; SDK sinh từ
`openapi.json` bằng `openapi-typescript` + `openapi-fetch`; Next.js 16 (App Router, React 19) cho
**cả** trang quản trị và website, máy chủ Next.js làm BFF giữ refresh token trong cookie `httpOnly`;
Tailwind CSS 4 + shadcn/ui; TanStack Query + TanStack Table; React Hook Form + Zod; Tiptap (nội dung
lưu JSON); next-intl (vi, en); Vitest, Playwright, MSW; build `output: standalone` chạy Docker cạnh
core.

## 2. Trạng thái API

| Phần | Trạng thái | Ghi chú |
|---|---|---|
| Đăng nhập, MFA (TOTP, recovery code), passkey, phiên đăng nhập, quên / đổi mật khẩu, xác minh email | **Dùng được** | `/api/v1/auth/…` |
| Người dùng, role, permission, API key, xoá tài khoản, xuất dữ liệu cá nhân | **Dùng được** | |
| Audit log, webhook | **Dùng được** | |
| `files` (kho file riêng tư) | **Dùng được** | |
| `media` (thư viện ảnh / video / tài liệu: thư mục, nhãn, ảnh thu nhỏ, thùng rác, công khai / riêng tư) | **Dùng được** | `/api/v1/media/…` |
| `content` (bài viết, trang có cấp cha con, danh mục, thẻ, menu, phiên bản, đa ngôn ngữ, SEO, xem trước, chuyển hướng) | **Đang làm** | Thiết kế đã chốt; tên endpoint có thể đổi khi làm |
| `forms`, `newsletter`, `comments` | **Thiết kế xong, chưa làm** | Làm sau `content` |

Làm được **ngay**: khung repo, SDK, BFF + đăng nhập, quản trị tài khoản / role / API key, thư viện
media. Phần `content` có thể dựng giao diện trước bằng dữ liệu giả (MSW) theo thiết kế, nhưng phải
sinh lại SDK và sửa khi API thật ra.

## 3. Quy ước chung phía giao diện phải theo

1. **Hình dạng response:** chỉ ba dạng — `{"data": …}`, `{"data": […], "pagination": {"limit", "next_cursor"}}`,
   và lỗi `application/problem+json` (RFC 9457).
2. **Lỗi:** hiển thị theo `code` (ổn định, ví dụ `media.too_large`), tra bản dịch trong next-intl;
   `message` của máy chủ chỉ là dự phòng. Lỗi kiểm dữ liệu trả **mọi** lỗi một lần, mỗi lỗi kèm
   trường — gắn lên đúng ô của form.
3. **Ngôn ngữ:** gửi `Accept-Language` (vi / en) mọi request; không gửi thì máy chủ trả tiếng Anh.
4. **Ghi lặp lại an toàn:** mọi lệnh ghi gửi `Idempotency-Key` (UUID mới mỗi thao tác của người
   dùng, giữ nguyên khi retry). Lớp chặn trong SDK tự gắn.
5. **Chống ghi đè:** `GET` một bản ghi trả `ETag`; lệnh sửa gửi lại trong `If-Match`. `412` = người
   khác đã sửa trước → báo người dùng và cho tải lại / so sánh; `428` = quên gửi `If-Match` (lỗi
   của giao diện). Endpoint mới (kể cả `content`) **bắt buộc** `If-Match`.
6. **Danh sách:** phân trang bằng cursor — không có số trang, không có tổng số. Dùng "tải thêm" /
   cuộn vô hạn; đổi sắp xếp thì bỏ cursor cũ.
7. **Cache:** gửi `If-None-Match` → `304` khi không đổi. Endpoint công khai có `Cache-Control`.
8. **Đăng nhập qua BFF:** access token sống 15 phút; refresh token **xoay vòng** — dùng lại refresh
   token cũ bị coi là bị đánh cắp và **cả họ token bị thu hồi**. BFF phải tuần tự hoá việc refresh
   (hai tab refresh cùng lúc sẽ tự đăng xuất nhau). Trình duyệt không bao giờ cầm refresh token.
   Đăng nhập hai bước khi bật MFA; passkey không cần email.
9. **Permission:** ẩn / khoá nút theo danh sách permission của người dùng (`GET /api/v1/auth/me`), nhưng
   máy chủ vẫn là bên kiểm cuối cùng.
10. **Tạo mới** trả `201` + `Location`.

## 4. Việc theo module

### `media` (làm được ngay)

Thư viện: lưới / danh sách, thư mục lồng nhau, nhãn, lọc theo loại, tìm theo tên, sắp theo tên /
kích thước / ngày; tải lên (thân request là **nội dung thô**, không phải multipart — xem
`docs/MEDIA.md`); sửa alt / tiêu đề / chú thích; thay nội dung giữ nguyên ID; thùng rác (khôi phục,
xoá hẳn); công khai / riêng tư; ảnh thu nhỏ lấy bằng `?w=`; video / PDF tải theo `Range`. Hộp chọn
ảnh dùng lại cho `content`.

### `content` (khi API ra)

**Trang quản trị**

- Trình soạn Tiptap **chỉ bật đúng các node / mark máy chủ cho phép** (danh sách trong đề bài
  `content`, mục "Thân bài" — chốt cùng phía core; máy chủ từ chối node lạ). Ảnh trong bài là node
  `image` mang `mediaId`, **không có `src`**; nhúng video chỉ host được phép.
- Nháp / đăng / gỡ / lưu trữ; hẹn giờ đăng và gỡ; lịch sử phiên bản + khôi phục (tạo nháp mới).
- Cây trang: chọn trang cha, kéo thả để **chuyển nhánh** (thao tác `move` riêng); hiện đường dẫn do
  máy chủ tính.
- Bản dịch vi / en liên kết nhau; tạo bản dịch từ bản gốc.
- Danh mục, thẻ, tác giả, menu nhiều tầng (sửa cả cây menu một lần), chuyển hướng làm tay.
- Nút "Xem trước": xin preview token rồi mở website ở chế độ xem trước.

**Website (Next.js)**

- Một route bắt mọi đường dẫn: gọi `resolve(locale, path)` → hiển thị nội dung / chuyển hướng
  (301) / trả 410 / 404. **Không tự ghép đường dẫn**: máy chủ tính `path`, website chỉ thêm tên miền
  và tiền tố ngôn ngữ.
- Không fallback ngôn ngữ: thiếu bản dịch là 404 (máy chủ trả danh sách bản dịch đang có để gợi ý).
- Xem trước: route handler nhận preview token, bật Draft Mode, đọc bản nháp; chuyển hướng tới
  đường dẫn **lấy từ máy chủ**, không lấy từ query string (tránh open redirect).
- Làm mới cache: route nhận webhook của core, **kiểm chữ ký Standard Webhooks**, gọi
  `revalidateTag` / `revalidatePath` theo `path` mới **và mọi `path` cũ** trong payload (đổi slug
  trang cha thì trang con cũng đổi).
- SEO: meta title / description, Open Graph (ảnh từ `media`), canonical, `noindex`, JSON-LD
  `Article` / `BreadcrumbList`; sitemap có `alternates.languages` (hreflang) từ endpoint dữ liệu
  sitemap; RSS 2.0 / Atom / JSON Feed từ endpoint feed (render thân JSON ra HTML phía máy chủ
  Next.js bằng Tiptap; ID của item là UUID, không phải URL).

### `forms`, `newsletter`, `comments` (sau `content`)

- **Chống spam phía giao diện:** mỗi form có **trường bẫy** ẩn (người không thấy, bot điền); **lấy
  token form lúc hiển thị** (không nhúng trong HTML đã cache); nếu máy chủ bật ALTCHA thì gắn widget
  ALTCHA đúng phiên bản ghim trong tài liệu core. Máy chủ trả **cùng một câu trả lời** cho bài nhận,
  bài bị cách ly và bot — giao diện luôn hiện "đã gửi".
- **`forms`:** hiển thị form từ định nghĩa máy chủ trả (kiểu trường, nhãn theo ngôn ngữ, bắt buộc,
  ô đồng ý kèm văn bản); trang quản trị: trình tạo form, hộp thư bài gửi (đánh dấu, spam / không
  phải spam), xuất CSV.
- **`newsletter`:** form đăng ký; **trang xác nhận gọi `POST`** với token (không xác nhận khi chỉ mở
  link — máy quét link của hộp thư sẽ tự mở); trang chọn danh sách / huỷ đăng ký bằng token; quản
  trị: danh sách, người đăng ký, chiến dịch (nháp, gửi thử, hẹn giờ, tạm dừng / tiếp tục), danh sách
  chặn.
- **`comments`:** khung bình luận theo đích (luồng hai cấp, tải thêm bằng cursor, đếm số bình luận
  theo nhiều bài một lần); khách nhận **token sửa** khi gửi — lưu ở trình duyệt để sửa / xoá trong
  thời hạn; nút báo vi phạm; trang quản trị: hàng chờ duyệt, cấm người dùng / email.

## 5. Cần thống nhất với phía core

| Việc | Vì sao |
|---|---|
| Danh sách node / mark Tiptap được phép | Máy chủ kiểm và từ chối node lạ; hai bên phải giống nhau |
| Quy ước cache tag cho `revalidateTag` | Payload webhook mang đủ dữ liệu; tên tag do website đặt, nên ghi lại một chỗ |
| Tiền tố ngôn ngữ và tiền tố bài viết trên URL (`/vi/…`, `/blog/…`) | Máy chủ tính `path` theo cấu hình `AUTOXIDE_CONTENT_POST_PATH_PREFIX` |
| `AUTOXIDE_HTTP_CORS_ORIGINS`, số proxy tin cậy (CDN → Next.js → API) | Sai số proxy thì rate limiting và chống spam đếm sai IP |
| Secret ký webhook và preview | Cấu hình khi deploy, không vào git |
