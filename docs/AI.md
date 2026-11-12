# AI features

*Tiếng Việt: [AI.vi.md](AI.vi.md)*

Autoxide can use an AI model, chosen and paid for by you, to help editors and AI agents:

| Feature | Where | Needs |
| ------- | ----- | ----- |
| SEO check (no AI) | `GET /api/v1/content/entries/{id}/seo-check` | nothing |
| SEO suggestions | `POST /api/v1/content/entries/{id}/seo-suggestions` | text model |
| Alt text suggestions | `POST /api/v1/media/{id}/alt-suggestions` | text model that reads images |
| Translate a post or page | `POST /api/v1/content/entries/{id}/translate` | text model |
| Search by meaning | `GET /api/v1/content/search`, `/public/search` | embeddings |
| Limits for AI agents | permissions `content.write_own`, `content.publish_own` | nothing |

**Everything is off until you turn it on.** No content leaves your server before you
configure a provider. Suggestions never save anything: a person (or an agent) reviews them
and saves with the usual `PATCH`. Translations are written as **drafts** and never published
automatically.

Every endpoint and its error codes are in [`openapi.json`](openapi.json) (and at `/api/docs`
when `AUTOXIDE_HTTP_API_DOCS=true`). The same features are MCP tools for AI agents:
`ai.status`, `content.seo_check`, `content.suggest_seo`, `content.translate`,
`content.get_translation_request`, `content.search`, `media.suggest_alt`.

## Turning on text generation

Set a provider, a key and (for `openai`) a model, then check the settings:

```bash
autoxide ai check
```

It sends one tiny request and says in plain words whether it works (a few tokens).

**Claude (Anthropic):**

```bash
AUTOXIDE_AI_PROVIDER=anthropic
AUTOXIDE_AI_API_KEY=sk-ant-…            # your key
# AUTOXIDE_AI_MODEL=claude-opus-5-5     # the default
```

When Claude's safety classifier declines a request, Anthropic reruns it on another model in
the same call (`AUTOXIDE_AI_ANTHROPIC_FALLBACK=true`, the default). Set it to `false` for a
model that does not support it, such as Claude Haiku.

**OpenAI:**

```bash
AUTOXIDE_AI_PROVIDER=openai
AUTOXIDE_AI_API_KEY=sk-…
AUTOXIDE_AI_MODEL=<an OpenAI model name>
# AUTOXIDE_AI_OPENAI_SEND_EFFORT=true   # only for reasoning models
```

**Google Gemini** (through its OpenAI-compatible endpoint):

```bash
AUTOXIDE_AI_PROVIDER=openai
AUTOXIDE_AI_BASE_URL=https://generativelanguage.googleapis.com/v1beta/openai
AUTOXIDE_AI_API_KEY=<your Gemini API key>
AUTOXIDE_AI_MODEL=<a Gemini model name>
```

**Ollama, on your own server** — nothing leaves the machine:

```bash
AUTOXIDE_AI_PROVIDER=openai
AUTOXIDE_AI_BASE_URL=http://localhost:11434/v1
AUTOXIDE_AI_MODEL=<a model you pulled>
AUTOXIDE_AI_IMAGES=false                # unless the model reads images
# AUTOXIDE_AI_OPENAI_SCHEMA_MODE=json_object   # if the server ignores JSON Schemas
# AUTOXIDE_AI_OPENAI_MAX_TOKENS_PARAM=max_tokens
```

vLLM, LM Studio and OpenRouter work the same way: `openai` plus their `AUTOXIDE_AI_BASE_URL`.

**How answers are checked.** Every request asks the model for JSON that follows a schema, and
the server checks every answer again — some servers ignore the schema without saying so. A
wrong answer is retried once, then reported (`ai.output_invalid`). Small local models fail
this more often; use a capable model for translation.

## Turning on embeddings (search by meaning)

Embeddings are separate from text generation: you may generate text with Claude and compute
embeddings with Ollama.

