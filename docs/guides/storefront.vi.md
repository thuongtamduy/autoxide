# Dựng trang chủ của cửa hàng từ API

Bản tiếng Anh: [`storefront.md`](storefront.md). Ví dụ dùng `curl` với `API` là địa chỉ máy chủ của bạn (`export API=https://api.example.com`), và SDK TypeScript (`sdk/typescript`). Các ví dụ chạy như test ở mỗi bản dựng, nên luôn khớp máy chủ của phiên bản này.

Website tự vẽ trang; Autoxide cho nó dữ liệu: **banner** ở các vị trí có tên trên trang, **bộ sưu tập sản phẩm**, và **thương hiệu, thông tin liên hệ** của cửa hàng. Đổi banner hay bộ sưu tập là đổi dữ liệu (REST, hoặc AI agent qua MCP), không bao giờ phải sửa code của website. Tài liệu đầy đủ: [`docs/CONTENT.vi.md`](../CONTENT.vi.md) (banner), [`docs/CATALOG.vi.md`](../CATALOG.vi.md) (bộ sưu tập).

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
```

## Vị trí hiển thị: banner đặt ở đâu

Một **vị trí** (placement) là chỗ có tên trên trang (`top_strip`, `home_hero`, `sidebar_left`, `popup`…), nói banner của nó hiện thế nào (`single`, `carousel`, `grid`, `stack`, `strip`, `popup`), nhận ảnh tỉ lệ và cỡ nào, trên thiết bị và loại trang nào. Khách mới có sẵn bộ vị trí chuẩn; khách tạo trước tính năng này thì thêm các vị trí còn thiếu:

<!-- example: presets -->
```bash
curl -sS -X POST "$API/api/v1/content/placements/presets" -H "Authorization: Bearer $TOKEN"
```

## Một banner

Banner dạng chữ ở dải trên cùng, trỏ tới một trang của website. `tone` là vai màu của theme (`primary`, `secondary`, `accent`, `neutral`), không bao giờ là mã màu, nên đổi theme thì banner đổi màu theo. Banner ảnh nhận một ảnh **công khai** (`media_id`) đúng tỉ lệ của vị trí; link cũng trỏ được tới một đối tượng, vd `{"kind": "catalog.collection", "id": "…"}`.

<!-- example: create-banner -->
```bash
curl -sS "$API/api/v1/content/banners" -H "Authorization: Bearer $TOKEN" \
  --json '{"placement":"top_strip","locale":"en","kind":"text","title":"Free delivery over 500,000 VND","tone":"accent","link":{"url":"/shipping"},"status":"active"}'
# expect: 201
```

AI agent làm được việc tương tự bằng tool MCP `content.save_banner`, `content.order_placement`, `catalog.save_collection`: tool chỉ ghi **bản nháp** mang dấu `ai_assisted`, con người bật lên.

## Một bộ sưu tập

Bộ sưu tập gom sản phẩm cho một trang: chọn tay theo thứ tự (`manual`), hoặc theo quy tắc (danh mục, thuộc tính, khoảng giá, hàng mới, đang khuyến mại), sắp theo `newest`, `price_asc`, `price_desc` hay `best_selling`.

<!-- example: create-collection -->
```bash
curl -sS "$API/api/v1/catalog/collections" -H "Authorization: Bearer $TOKEN" \
  --json '{"kind":"rule","rules":{"new_within_days":30},"status":"active","texts":{"en":{"title":"New this month"}}}'
# expect: 201
```

## Trang chủ, như website đọc

Ba lời gọi công khai, không cần đăng nhập. Mọi vị trí của trang trong một lần gọi — banner đang hiệu lực, đúng thứ tự, `href` dựng sẵn và `variants` của ảnh cho `srcset`:

<!-- example: public-placements -->
```bash
curl -sS "$API/api/v1/content/public/placements?keys=top_strip,home_hero,home_middle,popup&locale=en&page_kind=home"
# save: PLACEMENTS_ETAG=header:etag
```

Thương hiệu, liên hệ, địa chỉ và giờ mở cửa của cửa hàng:

<!-- example: public-storefront -->
```bash
curl -sS "$API/api/v1/organizations/public/storefront"
```

Trang bộ sưu tập, kèm một trang sản phẩm (cùng thẻ sản phẩm như danh sách sản phẩm):

<!-- example: public-collection -->
```bash
curl -sS "$API/api/v1/catalog/public/collections/new-this-month?locale=en"
```

Cửa hàng bán trực tuyến ở Việt Nam còn hiện thông tin chủ sở hữu và logo Bộ Công Thương ở chân trang: `GET /api/v1/compliance/public/owner` và `GET /api/v1/compliance/public/badge`.

## Cache

Câu trả lời công khai có `ETag` (gửi lại trong `If-None-Match` để nhận `304`) và `Cache-Control` hết hạn đúng lúc banner kế tiếp bắt đầu hay kết thúc, nên CDN không giữ banner đã hết giờ:

<!-- example: placements-not-modified -->
```bash
curl -sS "$API/api/v1/content/public/placements?keys=top_strip,home_hero,home_middle,popup&locale=en&page_kind=home" -H "If-None-Match: $PLACEMENTS_ETAG"
# expect: 304
```

## Luật cho website

- Vẽ banner đúng thứ tự trả về; không xáo trộn.
- `popup` có nút đóng nhìn thấy được, theo `frequency`, và không hiện ở trang giỏ hàng, thanh toán, đăng nhập hay tài khoản.
- `carousel` không tự chạy khi người xem yêu cầu giảm chuyển động (`prefers-reduced-motion`) và dừng được.
- Banner và bộ sưu tập có `ai_assisted: true` mang nhãn "có hỗ trợ của AI" (Luật Trí tuệ nhân tạo — cần đối chiếu văn bản gốc).

## Với SDK

```ts
import { createClient } from "@autoxide/sdk";

const api = createClient({ baseUrl: "https://api.example.com" });

const [placements, shop, collection] = await Promise.all([
  api.get("/api/v1/content/public/placements", {
    query: { keys: "top_strip,home_hero,home_middle,popup", locale: "en", page_kind: "home" },
  }),
  api.get("/api/v1/organizations/public/storefront"),
  api.get("/api/v1/catalog/public/collections/{slug}", {
    path: { slug: "new-this-month" },
    query: { locale: "en" },
  }),
]);

for (const placement of placements.placements) {
  console.log(placement.key, placement.display, placement.banners.length);
}
console.log(shop.brand.name, collection.title, collection.ai_assisted);
```
