- Module `content`: `GET /api/v1/content/public/resolve` and preview links return a page's block layout with
  `blocks_data`, the data of its data blocks read once per block type; a failing block is `null` and the page still
  answers. The `ETag` covers the block data.
