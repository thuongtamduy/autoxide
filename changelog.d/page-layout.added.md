- Module `content`: pages can have a block layout body (`{"type": "layout", …}`) instead of rich text, checked
  against the block registry (deny by default, values per screen size, visibility per screen size, one layout per
  translation). `PUT /api/v1/content/entries/{id}/layout` replaces it and `PATCH` changes part of it with JSON Patch
  (RFC 6902), both with `If-Match`; drafts, revisions, previews and schedules work as for rich text. First base
  blocks: `content.section`, `content.heading`.
