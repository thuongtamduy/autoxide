# Nâng cấp client: mọi danh sách là một trang

Tiếng Anh: [`list-pages-upgrade.md`](list-pages-upgrade.md). Cách đọc danh sách nói chung: xem
[Đọc danh sách dài](pagination.vi.md).

Trước bản 1.0, mọi endpoint `GET` trả danh sách đã được đổi tại chỗ để trả **một trang**. Đường dẫn giữ nguyên; hình
dạng câu trả lời thì đổi. Client viết theo hình dạng cũ chỉ đọc được 50 mục đầu và phải sửa.

## Đổi gì

Trước đây, một số danh sách trả cả danh sách trong `data`:

```json
{"data": [{"id": "…"}, {"id": "…"}]}
```

Nay mọi danh sách trả một trang, có `pagination` bên cạnh `data`:

```json
{"data": [{"id": "…"}, {"id": "…"}], "pagination": {"limit": 50, "next_cursor": "…"}}
```

- `data` chứa các mục của trang này, như trước.
- `pagination.limit` là cỡ trang đã áp.
- `pagination.next_cursor` là chỗ trang sau bắt đầu; `null` (hay không có) nghĩa là trang cuối.
- `pagination.total` là số mục khớp bộ lọc của bạn, tính trên mọi trang. Chỉ có ở trang đầu của request gửi kèm
  `count=true`.

Tool MCP trả danh sách cũng vậy: nhận cùng tham số và trả cùng dạng `{"data": […], "pagination": {…}}`.

## Đọc hết một danh sách

Mọi danh sách nhận cùng bốn tham số query:

| Tham số  | Nghĩa |
| -------- | ----- |
| `limit`  | Số mục mỗi trang, 1 tới 200. Mặc định 50. |
| `cursor` | `next_cursor` của trang trước. Bỏ trống ở trang đầu. |
| `order`  | `asc` hay `desc`. Mặc định tuỳ danh sách (xem dưới). |
| `count`  | `true` để có thêm `pagination.total` ở trang đầu. Đếm tốn một câu truy vấn: chỉ hỏi khi bạn hiển thị nó. |

Hỏi trang đầu, rồi khi `next_cursor` còn khác `null` thì hỏi lại với `cursor` là giá trị đó **và cùng bộ lọc, `order`,
`sort`**. Cursor của danh sách khác, hay của cùng danh sách nhưng `sort` khác, trả `422` với lỗi ở ô `cursor`. Cursor
vẫn đúng trong lúc bạn đọc, kể cả khi có mục mới.

```bash
curl -sS "$API/api/v1/content/entries?limit=100&count=true" -H "Authorization: Bearer $TOKEN"
```

```bash
curl -sS "$API/api/v1/content/entries?limit=100&cursor=$CURSOR" -H "Authorization: Bearer $TOKEN"
```

Dừng khi `next_cursor` là `null`, **không** dừng khi một trang ngắn hơn `limit`. Danh sách kiểm quyền của bạn trên
từng mục (vd `GET /api/v1/kernel/custom-fields` không có `target`, hay các yêu cầu duyệt) chỉ đọc một số dòng có giới
hạn mỗi request: một trang có thể ít hơn `limit` mục, kể cả rỗng, mà vẫn có `next_cursor`. Mô tả của chúng trong tài
liệu API nói rõ điều này.

## Danh sách xếp theo thứ tự nào

Mô tả của mỗi danh sách nói nó xếp theo gì. Có ba loại:

- **Mới nhất trước** — danh sách không có thứ tự nào khác ngoài lúc tạo (đơn hàng, hoàn tiền, event, ghi chú…):
  `order` mặc định `desc`; `order=asc` cho cũ nhất trước.
- **Thứ tự tự nhiên** — danh sách có thứ tự riêng có nghĩa (vị trí, tên, mã, tầng trong cây…): `order` mặc định
  `asc` (vị trí đầu, A tới Z); `order=desc` đảo lại.
- **Theo `sort`** — danh sách có tham số `sort`: `order` mặc định theo cột đang chọn — `desc` (mới nhất trước) cho thời
  gian như `created_at`, `updated_at`; `asc` cho tên, mã, vị trí, kích thước. Mô tả của `sort` nêu mặc định của từng
  giá trị. `order` bạn tự gửi luôn được giữ.

Thứ tự ổn định giữa các trang: theo `next_cursor` thì không mục nào lặp lại hay bị sót.

## Với SDK TypeScript

`paginate()` tự theo `next_cursor` và trả từng mục một:

```ts
import { createClient } from "@autoxide/sdk";

const api = createClient({ baseUrl: "https://api.example.com", token: "axk1_…" /* read it from your secret store */ });

// Every record that uses an image, whatever the number of pages.
for await (const usage of api.paginate("/api/v1/kernel/records/{kind}/{id}/usages", {
  path: { kind: "media.item", id: "0190a4c2-0000-7000-8000-000000000001" },
  query: { limit: 100 },
})) {
  console.log(usage.kind, usage.id, usage.title);
}
```

Để hiện từng trang (bảng có nút "trang sau") kèm tổng, gọi `get()`; danh sách giữ nguyên `pagination`:

```ts
import { createClient } from "@autoxide/sdk";

const api = createClient({ baseUrl: "https://api.example.com", token: "axk1_…" /* read it from your secret store */ });

const first = await api.get("/api/v1/content/entries", { query: { limit: 20, count: true } });
console.log(`${first.pagination.total ?? "?"} entries`);
let cursor = first.pagination.next_cursor;
while (cursor) {
  const page = await api.get("/api/v1/content/entries", { query: { limit: 20, cursor } });
  for (const entry of page.data) console.log(entry.id, entry.title);
  cursor = page.pagination.next_cursor;
}
```

## Với tool MCP

Tool trả danh sách nhận `limit`, `cursor`, `order` và `count` (thêm `sort` khi endpoint REST có) với cùng nghĩa, và trả
`{"data": […], "pagination": {…}}`. Agent gọi lại tool với `cursor` là `pagination.next_cursor` và các đối số khác giữ
nguyên cho tới khi nó là `null`. Xem [Nối AI agent qua MCP](mcp.vi.md).

## Ngoại lệ

Ba endpoint tra cứu trả danh sách trong `data` không có `pagination`, vì mỗi khoá bạn gửi cho tối đa một mục (tối đa
100 khoá), theo thứ tự bạn gửi: `GET /api/v1/catalog/variants?code=…`, `GET /api/v1/comments/public/counts` và
`GET /api/v1/inventory/public/availability`.

## Đổi gì, theo từng endpoint

Changelog liệt kê mọi endpoint và tool đã đổi, thứ tự mặc định của nó và các thay đổi hành vi (danh sách đổi thứ tự,
tham số `limit` riêng của một endpoint thay bằng tham số chung): xem mục "Changed" của `CHANGELOG.md` cho bản phát
hành này, các dòng ghi **Breaking (pre-1.0, D-20261125-list-pages)**.
