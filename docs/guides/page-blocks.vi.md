# Thêm khối cho trang dựng từ khối

Trang dựng từ khối (bước 67) là cây khối: phần, tiêu đề, ảnh, lưới sản phẩm, form. Người dùng kéo thả
trong giao diện, hoặc trình tạo trang bằng AI ghép từ một đoạn mô tả; cả hai sửa cùng một bố cục JSON. Mọi
khối do một module khai. Hướng dẫn này cho người phát triển thêm khối vào một module, để chủ cửa hàng đặt
được tính năng của module lên bất kỳ trang nào.

English: [`page-blocks.md`](page-blocks.md).

## Luật

1. **Khối là dữ liệu, không phải code.** Khối có tên, nhóm, nhãn, một câu cho AI và JSON Schema của props.
   Website vẽ khối; máy chủ kiểm khối. Không có đường nào để khối mang HTML, CSS hay script.
2. **Tên là `<module>.<khối>`**, vd `catalog.product_grid`, `forms.embed`. Nhãn và gợi ý cho AI là câu tiếng
   Anh, dịch trong `locales/vi.json` của module dưới khoá `text:<câu>`; `cargo xtask check` chặn câu chưa dịch.
3. **Schema props là chỗ kiểm duy nhất.** Schema dùng một phần nhỏ của JSON Schema mà máy chủ tự kiểm (xem
   "Props" dưới đây). Object nào cũng đóng (`additionalProperties: false`), nên trang không giấu được gì
   schema không tả.
4. **Giá, tồn kho và ngày khuyến mại lấy từ dữ liệu, không bao giờ từ chữ.** Khối hiện chúng khai
   `with_data()` và trả giá trị hiện tại lúc đọc trang. AI được dặn không bao giờ viết chúng vào chữ.
5. **Đổi khối giống đổi API** (D-059): thêm props tuỳ chọn thì được; bỏ props hay đổi nghĩa props là khối mới
   (`…_v2`), khối cũ vẫn chạy.

## 1. Khai khối

```rust
use autoxide_kernel::blocks::{BlockCategory, BlockType};

pub(crate) const PRODUCT_GRID: BlockType = BlockType::new(
    "shop.product_grid",
    BlockCategory::Commerce,
    "Product grid",
    "Use to show several products of a collection with their current prices; never write prices into text.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["collection_id"],
        "properties": {
            "collection_id": {"type": "string", "format": "uuid"},
            "limit": {"type": "integer", "minimum": 1, "maximum": 24},
            "columns": {"type": "integer", "minimum": 1, "maximum": 6, "x-responsive": true}
        }
    }"#,
)
.with_data();
```

Rồi trả nó từ module, cạnh permission và event:

```rust
impl Module for ShopModule {
    fn blocks(&self) -> Vec<autoxide_kernel::blocks::BlockType> {
        vec![PRODUCT_GRID]
    }
    // …
}
```

`with_children()` tạo khối chứa khối khác (phần, cột, tab). `requires(feature)` chỉ cho khách có tính năng
(do module khai) trong gói dùng khối. Tên sai, schema máy chủ không kiểm được hay câu chưa dịch làm chương
trình dừng ngay lúc khởi động, không đợi tới lúc chạy.

## 2. Props

Schema nhận `type`, `properties`, `required`, `additionalProperties`, `items`, `enum`, `const`, `minLength` /
`maxLength`, `minimum` / `maximum`, `minItems` / `maxItems`, `format` (`uuid`, `date-time`), `title`,
`description`, `default`, `examples`, và ba từ khoá riêng của Autoxide:

| Từ khoá | Nghĩa |
|---|---|
| `x-responsive: true` | Giá trị là một giá trị, hay `{"base": …, "md": …, "lg": …}` theo cỡ màn hình. |
| `x-media: true` | Chuỗi là ID của một mục media (ảnh hay video của module `media`), không bao giờ là URL. |
| `x-link: true` | Link theo luật của thân bài viết: `https://`, `http://`, `mailto:`, `tel:` hay đường dẫn trong site (`/…`); không bao giờ `javascript:`. |

