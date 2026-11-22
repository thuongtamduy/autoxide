# Adding blocks for pages built from blocks

Pages built from blocks (step 67) are trees of blocks: sections, headings, images, a product grid, a
form. A person drags them in an interface, or the AI page builder assembles them from a description;
both edit the same JSON layout. Every block is declared by a module. This guide is for developers who
add blocks to a module, so that a shop owner can put the module's features on any page.

Vietnamese: [`page-blocks.vi.md`](page-blocks.vi.md).

## The rules

1. **A block is data, not code.** It has a name, a group, a label, one sentence for the AI and a JSON
   Schema of its props. The website draws it; the server checks it. There is no way for a block to
   carry HTML, CSS or a script.
2. **The name is `<module>.<block>`**, e.g. `catalog.product_grid`, `forms.embed`. The label and the AI
   hint are English sentences, translated in the module's `locales/vi.json` under `text:<sentence>`;
   `cargo xtask check` refuses a sentence without a translation.
3. **The props schema is the only check.** It uses a small part of JSON Schema that the server checks
   itself (see "Props" below). Every object is closed (`additionalProperties: false`), so a page cannot
   hide anything the schema does not describe.
4. **Prices, stock and dates of a sale come from data, never from text.** A block that shows them
   declares `with_data()` and returns the current values when the page is read. The AI is told never
   to write them into text.
5. **Changing a block is like changing an API** (D-059): adding an optional prop is fine; removing a
   prop or changing its meaning is a new block (`…_v2`), and the old one keeps working.

## 1. Declare the block

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

Then return it from the module, next to its permissions and events:

```rust
impl Module for ShopModule {
    fn blocks(&self) -> Vec<autoxide_kernel::blocks::BlockType> {
        vec![PRODUCT_GRID]
    }
    // …
}
```

`with_children()` makes a block that holds other blocks (a section, columns, tabs).
`requires(feature)` limits it to tenants whose plan has a feature the module declares. A wrong name, a
schema the server cannot check or a missing translation stops the program at start, not at run time.

## 2. Props

The schema accepts `type`, `properties`, `required`, `additionalProperties`, `items`, `enum`, `const`,
`minLength` / `maxLength`, `minimum` / `maximum`, `minItems` / `maxItems`, `format` (`uuid`,
`date-time`), `title`, `description`, `default`, `examples`, and three keywords of Autoxide:

| Keyword | Meaning |
|---|---|
| `x-responsive: true` | The value is one value, or `{"base": …, "md": …, "lg": …}` by screen size. |
| `x-media: true` | The string is the id of a media item (an image or a video of the `media` module), never a URL. |
| `x-link: true` | A link with the rules of post bodies: `https://`, `http://`, `mailto:`, `tel:` or a path on the site (`/…`); never `javascript:`. |

Any other keyword is an error at start. Strings that look like HTML tags are refused. Colors are theme
tokens (`"primary"`), never color codes, so a page keeps the brand when the theme changes.

## 3. Data at read time

A block declared `with_data()` must give its data when a public page is read. Register a `BlockData`
in `Module::start`:

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

- `resolve` is called **once per block type per page**, with every block of that type: read them all
  in one query. Return one item per block, in the same order.
- Return only public data, filtered by `request.site` (the website being read) and `request.locale`.
  A record that is no longer public is `None`: the website hides that block and the page still shows.
- An error or a slow answer (over 2 seconds) turns only your blocks into `null`; the page still answers.
- The page is read through `GET /api/v1/content/public/resolve`; your data is in `blocks_data`, by
  block `id`.

## 4. What the AI sees

The AI page builder receives the list of blocks the tenant can use: each name, the AI hint and the
props schema. Write the hint as one sentence saying **when** to use the block and what it must not be
used for. Keep props few and named plainly; give `description` to props whose meaning is not obvious.
Whatever the AI answers goes through the same checks as a person's layout.

## 5. Test it

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

Also check that a block of a record that is not public answers `null`, and that the data of every
block of a page is read in one query. Run the test on SQLite and PostgreSQL, like every test.

## Checklist

- [ ] Name `<module>.<block>`, label and AI hint in English, translated in `locales/vi.json`.
- [ ] Closed props schema; media as `x-media`, links as `x-link`, colors as theme tokens.
- [ ] Prices, stock and sale dates come from `with_data()`, never from text.
- [ ] `BlockData` reads every block of a type in one query and returns only public data of the website.
- [ ] Tests on both databases.
