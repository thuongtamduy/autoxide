# Retrying safely

Vietnamese: [`retries.vi.md`](retries.vi.md). Examples use `curl` with `API` set to your server (`export API=https://api.example.com`), and the TypeScript SDK (`sdk/typescript`). They run as tests against every build, so they match the server of this version.

A network error after you sent a write leaves you unsure whether it happened. Send an `Idempotency-Key` header (a random UUID per logical operation) with every write: a repeat with the same key within 24 hours returns the **first** result, with `Idempotent-Replayed: true`, instead of doing it twice.

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
# save: REFRESH=.data.refresh_token
```

<!-- example: first-try -->
```bash
curl -sS "$API/api/v1/content/entries" -H "Authorization: Bearer $TOKEN" \
  -H 'Idempotency-Key: 0193e3a4-7b1c-7f00-9a51-6f2b8c0d4e11' \
  --json '{"kind":"post","title":"Written once"}'
# expect: 201
# save: ID=.data.id
```

<!-- example: retry-gets-the-first-result -->
```bash
curl -sS "$API/api/v1/content/entries" -H "Authorization: Bearer $TOKEN" \
  -H 'Idempotency-Key: 0193e3a4-7b1c-7f00-9a51-6f2b8c0d4e11' \
  --json '{"kind":"post","title":"Written once"}'
# expect: 201
# expect-header: idempotent-replayed: true
```

Using the same key for a **different** request is a mistake the server refuses (`idempotency_key_reused`, 422): make a new key for each distinct operation.

<!-- example: same-key-other-request -->
```bash
curl -sS "$API/api/v1/content/entries" -H "Authorization: Bearer $TOKEN" \
  -H 'Idempotency-Key: 0193e3a4-7b1c-7f00-9a51-6f2b8c0d4e11' \
  --json '{"kind":"post","title":"Something else"}'
# expect: 422
```

Notes:

- A request still running answers `409 idempotency_key_in_progress` with `Retry-After`; retry after that many seconds.
- A result that holds a secret shown once (a new API key) is not kept: a repeat answers `409 idempotency_result_not_kept`. Read the key list to see whether it was created.
- Business commands that move goods or money also take a reference of your own (`reference`), unique per operation, so even a retry after 24 hours is not done twice.

## With the SDK

```ts
import { AutoxideError, createClient } from "@autoxide/sdk";

const api = createClient({ baseUrl: "https://api.example.com", token: "axk1_…" /* read it from your secret store */ });

// One key per logical operation, reused on every retry of it.
const key = crypto.randomUUID();
for (let attempt = 1; ; attempt++) {
  try {
    const entry = await api.post("/api/v1/content/entries", {
      body: { kind: "post", title: "Written once" },
      idempotencyKey: key,
    });
    console.log(entry.id);
    break;
  } catch (error) {
    const retryable = error instanceof TypeError || (error instanceof AutoxideError && error.status >= 500);
    if (!retryable || attempt === 3) throw error;
    await new Promise((resolve) => setTimeout(resolve, attempt * 1000));
  }
}
```
