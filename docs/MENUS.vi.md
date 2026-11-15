# Menu của website (module `menus`)

Module `menus` giữ các menu của website: thanh menu chính, bảng xổ lớn nhiều cột, menu chân trang và mọi menu khác.
Một mục menu trỏ tới một địa chỉ, tới một đối tượng của bất kỳ module nào (bài viết, danh mục, danh mục sản phẩm,
khuyến mại, gian hàng của người bán), hoặc không trỏ đi đâu (nhóm tiêu đề gom các mục con). Website đọc một
endpoint công khai trả menu đã sẵn để hiện.

Tiếng Anh: [`MENUS.md`](MENUS.md).

> Module này là menu **trên website**. Menu của giao diện quản trị là thứ khác, không lưu ở đây.

## Permission

| Permission     | Cho phép                                         |
| -------------- | ------------------------------------------------ |
| `menus.read`   | Xem menu như đã lưu                              |
| `menus.manage` | Tạo menu, thay mục, xoá                          |

Đọc menu trên website (`/api/v1/menus/public/…`) không cần đăng nhập.

## Menu và mục

Menu có **khoá** (`main`, `footer`…: chữ thường, số, `-`, `_`) và **ngôn ngữ**. Mỗi khoá và ngôn ngữ có một menu.
Website hỏi `main` tiếng Việt; không có `main` tiếng Việt thì nhận bản ở ngôn ngữ mặc định.

Mỗi mục có:

| Trường | Nghĩa |
|---|---|
| `label` | Tối đa 200 ký tự. Có `ref` thì được để trống: mục hiện tiêu đề của đối tượng theo ngôn ngữ của menu, tự theo khi đối tượng đổi tên |
| `url` | `https://`, `http://`, `mailto:` hay đường dẫn bắt đầu bằng đúng một `/` (`//site-khac` và `\` bị từ chối) |
| `ref` | `{"kind": "content.term", "id": "…"}`: một đối tượng của module bất kỳ, theo loại và ID |
| `auto_children`, `auto_children_limit` | Hiện thêm mục con của chính đối tượng (vd danh mục con của một danh mục), 1 tới 50, mặc định 12 |
| `icon` | Một tên trong bộ cố định: `bag`, `bolt`, `book`, `calendar`, `cart`, `category`, `chat`, `fire`, `gift`, `grid`, `heart`, `help`, `home`, `info`, `mail`, `map_pin`, `news`, `percent`, `phone`, `search`, `shop`, `sparkles`, `star`, `store`, `tag`, `truck`, `user` |
| `highlight` | Làm nổi một mục (vd link khuyến mại dồn phải, màu riêng): `accent`, `brand`, `sale`, `muted` |
| `badge` | Nhãn nhỏ: `{"text": "-50%", "tone": "sale", "from": …, "until": …}`; tông `sale`, `brand`, `new`, `neutral`. Ngoài khoảng ngày thì tự ẩn |
| `feature` | Một đối tượng hiện nổi trong bảng xổ của mục (tiêu đề, tóm tắt, link, ảnh) |
| `children` | Mục con, cùng hình dạng |

Một mục có tối đa một trong `url` và `ref`; không có cả hai là nhóm tiêu đề. Menu tối đa 4 tầng và 200 mục (mục con
từ `auto_children` tính là một tầng).

Loại đối tượng dùng được bây giờ: `content.entry` (bài viết, trang) và `content.term` (danh mục, thẻ; danh mục có mục
con). Danh mục sản phẩm, khuyến mại, người bán, cửa hàng có khi module của chúng có.

## API quản trị

| Request | Làm gì |
|---|---|
| `GET /api/v1/menus` | Danh sách menu (phân trang bằng `limit`, `cursor`, `order`) |
| `POST /api/v1/menus` | Tạo menu: `{"key", "locale", "items"}`. 201 kèm `Location`; 409 `menus.menu_exists` nếu khoá và ngôn ngữ đã có |
| `GET /api/v1/menus/{id}` | Menu như đã lưu, kèm `ETag` |
| `PUT /api/v1/menus/{id}` | Thay cả cây mục: `{"items": […]}`. Gửi `ETag` đã đọc trong `If-Match` (`*` để ghi đè). Thiếu thì 428; người khác vừa sửa thì 412 |
| `DELETE /api/v1/menus/{id}` | Xoá menu và mục của nó |

Mọi lỗi của các mục báo một lần, mỗi lỗi kèm vị trí (`items[0].children[1].ref.id`). Khi mọi lỗi cùng một loại thì lỗi
có mã riêng: `menus.ref_unknown_kind` (không module nào cung cấp loại đó), `menus.ref_not_found` (không có đối tượng
đó), `menus.children_unsupported` (`auto_children` trên loại không có mục con).

AI agent làm cùng các việc đó qua MCP: `menus.list_menus`, `menus.get_menu` (trả `version`), `menus.create_menu`,
`menus.replace_menu_items` (cần `version`), `menus.delete_menu`.

## Trên website

```
GET /api/v1/menus/public/main?locale=vi
```

```json
{
  "data": {
    "key": "main",
    "locale": "vi",
    "items": [
      {
        "label": "Đồ gia dụng",
        "href": "/blog/category/do-gia-dung",
        "ref": {"kind": "content.term", "id": "…"},
        "icon": "category",
        "highlight": null,
        "badge": null,
        "feature": null,
        "children": [{"label": "Nhà bếp", "href": "/blog/category/nha-bep", "ref": {…}, "children": []}]
      },
      {"label": "Khuyến mại", "href": "/sale", "ref": null, "highlight": "sale",
       "badge": {"text": "-50%", "tone": "sale"}, "children": []}
    ]
  }
}
```

- Mục trỏ tới đối tượng không công khai (bản nháp, bài đã lưu trữ, danh mục ẩn) bị bỏ cùng mục con; nhóm tiêu đề
  không còn gì bên dưới cũng bị bỏ.
- `locale` của câu trả lời là ngôn ngữ thật đã trả.
- Sản phẩm bán chạy trong bảng xổ không thuộc menu: khi bảng xổ mở, gọi danh sách sản phẩm của catalog với `ref` của
  mục.
- Câu trả lời có `ETag` (gửi lại trong `If-None-Match` để nhận `304`) và `Cache-Control: public, max-age=…`.

### Mới tới mức nào?

Máy chủ giữ mỗi menu đã giải trong bộ nhớ tối đa `AUTOXIDE_MENUS_PUBLIC_CACHE_SECONDS` (mặc định 60), trình duyệt
được dùng lại chừng ấy. Lưu menu thì máy chủ vừa lưu xoá bản giữ ngay. Nhãn sắp bật hay tắt thì thời gian giữ ngắn lại,
nên nhãn hiện và ẩn đúng giờ. Danh mục đổi tên hay trang đổi đường dẫn thì hiện trong khoảng thời gian đó. Có nhiều
máy chủ thì mỗi máy giữ bản riêng, cũng tối đa chừng ấy.

## Cấu hình

| Biến | Mặc định | Nghĩa |
|---|---|---|
| `AUTOXIDE_MENUS_LOCALES` | `en,vi` | Ngôn ngữ được viết menu |
| `AUTOXIDE_MENUS_DEFAULT_LOCALE` | `en` | Ngôn ngữ trả về khi menu thiếu ở ngôn ngữ được hỏi |
| `AUTOXIDE_MENUS_PUBLIC_CACHE_SECONDS` | `60` | Giữ menu công khai sẵn và cho cache bao lâu (0 tới 3600; 0 là tắt) |

## Chuyển từ menu của `content`

Endpoint menu của module nội dung (`/api/v1/content/menus…`, `/api/v1/content/public/menus/{key}`) vẫn chạy nhưng đã
deprecated: câu trả lời mang `Deprecation`, `Sunset: 14/05/2027` và `Link` tới endpoint thay thế. Chép menu sang một
lần:

```bash
autoxide task menus.import_content_menus
```

Menu đã có ở đây (cùng khoá và ngôn ngữ) được để nguyên, nên chạy lại vẫn an toàn. Mục trỏ bài viết thành `ref`
`content.entry`; mục trỏ danh mục hay thẻ thành `content.term`. Sau đó cho website đọc
`GET /api/v1/menus/public/{key}`.

Lệnh chép menu của khách mặc định. Máy chủ có nhiều khách thì chạy một lần cho mỗi khách có menu cũ, nêu khách bằng
tên rút gọn: `autoxide task menus.import_content_menus tenant=shop-a`. Menu của khách nào là của khách đó: menu công
khai `main` của khách này không bao giờ trả cho khách khác.