```bash
AUTOXIDE_EMBEDDING_PROVIDER=openai          # the /embeddings format
AUTOXIDE_EMBEDDING_BASE_URL=http://localhost:11434/v1
AUTOXIDE_EMBEDDING_MODEL=bge-m3
AUTOXIDE_EMBEDDING_DIMENSIONS=1024
```

The same format is served by OpenAI (`https://api.openai.com/v1`; send
`AUTOXIDE_EMBEDDING_SEND_DIMENSIONS=true` to shorten `text-embedding-3-*` vectors), Gemini
and Voyage AI (Anthropic has no embeddings API and points to Voyage). Pick a **multilingual**
model of about 1024 dimensions: `bge-m3` and `multilingual-e5-large` handle Vietnamese well;
models of 3072 dimensions use three times the memory.

Once on, every change of an entry updates its search index in the background, and an hourly
job indexes what is missing — after you turn embeddings on or change the model or the
dimensions (which rebuilds the whole index). Search keeps the vectors of a language in memory:
about 41 MB per 10 000 passages at 1024 dimensions (a 1 500-word post is 5–6 passages);
[`BENCHMARKS.md`](BENCHMARKS.md) has the measured search times.

Search combines meaning and words (accents and case ignored): each result says how it matched
(`matched_by`) and gives the best passage. With embeddings off, search matches words only.
`/api/v1/content/public/search` (for anonymous readers) is off unless
`AUTOXIDE_CONTENT_PUBLIC_SEMANTIC_SEARCH=true`; each public search costs an embeddings
request, so searches by meaning have a budget per minute
(`AUTOXIDE_CONTENT_PUBLIC_SEARCH_PER_MINUTE`, default 30) — past it, they match words only.

## Costs and limits

- **Daily token limit:** `AUTOXIDE_AI_DAILY_TOKEN_LIMIT` (default 2 000 000 tokens, text and
  embeddings together; `0` = no limit), counted in the database across all server processes.
  Past it, AI features answer `429 ai.quota_exceeded` until midnight UTC. `GET
  /api/v1/ai/status` (permission `ai.view`) shows today's use.
- **Metrics** (with `AUTOXIDE_HTTP_METRICS=true`): `ai_requests_total{purpose,outcome}`,
  `ai_tokens_total{purpose,direction}`, `ai_request_duration_seconds{purpose}`.
- **Rough cost** of translating a 1 500-word post (about 5 000 tokens in and 5 000 out), at
  Anthropic's prices of 3 November 2026: Claude Opus 5.5 ≈ 0.12 USD, Claude Sonnet 5.5 ≈
  0.06 USD, Claude Haiku 5.5 ≈ 0.003 USD. An SEO suggestion costs a fraction of that. A local
  model costs nothing per request.

## What is sent to the provider

| Feature | Sent |
| ------- | ---- |
| SEO suggestions | title, summary, plain text of the draft (first 12 000 characters), names of the categories and tags of its language |
| Alt text | a small copy of the image (at most 1280 px, never the original file), the language, the optional context you send |
| Translation | the text of the pinned revision in pieces — title, summary, SEO fields, image alt texts, paragraphs and headings with formatting as numbered tags; never links, image ids or code blocks |
| Search | the text of each entry in passages (indexing), and each search query |

**Private images** are sent only with `AUTOXIDE_MEDIA_AI_SEND_PRIVATE=true`. Logs never contain
prompts, content, images or keys — only the task, the model, token counts and timing.

## Translation in detail

`POST /api/v1/content/entries/{id}/translate` with `{"locale": "vi"}` answers `202` with a
request; a background job does the work:

1. It pins the entry as it is now (a revision): later edits do not change this translation.
2. The text is cut into pieces; formatting inside a paragraph becomes numbered tags
   (`<m1>bold words</m1>`, `<br2/>`) so word order can change between languages. The model
   never sees the JSON of the body, links or image ids.
3. The answer is checked: every piece back exactly once, every tag exactly once, nothing
   empty. A bad answer is retried once, then the request fails with
   `content.translation_invalid`.
