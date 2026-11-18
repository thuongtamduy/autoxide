# Tự động hoá công việc bằng luật

English: [`automations.md`](automations.md). Ví dụ dùng `curl` với `API` là máy chủ của bạn (`export API=https://api.example.com`). Ví dụ chạy như test ở mỗi bản build, nên luôn khớp máy chủ của phiên bản này.

Một luật tự động hoá nói "**khi** việc gì đó xảy ra **và** điều kiện đúng **thì** làm các hành động này", không cần viết code. Mỗi hành động là một tool MCP máy chủ đã có, nên luật làm được đúng điều bạn tự làm được qua REST API hay MCP với cùng permission — không hơn.

- **Kích hoạt**: một event của module (`{"type": "event", "event": "<module>.<event>"}`), lịch (`{"type": "schedule", "every": "hour" | "day" | "week", "weekday": "monday", "hour": 9, "minute": 0}`), gọi tay (`{"type": "manual"}`), hay một số liệu ra khỏi mức thường (`{"type": "metric", …}`, xem dưới).
- **Điều kiện**: cây `all` / `any` / `not` trên các phép so `{"field": "event.total", "op": "gt", "value": 5000000}`. Toán tử: `eq`, `ne`, `gt`, `gte`, `lt`, `lte`, `in`, `contains`, `exists`. Không có biểu thức tự do: luật được kiểm lúc lưu và chạy giống nhau ở mọi nơi.
- **Hành động**, theo thứ tự: `{"tool": "<module>.<tool>", "arguments": {…}}`, `{"wait": {"minutes": 60}}` (tối đa 30 ngày), `{"stop_if": <điều kiện>}`, `{"notify": {"title": "…", "body": "…"}}` (thông báo tới người sở hữu luật). Giá trị dùng được mẫu: `"{{ event.code }}"` đứng một mình giữ nguyên kiểu của giá trị; chữ như `"Đơn {{ event.code }}"` được điền thành chữ. Mẫu thấy `event` (dữ liệu kích hoạt), `trigger` (thứ đã bắt đầu lần chạy) và `steps` (kết quả các bước trước: `steps.0.result`).

`GET /api/v1/automations/catalog` liệt kê event, số liệu và tool bạn dùng được, kèm schema đầu vào của từng tool.

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
```

<!-- example: catalog -->
```bash
curl -sS "$API/api/v1/automations/catalog" -H "Authorization: Bearer $TOKEN"
```

## Tạo luật

Lưu luật thì mọi thứ được kiểm một lượt và mọi lỗi báo trong `errors`: event hay tool không có (`automations.trigger_unknown`, `automations.tool_unknown`), tool bạn không có permission (`automations.permission_missing`), tham số không khớp schema đầu vào của tool. Luật chạy với quyền **hiện tại** của người nó chạy thay (`run_as`: người tạo, hoặc người sửa hành động hay bật luật gần nhất); người đó mất một permission luật cần thì luật tự tạm dừng.

<!-- example: create-rule -->
```bash
curl -sS "$API/api/v1/automations/rules" -H "Authorization: Bearer $TOKEN" \
  --json '{"name":"Big orders","trigger":{"type":"manual"},"conditions":{"field":"event.total","op":"gte","value":5000000},"actions":[{"notify":{"title":"Big order {{ event.code }}","body":"Total: {{ event.total }}"}}]}'
# expect: 201
# save: RULE=.data.id
```

Luật do AI agent tạo (API key loại `agent`) nằm ở `pending_approval` tới khi một người bật bằng `POST /api/v1/automations/rules/{id}/enable`; từ đó luật chạy với quyền của người đó.

## Thử mà không làm gì

Chạy thử khô tính điều kiện trên dữ liệu mẫu, điền mẫu, kiểm tool và quyền — không gọi tool nào.

<!-- example: dry-run -->
```bash
curl -sS "$API/api/v1/automations/rules/$RULE/test" -H "Authorization: Bearer $TOKEN" \
  --json '{"event_sample":{"total":6000000,"code":"DH1"}}'
```

## Chạy và đọc nhật ký

Lần chạy diễn ra ở nền. Mỗi lần chạy giữ nhật ký từng bước: trạng thái, kết quả, `code` lỗi, và `Idempotency-Key` của bước (`automation:<run id>:<bước>`) — chạy lại một lần hỏng (`POST /runs/{id}/retry`) bắt đầu từ bước hỏng, không bao giờ làm lại bước đã xong.

<!-- example: run -->
```bash
curl -sS "$API/api/v1/automations/rules/$RULE/run" -H "Authorization: Bearer $TOKEN" \
  --json '{"input":{"total":6000000,"code":"DH1"}}'
# expect: 202
# save: RUN=.data.id
```

<!-- example: read-run -->
```bash
curl -sS "$API/api/v1/automations/runs/$RUN" -H "Authorization: Bearer $TOKEN"
```

<!-- example: list-runs -->
```bash
curl -sS "$API/api/v1/automations/runs?rule_id=$RULE" -H "Authorization: Bearer $TOKEN"
```

## Bước rủi ro chờ người duyệt

Bước có tool xoá hay ghi đè dữ liệu (hoặc cần duyệt khi agent gọi) dừng lần chạy ở `awaiting_approval` mỗi lần. Một người có `automations.run` và permission của chính tool đó duyệt (`POST /runs/{id}/approve`) hay từ chối (`POST /runs/{id}/reject`); hộp việc liệt kê các lần chạy này (`automations.approvals`).

## Vòng lặp và giới hạn

Event do hành động của một luật gây ra không bao giờ kích hoạt lại chính luật đó, và chuỗi luật (hành động của A kích hoạt B, …) dừng sau 3 luật: lần chạy ghi `skipped` với `automations.loop_detected`. Luật chạy quá `AUTOXIDE_AUTOMATIONS_MAX_RUNS_PER_HOUR` lần trong một giờ (mặc định 1000) tự tạm dừng (`status_reason`: `rate_limit`).

## Cảnh báo khi số liệu bất thường

Kích hoạt `metric` kiểm một số liệu mỗi ngày lúc `hour:minute` (múi giờ của luật), so với ngưỡng cố định (`below`, `above`) hay với mức thường — trung bình của cùng thứ trong 4 tuần trước (`drop_pct`, `rise_pct`, tính phần trăm):

```json
{"type": "metric", "metric": "fact:<kind>", "currency": "VND", "operator": "drop_pct", "value": 30, "hour": 8}
```

- `fact:<kind>`: tổng theo ngày của một số liệu báo cáo do module khai (ngày địa phương vừa hết).
- `summary:<id>`: một số liệu tóm tắt, đọc lúc kiểm với quyền của người sở hữu luật; tỉ lệ tính bằng phần vạn (1% = 100).

Thống kê đơn giản, tất định, và `event.explanation` của lần chạy nói lý do, vd "… was 12000000, 45% lower than the average of the 4 previous Mondays: 21800000". Chưa đủ bốn tuần dữ liệu thì luật không báo, và mỗi ngày báo nhiều nhất một lần. Mẫu luật dựng sẵn (do module sở hữu số liệu khai) ở `GET /api/v1/automations/templates`.

<!-- example: templates -->
```bash
curl -sS "$API/api/v1/automations/templates" -H "Authorization: Bearer $TOKEN"
```
