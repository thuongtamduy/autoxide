# Website menus (module `menus`)

The `menus` module keeps the menus of your website: the main bar, mega menus with columns, the footer, and any
other menu. An item links to an address, to an object of any module (a post, a category, a product category, a
promotion, a seller's shop), or to nothing (a heading that groups sub-items). The website reads one public
endpoint that returns the menu ready to show.

Vietnamese: [`MENUS.vi.md`](MENUS.vi.md).

> This module is the menu **on your website**. The menu of an admin interface is a different thing and is not
> stored here.

## Permissions

| Permission     | Allows                                         |
| -------------- | ---------------------------------------------- |
| `menus.read`   | Seeing menus as they are saved                 |
| `menus.manage` | Creating menus, replacing their items, deleting |

Reading a menu on the website (`/api/v1/menus/public/…`) needs no sign-in.

## Menus and items

A menu has a **key** (`main`, `footer`…: lower-case letters, digits, `-`, `_`) and a **language**. There is one
menu per key and language. The website asks for `main` in Vietnamese; when there is no Vietnamese `main`, it gets
the one in the default language.

Each item has:

| Field | Meaning |
|---|---|
| `label` | Up to 200 characters. With `ref` it may be empty: the item then shows the object's title in the menu's language, and follows renames by itself |
| `url` | `https://`, `http://`, `mailto:` or a path starting with a single `/` (`//other.site` and `\` are refused) |
| `ref` | `{"kind": "content.term", "id": "…"}`: an object of any module, by resource type and id |
| `auto_children`, `auto_children_limit` | Also list the object's own sub-items (e.g. the sub-categories of a category), 1 to 50, default 12 |
| `icon` | A name from a fixed set: `bag`, `bolt`, `book`, `calendar`, `cart`, `category`, `chat`, `fire`, `gift`, `grid`, `heart`, `help`, `home`, `info`, `mail`, `map_pin`, `news`, `percent`, `phone`, `search`, `shop`, `sparkles`, `star`, `store`, `tag`, `truck`, `user` |
| `highlight` | Sets an entry apart (e.g. a promotions link pushed to the right in its own colour): `accent`, `brand`, `sale`, `muted` |
| `badge` | A small label: `{"text": "-50%", "tone": "sale", "from": …, "until": …}`; tones `sale`, `brand`, `new`, `neutral`. Outside its dates it hides by itself |
| `feature` | An object to feature in the item's drop-down panel (title, summary, link, picture) |
| `children` | Sub-items, same shape |

An item has at most one of `url` and `ref`; with neither it is a heading. A menu nests at most 4 levels and holds at
most 200 items (sub-items from `auto_children` count as a level).

Object types available now: `content.entry` (posts and pages) and `content.term` (categories and tags; categories
have sub-items). Product categories, promotions, sellers and shops come with their modules.

## Admin API

| Request | Does |
|---|---|
| `GET /api/v1/menus` | Lists menus (paged with `limit`, `cursor`, `order`) |
| `POST /api/v1/menus` | Creates a menu: `{"key", "locale", "items"}`. 201 with `Location`; 409 `menus.menu_exists` if the key and language are taken |
| `GET /api/v1/menus/{id}` | The menu as saved, with an `ETag` |
| `PUT /api/v1/menus/{id}` | Replaces the whole item tree: `{"items": […]}`. Send the `ETag` you read in `If-Match` (`*` to overwrite). 428 without it, 412 if someone changed the menu since |
| `DELETE /api/v1/menus/{id}` | Deletes the menu and its items |

All problems with the items are reported at once, each with its place (`items[0].children[1].ref.id`). When all
problems are of one kind the error has its own code: `menus.ref_unknown_kind` (no module provides that type),
`menus.ref_not_found` (no such object), `menus.children_unsupported` (`auto_children` on a type without sub-items).

AI agents use the same operations through MCP: `menus.list_menus`, `menus.get_menu` (returns `version`),
`menus.create_menu`, `menus.replace_menu_items` (needs `version`), `menus.delete_menu`.

## Menus of each website

With an organization tree (module `organizations`), a menu belongs to a website: `org_id` when
creating it (omitted: the website of the whole tenant). `menus.read` and `menus.manage` follow the
scope of the role, so a store manages only its own menus. The public endpoint picks the website
from the domain (a domain bound to a unit) or `?site=<unit id>`, and returns that website's menu —
or, when it has none with this key, the menu of the nearest website above (the whole tenant's
menu is the default for every store). The languages of a website's menus are the setting
`organizations.languages` of its unit (the first is its default), kept to the server's
`AUTOXIDE_MENUS_LOCALES`; unset, the server's.

## On the website

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

- Items pointing to objects that are not public (a draft, an archived post, a hidden category) are left out with
  their sub-items; a heading with nothing left under it is left out too.
- `locale` in the answer is the language actually returned.
- Best sellers in a drop-down panel are not part of the menu: when the panel opens, call the product list of the
  catalog with the item's `ref`.
- The answer has an `ETag` (send it back in `If-None-Match` to get `304`) and `Cache-Control: public, max-age=…`.

### How fresh is it?

The server keeps each worked-out menu in memory for `AUTOXIDE_MENUS_PUBLIC_CACHE_SECONDS` (default 60), and browsers
may reuse it as long. Saving a menu clears this at once on the server that saved it. A badge starting or ending cuts
the time short, so it shows and hides on time. A renamed category or a moved page shows within that time. With
several servers, each server keeps its own copy, for the same time at most.

## Settings

| Variable | Default | Meaning |
|---|---|---|
| `AUTOXIDE_MENUS_LOCALES` | `en,vi` | Languages a menu may be written in |
| `AUTOXIDE_MENUS_DEFAULT_LOCALE` | `en` | Language served when a menu is missing in the language asked for |
| `AUTOXIDE_MENUS_PUBLIC_CACHE_SECONDS` | `60` | How long a public menu is kept ready and may be cached (0 to 3600; 0 turns it off) |

## Moving from the menus of `content`

The menu endpoints of the content module (`/api/v1/content/menus…`, `/api/v1/content/public/menus/{key}`) still work
but are deprecated: their answers carry `Deprecation`, `Sunset: 14 May 2027` and a `Link` to the replacement. Copy
your menus once:

```bash
autoxide task menus.import_content_menus
```

Menus that already exist here (same key and language) are left alone, so running it again is safe. Items pointing
to posts become `ref` `content.entry`; items pointing to categories or tags become `content.term`. Then switch the
website to `GET /api/v1/menus/public/{key}`.

It copies the menus of the default tenant. On a server with several tenants, run it once per tenant that has old
menus, naming the tenant by its short name: `autoxide task menus.import_content_menus tenant=shop-a`. Each tenant's
menus stay its own: the public menu `main` of one tenant is never served to another.