4. The pieces are put back into the original structure (links and images come from the
   original) and saved as the **draft** of the translation: a new translation with a slug made
   from the translated title, or the existing translation — **only if nobody changed it since
   the request**; otherwise the request fails with `content.translation_conflict` and nothing
   is overwritten.

Follow it with `GET /api/v1/content/translation-requests/{id}` or webhooks
(`content.translation_succeeded`, `content.translation_failed`). The translation is marked
`ai_generated`; when the source changes afterwards, the admin API shows
`translation_outdated: true`. Code blocks and embeds are not translated; names, numbers and
addresses are kept.

## AI agents: limits and review

Give each agent its own API key, with a short expiry and only what it needs:

| The agent may… | Permissions |
| -------------- | ----------- |
| write drafts, a person publishes | `content.read`, `content.write_own` |
| write and publish its own posts | `content.read`, `content.write_own`, `content.publish_own` |
| also use the AI helpers | add nothing: SEO suggestions and translation follow `content.write_own`; alt text needs `media.write` |

- `_own` permissions reach only entries created with **the same key**; another key of the
  same user gets `403 content.not_owner`. Agents never archive, move, schedule or delete
  (deleting needs `content.delete`).
- When done, the agent asks for a review: `POST /api/v1/content/entries/{id}/request-review`
  (event `content.review_requested` — forward it to editors with a webhook).
- Requests with an API key are rate limited per key
  (`AUTOXIDE_HTTP_RATE_LIMIT_API_KEY_PER_MINUTE`, default 120), and every change is in the
  audit log with the key that made it.

## Telling readers about AI-generated content

Entries carry `ai_generated` (set by the server for its translations; agents should send
`"ai_generated": true` for what they write). When a **person** (signed in, not with an API
key) publishes such an entry after reviewing it, they send `{"ai_reviewed": true}` to
`POST /entries/{id}/publish`; the published version records who and when. The public
endpoints return `ai_generated` and `ai_reviewed`, so your website can show a notice where the
rules ask for one — for example Article 50 of the EU AI Act, which applies from 2 August 2026
to AI-generated text published to inform the public on matters of public interest; its
exception for human-reviewed text with editorial responsibility is read narrowly. Autoxide
records the facts; it does not give legal advice.

## When something goes wrong

| Code | Meaning | What to do |
| ---- | ------- | ---------- |
| `ai.disabled`, `ai.embedding_disabled` | not configured | set `AUTOXIDE_AI_*` / `AUTOXIDE_EMBEDDING_*` |
| `ai.misconfigured` | the provider refused the key or settings | run `autoxide ai check`; the server log names the HTTP status |
| `ai.provider_unavailable` | busy, rate limited or unreachable | retry after `Retry-After`; translations retry by themselves |
| `ai.quota_exceeded` | daily token limit reached | wait for midnight UTC or raise the limit |
| `ai.refused` | the model declined the content | change the content or do it by hand |
| `ai.output_truncated` | the answer was too long | lower `AUTOXIDE_CONTENT_TRANSLATE_BATCH_TOKENS` |
| `ai.output_invalid`, `content.translation_invalid` | wrong shape twice | retry, or use a more capable model |
| `ai.unsupported_input` | the model reads no images | set `AUTOXIDE_AI_IMAGES` right, or use a vision model |

## Settings

Every setting, with its default, is in [`.env.example`](../.env.example): `AUTOXIDE_AI_*`,
`AUTOXIDE_EMBEDDING_*`, `AUTOXIDE_MEDIA_AI_SEND_PRIVATE`,
`AUTOXIDE_CONTENT_TRANSLATE_BATCH_TOKENS`, `AUTOXIDE_CONTENT_PUBLIC_SEMANTIC_SEARCH`,
`AUTOXIDE_CONTENT_PUBLIC_SEARCH_PER_MINUTE`, `AUTOXIDE_HTTP_RATE_LIMIT_API_KEY_*`.
