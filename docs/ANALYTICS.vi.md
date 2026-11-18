# Phân tích truy cập

*English: [ANALYTICS.md](ANALYTICS.md)*

Module `analytics` đếm lượt truy cập website của bạn ngay trong Autoxide: bao nhiêu người ghé, xem
trang và đối tượng nào (sản phẩm, bài viết), đến từ đâu (tìm kiếm, mạng xã hội, chiến dịch UTM) và
phễu như "sản phẩm → giỏ → đơn đã trả tiền" rơi ở bước nào. Không gửi gì cho bên thứ ba, không đặt
cookie, chỉ giữ lâu số đã gộp theo ngày.

## Cách hoạt động

1. Trang của website gửi sự kiện tới `POST /api/v1/analytics/public/events`.
2. Module khác khai loại sự kiện (vd `catalog.product_viewed`) và phễu của mình; sự kiện máy chủ
   biết rõ hơn (đơn đã trả tiền) lấy từ event của chính module đó, không bao giờ từ trình duyệt.
3. Mỗi giờ một job cộng sự kiện thành số gộp theo ngày của từng website. Báo cáo chỉ đọc số gộp nên
   vẫn nhanh khi website lớn. Số của hôm nay chậm tối đa một giờ.
4. Sự kiện lẻ bị xoá sau `AUTOXIDE_ANALYTICS_RAW_RETENTION_DAYS` (7) ngày; số gộp sau
   `AUTOXIDE_ANALYTICS_RETENTION_DAYS` (790 ngày, hai năm hai tháng).

### Mặc định tôn trọng quyền riêng tư

- **Không cookie, không dấu vân tay trình duyệt.** Một lượt ghé được phân biệt bằng khoá phiên: bản
  băm có khoá của website, khối mạng của địa chỉ (IPv4 /24, IPv6 /64) và nhãn trình duyệt thô
  (`chrome|android|mobile`, không phiên bản). Khoá là ngẫu nhiên theo ngày và bị xoá khi ngày qua:
  không nối được phiên của hai ngày, không ai dò ngược ra địa chỉ.
- **Không lưu địa chỉ IP, không lưu user agent đầy đủ.** Query và fragment bị cắt khỏi đường dẫn
  trang (có thể chứa email); của trang giới thiệu chỉ giữ tên miền.
- **Theo dõi theo người chỉ khi đồng ý.** Người dùng đã đăng nhập và đồng ý mục đích
  `analytics.personal` trong sổ đồng ý của module `compliance` thì sự kiện gắn với tài khoản của họ,
  để module khác hiện "vừa xem". Rút đồng ý là ngừng ngay và gỡ phần gắn người khỏi sự kiện cũ; số
  gộp không chứa ai nên giữ nguyên.
- Khách trong nhiều ngày là tổng khách duy nhất của từng ngày: không cookie nên một người ghé hai
  ngày được đếm hai lần.

## Gửi sự kiện từ website

```js
const queue = [];
function track(type, extra = {}) {
  const params = new URLSearchParams(location.search);
  queue.push({
    type,
    path: location.pathname,
    referrer: document.referrer || undefined,
    utm: {
      source: params.get("utm_source") || undefined,
      medium: params.get("utm_medium") || undefined,
      campaign: params.get("utm_campaign") || undefined,
    },
    ...extra,
  });
}
function flush() {
  if (!queue.length) return;
  const events = queue.splice(0, 20);
  fetch("https://api.shop.example/api/v1/analytics/public/events", {
    method: "POST",
    keepalive: true, // vẫn gửi được khi trang đóng, như sendBeacon
    credentials: "include", // chỉ khi muốn gắn người dùng đã đăng nhập (có đồng ý)
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ events }),
  });
}
track("analytics.page_view");
// Trên trang sản phẩm:
track("catalog.product_viewed", { object: "catalog.product:0190…" });
addEventListener("visibilitychange", () => document.visibilityState === "hidden" && flush());
```

- Tối đa 20 sự kiện mỗi lần gọi. Luôn trả `202` kèm số `accepted` và `dropped`.
- `GET /api/v1/analytics/event-types` liệt kê mọi loại các module đã cài khai, kèm thuộc tính và
  loại đối tượng.
- Sự kiện bị bỏ (không bị từ chối) khi: loại chưa ai khai hay chỉ máy chủ ghi; `props` có tên chưa
  khai hay sai kiểu; thiếu `object` hay sai loại với loại sự kiện có đối tượng; bên gọi là bot đã
  biết hay không gửi user agent; trang gửi (`Origin`, không có thì `Referer`) không thuộc website;
  quá `AUTOXIDE_ANALYTICS_EVENTS_PER_MINUTE` (120) sự kiện mỗi phút từ cùng một khối mạng.
