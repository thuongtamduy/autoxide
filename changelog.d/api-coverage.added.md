- `cargo xtask coverage-api [--run]` lists every endpoint in `docs/openapi.json` and every MCP tool in
  `docs/mcp-tools.json` that no test calls successfully; the testkit records calls when `TEST_API_COVERAGE`
  names a directory.
