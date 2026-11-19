- Looking into one tenant (step 39): request log lines and the spans of background jobs, event deliveries and
  MCP tools carry `tenant_id` (also as a trace attribute with OpenTelemetry). Metrics still have no tenant label.
  `docs/DEPLOY.md` (Monitoring) shows how to filter by it.
