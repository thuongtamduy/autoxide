# Automating work with rules

Vietnamese: [`automations.vi.md`](automations.vi.md). Examples use `curl` with `API` set to your server (`export API=https://api.example.com`). They run as tests against every build, so they match the server of this version.

An automation rule says "**when** something happens **and** the conditions hold, **then** run these actions", without writing code. Each action is an MCP tool that the server already has, so a rule can do exactly what you could do yourself through the REST API or MCP with the same permissions — nothing more.

- **Trigger**: an event of a module (`{"type": "event", "event": "<module>.<event>"}`), a schedule (`{"type": "schedule", "every": "hour" | "day" | "week", "weekday": "monday", "hour": 9, "minute": 0}`), a manual call (`{"type": "manual"}`), or a figure leaving its usual range (`{"type": "metric", …}`, below).
- **Conditions**: a tree of `all` / `any` / `not` over comparisons `{"field": "event.total", "op": "gt", "value": 5000000}`. Operators: `eq`, `ne`, `gt`, `gte`, `lt`, `lte`, `in`, `contains`, `exists`. No free-form expressions: a rule is checked when you save it and behaves the same everywhere.
- **Actions**, in order: `{"tool": "<module>.<tool>", "arguments": {…}}`, `{"wait": {"minutes": 60}}` (up to 30 days), `{"stop_if": <condition>}`, `{"notify": {"title": "…", "body": "…"}}` (a notification to the rule's owner). Values may use templates: `"{{ event.code }}"` alone keeps the type of the value; text such as `"Order {{ event.code }}"` is filled as text. Templates see `event` (the trigger's data), `trigger` (what started the run) and `steps` (results of earlier steps: `steps.0.result`).

`GET /api/v1/automations/catalog` lists the events, metrics and tools you may use, with each tool's input schema.

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
```

<!-- example: catalog -->
```bash
curl -sS "$API/api/v1/automations/catalog" -H "Authorization: Bearer $TOKEN"
```

## Create a rule

Saving checks everything at once and reports every problem in `errors`: unknown events or tools (`automations.trigger_unknown`, `automations.tool_unknown`), tools you lack the permission for (`automations.permission_missing`), and arguments that do not fit a tool's input schema. A rule runs with the **current** permissions of the person it runs as (`run_as`: the author, or whoever last changed its actions or enabled it); if that person loses a permission the rule needs, the rule pauses itself. A rule also never uses a permission its `run_as` did not have when they saved or enabled it (the limit is by permission name; the scope is the person's current one). Rules saved before this limit existed get it the next time someone edits or enables them.

When every way into the `run_as` account is cut, the account's rules pause with `status_reason` `owner_access_revoked`. This happens when the account is locked out (for example its owner reported a sign-in that was not them), recovered by email, disabled or deleted — and also when its owner **changes their password and signs out everywhere**. Unlocking the account does not turn the rules back on: someone with the permissions checks each rule and enables it again (and becomes its `run_as`).

Runs follow the rule they belong to. Whenever a rule stops being `active` (paused for any reason, disabled, deleted), its runs still waiting (a `wait` step, an approval) end with `automations.rule_not_active` instead of continuing. A run that started before the rule was edited (even only renamed) or enabled again stops with `automations.rule_changed`: it never finishes old actions under a new definition or a new person responsible.

<!-- example: create-rule -->
```bash
curl -sS "$API/api/v1/automations/rules" -H "Authorization: Bearer $TOKEN" \
  --json '{"name":"Big orders","trigger":{"type":"manual"},"conditions":{"field":"event.total","op":"gte","value":5000000},"actions":[{"notify":{"title":"Big order {{ event.code }}","body":"Total: {{ event.total }}"}}]}'
# expect: 201
# save: RULE=.data.id
```

Rules made by an AI agent (an API key of kind `agent`) wait in `pending_approval` until a person enables them with `POST /api/v1/automations/rules/{id}/enable`; the rule then runs as that person.

## Try it without doing anything

A dry run evaluates the conditions on a sample, fills the templates and checks tools and permissions — it calls no tool.

<!-- example: dry-run -->
```bash
curl -sS "$API/api/v1/automations/rules/$RULE/test" -H "Authorization: Bearer $TOKEN" \
  --json '{"event_sample":{"total":6000000,"code":"DH1"}}'
```

## Run it and read the log

Runs happen in the background. Each run keeps a log of every step: status, result, error `code`, and the step's `Idempotency-Key` (`automation:<run id>:<step>`) — retrying a failed run (`POST /runs/{id}/retry`) starts at the failed step and never does a finished step twice.

<!-- example: run -->
```bash
curl -sS "$API/api/v1/automations/rules/$RULE/run" -H "Authorization: Bearer $TOKEN" \
  --json '{"input":{"total":6000000,"code":"DH1"}}'
# expect: 202
# save: RUN=.data.id
```

<!-- example: read-run -->
```bash
curl -sS "$API/api/v1/automations/runs/$RUN" -H "Authorization: Bearer $TOKEN"
```

<!-- example: list-runs -->
```bash
curl -sS "$API/api/v1/automations/runs?rule_id=$RULE" -H "Authorization: Bearer $TOKEN"
```

## Risky steps wait for a person

A step whose tool deletes or overwrites data (or needs approval when an agent calls it) stops the run in `awaiting_approval` each time. A person with `automations.run` and the tool's own permission approves (`POST /runs/{id}/approve`) or rejects (`POST /runs/{id}/reject`) it; the work inbox lists such runs (`automations.approvals`).

## Loops and limits

Events caused by a rule's actions never start the same rule again, and a chain of rules (A's action starts B, …) stops after 3 rules: the run is logged as `skipped` with `automations.loop_detected`. A rule that starts more than `AUTOXIDE_AUTOMATIONS_MAX_RUNS_PER_HOUR` runs in an hour (default 1000) pauses itself (`status_reason`: `rate_limit`).

## Alerts when a figure is unusual

A `metric` trigger checks a figure once a day at `hour:minute` (the rule's time zone) and compares it with a fixed threshold (`below`, `above`) or with its usual level — the average of the same weekday over the 4 previous weeks (`drop_pct`, `rise_pct`, a percentage):

```json
{"type": "metric", "metric": "fact:<kind>", "currency": "VND", "operator": "drop_pct", "value": 30, "hour": 8}
```

- `fact:<kind>`: the daily total of a report figure that a module declares (the previous whole day in local time).
- `summary:<id>`: a dashboard figure, read at check time with the rule owner's permissions; ratios in basis points (1% = 100).

The statistics are simple and deterministic, and the run's `event.explanation` says why, e.g. "… was 12000000, 45% lower than the average of the 4 previous Mondays: 21800000". Without four weeks of data the rule does not alert, and it alerts at most once per day. Ready-made templates (from the modules that own the figures) are at `GET /api/v1/automations/templates`.

<!-- example: templates -->
```bash
curl -sS "$API/api/v1/automations/templates" -H "Authorization: Bearer $TOKEN"
```