Từ khoá khác là lỗi lúc khởi động. Chuỗi có dạng thẻ HTML bị từ chối. Màu là token của theme (`"primary"`),
không phải mã màu, để trang giữ đúng thương hiệu khi theme đổi.

## 3. Dữ liệu lúc đọc trang

Khối khai `with_data()` phải trả dữ liệu khi trang công khai được đọc. Gắn một `BlockData` trong
`Module::start`:

```rust
use std::sync::Arc;

use autoxide_kernel::blocks::{BlockData, BlockRef, BlockRequest};
use autoxide_kernel::error::Error;
use serde_json::Value;

pub(crate) struct Grids(pub(crate) Arc<ShopService>);

#[async_trait::async_trait]
impl BlockData for Grids {
    async fn resolve(
        &self,
        request: &BlockRequest<'_>,
        _block: &str,
        items: &[BlockRef],
    ) -> Result<Vec<Option<Value>>, Error> {
        // One query for every grid of the page, filtered like the module's public endpoints.
        let ids: Vec<String> = items
            .iter()
            .filter_map(|item| item.props["collection_id"].as_str().map(str::to_owned))
            .collect();
        let found = self.0.public_collections(request.tenant, request.site, request.locale, &ids).await?;
        Ok(items
            .iter()
            .map(|item| {
                let id = item.props["collection_id"].as_str()?;
                found.get(id).cloned()
            })
            .collect())
    }
}
```

```rust
fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
    // …
    context.block_data(PRODUCT_GRID.name, Arc::new(Grids(service.clone())))?;
    Ok(())
}
```

- `resolve` được gọi **một lần mỗi loại khối mỗi trang**, với mọi khối loại đó: đọc hết trong một câu truy vấn.
  Trả mỗi khối một phần tử, đúng thứ tự.
- Chỉ trả dữ liệu công khai, lọc theo `request.site` (website đang đọc) và `request.locale`. Bản ghi không còn
  công khai là `None`: website ẩn khối đó, trang vẫn hiện.
- Lỗi hay trả lời chậm (quá 2 giây) chỉ làm khối của bạn thành `null`; trang vẫn trả.
- Trang được đọc qua `GET /api/v1/content/public/resolve`; dữ liệu của bạn nằm ở `blocks_data`, theo `id` của khối.

## 4. AI thấy gì

Trình tạo trang bằng AI nhận danh sách khối khách dùng được: tên, gợi ý cho AI và schema props của từng khối.
Viết gợi ý thành một câu nói **khi nào** dùng khối và không được dùng nó vào việc gì. Giữ props ít, đặt tên dễ
hiểu; thêm `description` cho props không rõ nghĩa. Mọi thứ AI trả lời đều qua cùng bộ kiểm với bố cục của người.

## 5. Test

```rust
#[tokio::test]
async fn grid_returns_current_prices() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).module(shop_module()).build().await.unwrap();
        // … create a public collection …
        let tenant = app.default_tenant();
        let site = autoxide_kernel::tenancy::Site::whole(&tenant);
        let request = autoxide_kernel::blocks::BlockRequest::new(&tenant, &site, "en");
        let grid = autoxide_kernel::blocks::BlockRef::new("g1", serde_json::json!({"collection_id": collection}));
        let data = app.blocks().resolve(&request, &[("shop.product_grid", grid)]).await;
        assert_eq!(data["g1"]["products"][0]["price_minor"], 120_000);
    })
    .await
    .unwrap();
}
```

Kiểm thêm: khối của bản ghi không còn công khai trả `null`, và dữ liệu của mọi khối trên trang được đọc trong một
câu truy vấn. Chạy test trên SQLite và PostgreSQL như mọi test.

## Danh sách kiểm

- [ ] Tên `<module>.<khối>`, nhãn và gợi ý cho AI bằng tiếng Anh, có bản dịch trong `locales/vi.json`.
- [ ] Schema props đóng; media là `x-media`, link là `x-link`, màu là token của theme.
- [ ] Giá, tồn kho, ngày khuyến mại lấy từ `with_data()`, không bao giờ từ chữ.
- [ ] `BlockData` đọc mọi khối cùng loại trong một câu truy vấn và chỉ trả dữ liệu công khai của website.
- [ ] Test trên cả hai database.
