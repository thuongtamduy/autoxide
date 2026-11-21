- `cargo xtask check` checks the shape of every `/api/v1` endpoint in `docs/openapi.json`: request
  bodies refuse unknown fields, errors are `Problem`, lists are paged (`Page<T>` with `limit` and
  `cursor`); exceptions are listed with a reason in `xtask/api-shape-allow.txt`.
