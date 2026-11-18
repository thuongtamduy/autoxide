# Nhận event theo thời gian thực

English: [`realtime-events.md`](realtime-events.md). Ví dụ dùng `curl` với `API` là địa chỉ máy chủ của bạn (`export API=https://api.example.com`), và SDK TypeScript (`sdk/typescript`). Chúng chạy như test ở mỗi bản dựng, nên khớp với máy chủ của phiên bản này.

`GET /api/v1/kernel/events/stream` giữ một kết nối mở và đẩy event tới bạn ngay khi xảy ra, theo chuẩn [Server-Sent Events](https://html.spec.whatwg.org/multipage/server-sent-events.html): màn hình quản trị làm mới khi một bài được đăng, hàng chờ kiểm duyệt khi có bình luận mới. Chỉ event mà module **khai là stream được** mới được gửi, với cùng permission như đọc bản ghi qua REST — stream không bao giờ đẩy thay đổi thô của bảng. Hệ thống không được bỏ sót gì khi ngừng hoạt động lâu hơn một tuần thì dùng webhook.

Máy chủ phải đặt `AUTOXIDE_EVENTS_STREAM=true`; nếu không, endpoint trả `404 kernel.stream_disabled`.

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
```

## Event nào stream được

Sổ module ghi `stream_permission` của từng event; event không có trường này không được stream. Hiện có: `content.entry_published` (`content.read`), `comments.comment_created` (`comments.moderate`), `media.item_uploaded` (`media.read`).

<!-- example: streamable-events -->
```bash
curl -sS "$API/api/v1/kernel/modules/content" -H "Authorization: Bearer $TOKEN"
```

## Mở stream

Nêu tên event trong `names`, cách nhau bằng dấu phẩy: `module.event`, hay `module.*` cho mọi event stream được của một module mà bạn được xem. Máy chủ ở đây không bật stream nên trả `404`; với `AUTOXIDE_EVENTS_STREAM=true` nó trả `200` và giữ kết nối mở (`curl -N` in từng event khi tới).

<!-- example: open-stream -->
```bash
curl -sS "$API/api/v1/kernel/events/stream?names=content.entry_published,comments.*" \
  -H "Authorization: Bearer $TOKEN"
# expect: 404
```

Mỗi event là một khung:

```text
id: 0193f1c2-5a7e-7c3d-8e21-4b6f0a9d2c11
event: content.entry_published
data: {"id":"0193f1c2-5a7e-7c3d-8e21-4b6f0a9d2c11","name":"content.entry_published","occurred_at":"2026-11-18T09:30:00Z","subject":"content.entry:0193…","actor_kind":"user","payload":{"after":{…}}}
```

Cứ 15 giây có một dòng chú thích `: ping` để proxy giữ kết nối.

## Ai thấy gì

- Chỉ event của khách của bạn.
- Với mỗi event, permission ghi ở `stream_permission`. Nêu đích danh một event bạn không được xem thì nhận `403 permission_denied`; với `module.*` những event đó bị bỏ ra.
- Với permission hẹp hơn cả khách (role ở một nút tổ chức), bạn chỉ nhận event do chính mình gây ra: event chưa mang nút của nó.

## Nối lại không mất event

Event được giao **ít nhất một lần**. Khi mất kết nối, nối lại với header `Last-Event-ID` là `id` cuối cùng bạn nhận — `EventSource` của trình duyệt tự làm việc này — hay với `last_event_id=` trong query khi không đặt được header. Bạn nhận phần đã lỡ, và có thể nhận lại vài event: bỏ qua `id` đã có.

Event được giữ 7 ngày. Khi `id` cuối cũ hơn, stream gửi `event: reset` trước: tải lại dữ liệu qua REST rồi nghe tiếp. Máy chủ cũng đóng mỗi stream sau 30 phút (để permission bị thu hồi có hiệu lực) và khi tắt; nối lại cùng cách. Nhiều máy chủ sau bộ cân bằng tải vẫn đúng: máy chủ nào cũng đọc event từ database, nên nối lại vào máy nào cũng được.

## Giới hạn

Mỗi máy chủ giữ tối đa `AUTOXIDE_EVENTS_STREAM_MAX_PER_USER` stream mỗi người dùng (mặc định 5), `AUTOXIDE_EVENTS_STREAM_MAX_PER_TENANT` mỗi khách (200) và `AUTOXIDE_EVENTS_STREAM_MAX_CONNECTIONS` tổng cộng (1000); vượt thì nhận `429 kernel.stream_too_many`. Mỗi màn hình mở một stream với mọi tên event nó cần, không phải mỗi event một stream. Sau Nginx, response đã có `X-Accel-Buffering: no`; proxy khác không được đệm `text/event-stream`.

## Với SDK

`EventSource` không gửi được header `Authorization`, nên đọc stream bằng `fetch`:

```ts
const response = await fetch(`${baseUrl}/api/v1/kernel/events/stream?names=content.entry_published`, {
  headers: { Authorization: `Bearer ${token}`, ...(lastId ? { "Last-Event-ID": lastId } : {}) },
});
const reader = response.body!.pipeThrough(new TextDecoderStream()).getReader();
let buffer = "";
for (;;) {
  const { value, done } = await reader.read();
  if (done) break; // closed by the server: reconnect with lastId
  buffer += value;
  let end;
  while ((end = buffer.indexOf("\n\n")) >= 0) {
    const frame = buffer.slice(0, end);
    buffer = buffer.slice(end + 2);
    const id = frame.match(/^id: (.*)$/m)?.[1];
    const data = frame.match(/^data: (.*)$/m)?.[1];
    if (frame.includes("event: reset")) { /* reload over REST */ }
    if (id && data && !seen.has(id)) { seen.add(id); lastId = id; handle(JSON.parse(data)); }
  }
}
```
