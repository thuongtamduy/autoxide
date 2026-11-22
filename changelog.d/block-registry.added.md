- Kernel: block registry for pages built from blocks. Modules declare blocks with `Module::blocks()`
  (name, category, label, AI hint, props JSON Schema, plan feature) and serve their data in batches through
  `BlockData`; `GET /api/v1/kernel/blocks` lists the blocks the tenant can use, with their schema.