- Website là website của tên miền được gọi. Trang phục vụ từ tên miền khác API (một khách, chưa có
  tên miền xác minh) được nhận khi tên miền đó có trong `AUTOXIDE_ANALYTICS_ALLOWED_HOSTS`. Gọi API
  không qua tên miền của website thì thêm `?site=<id của nút>`.

## Báo cáo

Đều cần `analytics.read` tại nút tổ chức của website; `site` mặc định là website của cả khách,
`from` / `to` (ngày UTC) mặc định 30 ngày gần nhất, tối đa 400 ngày một lần.

| Endpoint | Nội dung |
|---|---|
| `GET /api/v1/analytics/overview` | khách, lượt xem, sự kiện, tỉ lệ thoát, nguồn hàng đầu, từng ngày |
| `GET /api/v1/analytics/pages` | trang xem nhiều nhất |
| `GET /api/v1/analytics/sources` | nguồn: `utm_source`, không có thì trang giới thiệu, không có thì `direct` |
| `GET /api/v1/analytics/campaigns` | giá trị `utm_campaign` |
| `GET /api/v1/analytics/objects?kind=catalog.product` | đối tượng xem nhiều nhất |
| `GET /api/v1/analytics/types` | sự kiện theo loại |
| `GET /api/v1/analytics/funnels`, `/funnels/{key}` | phễu đã khai; số phiên mỗi bước |

**Tỉ lệ thoát** là phần lượt ghé chỉ xem đúng một trang. Các bước của **phễu** đếm độc lập (lượt
ghé không cookie không nối được với đơn do máy chủ ghi), nên bước sau có thể lớn hơn bước trước.

Tool MCP (chỉ đọc): `analytics.overview`, `analytics.top_pages`, `analytics.top_objects`,
`analytics.funnel`. Số liệu tóm tắt `analytics.visitors` (7 ngày qua) hiện ở trang tổng quan của
giao diện quản trị.

## Cho người viết module

```rust
use autoxide_kernel::analytics::{AnalyticsEvent, Funnel, Prop, PropKind, ServerHit};

const VARIANT: &[Prop] = &[Prop::new("variant", PropKind::Text)];

fn analytics_events(&self) -> Vec<AnalyticsEvent> {
    vec![
        AnalyticsEvent::browser("catalog.product_viewed", "A product page was shown.")
            .object_kind("catalog.product")
            .props(VARIANT),
        // Lấy từ event `orders.order_confirmed` của chính module.
        AnalyticsEvent::server("orders.purchased", "An order was paid.", "orders.order_confirmed", |event| {
            Ok(Some(ServerHit::new(event.occurred_at).object(format!("orders.order:{}", event.id))))
        }),
    ]
}

fn funnels(&self) -> Vec<Funnel> {
    vec![Funnel::new("orders.checkout", "From product to paid order.",
        &["catalog.product_viewed", "orders.cart_added", "orders.purchased"])]
}
```

Module khác đọc số qua `autoxide_analytics::api::Analytics`: `top_objects` (thịnh hành) và
`recently_viewed` (rỗng khi không có đồng ý).

## Cấu hình

| Biến | Mặc định | Ý nghĩa |
|---|---|---|
| `AUTOXIDE_ANALYTICS_ENABLED` | `true` | có nhận sự kiện không |
| `AUTOXIDE_ANALYTICS_RAW_RETENTION_DAYS` | `7` | số ngày giữ sự kiện lẻ (2–90) |
| `AUTOXIDE_ANALYTICS_RETENTION_DAYS` | `790` | số ngày giữ số gộp (30–3650) |
| `AUTOXIDE_ANALYTICS_EVENTS_PER_MINUTE` | `120` | số sự kiện mỗi khối mạng mỗi phút |
| `AUTOXIDE_ANALYTICS_ALLOWED_HOSTS` | rỗng | tên miền website khác được gửi |

## Ghi chú pháp lý

Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 và NĐ 356/2025 đòi đồng ý theo mục đích trước khi gắn hành
vi với một người, và chỉ giữ thứ cần thiết. Số gộp ẩn danh theo ngày có là dữ liệu cá nhân không,
website không cookie có cần banner đồng ý không: chưa thống nhất — cần đối chiếu văn bản gốc / người
có chuyên môn pháp lý cho trường hợp của bạn. Tin quảng cáo dựa trên hành vi cần đồng ý (NĐ
91/2020/NĐ-CP).
