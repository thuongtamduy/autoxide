# Building a web app with Autoxide

Vietnamese: [`web-app.vi.md`](web-app.vi.md). Examples use `curl` with `API` set to your server (`export API=https://api.example.com`), and the TypeScript SDK (`sdk/typescript`). They run as tests against every build, so they match the server of this version.

This guide is for people who build the front end — an admin screen, a shop, a website — in the browser,
with React, Vue, Svelte or plain TypeScript. Autoxide is only the API; your app is a separate project.
At the end there is a whole `api.ts` you can copy into your app.

- [1. Let the browser call the API (CORS)](#1-let-the-browser-call-the-api-cors)
- [2. Create the client](#2-create-the-client)
- [3. Sign in, stay signed in, sign out](#3-sign-in-stay-signed-in-sign-out)
- [4. Show only what the user may do](#4-show-only-what-the-user-may-do)
- [5. Change a record without overwriting someone else (ETag)](#5-change-a-record-without-overwriting-someone-else-etag)
- [6. Errors and retries](#6-errors-and-retries)
- [7. Upload a picture and show it](#7-upload-a-picture-and-show-it)
- [8. Write a post: the body format](#8-write-a-post-the-body-format)
- [9. Public pages: language and website](#9-public-pages-language-and-website)
- [10. Which endpoint for which task](#10-which-endpoint-for-which-task)
- [11. The whole `api.ts`](#11-the-whole-apits)

## 1. Let the browser call the API (CORS)

A page on another origin (`http://localhost:5173` while you develop, `https://admin.example.com` later)
may call the API only when the server lists it in `AUTOXIDE_HTTP_CORS_ORIGINS` (comma-separated; `*` for
any origin, not for production). Without it the browser blocks the answer, and the console says
"blocked by CORS policy". For the demo:

```bash
AUTOXIDE_HTTP_CORS_ORIGINS=http://localhost:5173 ./autoxide demo
```

With Docker, pass it with `-e AUTOXIDE_HTTP_CORS_ORIGINS=http://localhost:5173`. More in
[Getting started](../GETTING-STARTED.md#calling-the-api-from-a-browser-cors).

Credentials are sent in the `Authorization` header, never in cookies, so there is nothing to set for
cookies (`credentials: "include"` is not needed). JavaScript can read these headers of an answer: `ETag`,
`Location`, `Retry-After`, `X-Request-Id`, `Idempotent-Replayed`, `Content-Disposition`, `Content-Range`,
`Accept-Ranges`, `Link`, `Deprecation`, `Sunset` and `Mcp-Session-Id`.

## 2. Create the client

Copy `sdk/typescript/src` into your project or install the package of your server's release (see the
[SDK README](../../sdk/typescript/README.md)). Paths, parameters, bodies and answers are typed from the
API contract, so your editor completes them.

```ts
import { createClient } from "@autoxide/sdk";

// Public endpoints need no token.
const api = createClient({ baseUrl: "https://api.example.com", language: "vi" });

const posts = await api.get("/api/v1/content/public/posts", { query: { locale: "vi", limit: 10 } });
console.log(posts.data.map((post) => post.title));
```

`language` chooses the language of error messages (`detail`): `en` (the default) or `vi`. A `{"data": …}`
answer gives you the `data`; a list keeps `data` and `pagination` (see [Reading long lists](pagination.md)).

## 3. Sign in, stay signed in, sign out

`POST /api/v1/auth/login` answers `200` with an access token (about 15 minutes, `expires_in` seconds)
and a refresh token, or `202` with an `mfa_token` when the account has two-step sign-in: ask for the
6-digit code and send both to `POST /api/v1/auth/login/mfa`. Details: [Signing in](authentication.md).

**Where to keep the tokens.** The API uses bearer tokens, not cookies. Keep the access token in memory.
The refresh token must survive a reload to keep the user signed in: `localStorage` does that, but any
script that runs on your page can read it, so keep third-party scripts off the pages that hold it and set a
strict Content Security Policy. `sessionStorage` ends the session with the tab. If your framework has its
own server (Next.js, Nuxt, SvelteKit), it can keep the refresh token in an `HttpOnly` cookie of its own and
call the API for the browser instead.

**Refresh when the access token expires.** An expired access token answers `401` with code
`auth.token_expired`. Give the client a `refresh` function: the client then calls it, and sends the
request once more with the new token. A refresh token works **once**. If two requests refresh at the same
time with the same refresh token, the server sees a reused token, refuses it
(`auth.refresh_token_reused`) and **ends the whole session**. The client runs one `refresh` at a time for
the requests that fail together; open tabs share the stored token, so the `api.ts` below also takes a
[Web Lock](https://developer.mozilla.org/docs/Web/API/Web_Locks_API) around the refresh and reads the
stored refresh token inside it, so a tab never refreshes with a token another tab has already spent.

```ts
import { createClient } from "@autoxide/sdk";

let access: string | undefined;
let refreshToken = localStorage.getItem("refresh_token") ?? undefined;

// Login, refresh and logout go through a client without a token.
const auth = createClient({ baseUrl: "https://api.example.com" });

const api = createClient({
  baseUrl: "https://api.example.com",
  token: () => access,
  // Called once for all the requests that got `auth.token_expired` together; they are then sent again.
  refresh: async () => {
    if (!refreshToken) throw new Error("signed out");
    const next = await auth.post("/api/v1/auth/refresh", { body: { refresh_token: refreshToken } });
    access = next.access_token;
    refreshToken = next.refresh_token;
    localStorage.setItem("refresh_token", refreshToken);
  },
});

const me = await api.get("/api/v1/auth/me");
console.log(me.name);
```

Other `401` codes do not refresh: `auth.session_ended` (signed out, password changed, session revoked),
`auth.invalid_token`, `auth.missing_token`, and `auth.refresh_token_reused` or
`auth.invalid_refresh_token` from `/refresh` itself. Then forget the tokens and show the sign-in page.

**Sign out** with `POST /api/v1/auth/logout` and the refresh token (`204`). The session's access tokens
stop working at once. Forget both tokens even if the call fails (no network): the session then ends by
itself when the refresh token expires.

## 4. Show only what the user may do

`GET /api/v1/auth/me` lists the user's `permissions` (names such as `content.write`; the
[OpenAPI document](../openapi.json) and the module guides name the permission each endpoint needs).
When `all_permissions` is `true` (the `admin` role), `permissions` is **empty** and the user may do
everything: check `all_permissions` first.

```ts
import { createClient } from "@autoxide/sdk";

const api = createClient({ baseUrl: "https://api.example.com", token: () => sessionStorage.getItem("access") ?? undefined });

const me = await api.get("/api/v1/auth/me");
const can = (permission: string) => me.all_permissions || me.permissions.includes(permission);

const publishButton = document.querySelector<HTMLButtonElement>("#publish");
if (publishButton) publishButton.hidden = !can("content.publish");
if (me.mfa_required) {
  // A role of this user needs two-step sign-in: until it is on, that role's permissions are left out.
}
```

- Hiding a button is for comfort only: the server checks every request and answers `403`
  (`permission_denied`) when the permission is missing.
- An account made by self-registration (`POST /api/v1/auth/register`, on when the server sets
  `AUTOXIDE_AUTH_SELF_REGISTRATION=true`) has **no role and no permission** until an administrator gives it
  a role (`PUT /api/v1/auth/users/{id}/roles`, permission `auth.roles.manage`). Show such users a
  "waiting for access" screen instead of an empty admin.
- Read `/auth/me` again after a role change; a new access token is not needed.

## 5. Change a record without overwriting someone else (ETag)

Reading one record returns its version in the `ETag` header. Changing it takes that version back in
`If-Match`: a record someone changed meanwhile answers `412` (`precondition_failed`), and endpoints that
require it answer `428` (`precondition_required`) without it. `requestWithHeaders` gives you the
headers, and `ifMatch` sends the version:

```ts
import { AutoxideError, createClient } from "@autoxide/sdk";

const api = createClient({ baseUrl: "https://api.example.com", token: () => sessionStorage.getItem("access") ?? undefined });

async function rename(id: string, title: string) {
  const { data: entry, etag } = await api.requestWithHeaders("get", "/api/v1/content/entries/{id}", { path: { id } });
  console.log("was", entry.title);
  try {
    return await api.patch("/api/v1/content/entries/{id}", { path: { id }, ifMatch: etag, body: { title } });
  } catch (error) {
    if (error instanceof AutoxideError && error.code === "precondition_failed") {
      // Someone saved first: read again, show their version, let the user apply the change again.
    }
    throw error;
  }
}
```

The answer to a change carries the new `ETag` too (`requestWithHeaders("patch", …)`), so you can save
twice in a row without reading again. More in [Reading errors](errors.md#changing-a-record-if-match-428-and-412).

## 6. Errors and retries

Every error is a Problem with a stable `code`; the SDK throws it as an `AutoxideError` (`status`, `code`,
`detail`, `errors`, `retryAfter`, `requestId`). The [error table for apps](errors.md#what-your-app-should-do)
lists the common codes and what to do with each. In short:

| Status | What to do |
| ------ | ---------- |
| `401` | `auth.token_expired`: refresh (the client does it with `refresh`). Other codes: back to sign-in. `step_up_required`: ask for the password or code again (`POST /api/v1/auth/me/reauth`) |
| `403` | Not allowed: hide the action, or tell the user to ask for access |
| `404` | Gone or never existed: refresh the list |
| `412` / `428` | Read the record again, then apply the change to what you read |
| `422` | Show `errors[]` next to their fields (`field` is a path such as `items[0].url`) |
| `429` | Wait `Retry-After` seconds; never retry a sign-in automatically |

A write that fails on the network may still have happened. Send an `idempotencyKey` (a new
`crypto.randomUUID()` for each user action, the **same** one on each retry of it): a repeat returns the
first result instead of doing it twice. See [Retrying safely](retries.md).

```ts
import { AutoxideError, createClient } from "@autoxide/sdk";

const api = createClient({ baseUrl: "https://api.example.com", token: () => sessionStorage.getItem("access") ?? undefined });

async function withRetry<T>(send: () => Promise<T>, tries = 3): Promise<T> {
  for (let attempt = 1; ; attempt += 1) {
    try {
      return await send();
    } catch (error) {
      const busy = error instanceof AutoxideError && (error.status === 429 || error.status === 503);
      const network = error instanceof TypeError; // fetch failed: offline, DNS, connection reset
      if (attempt >= tries || !(busy || network)) throw error;
      const seconds = error instanceof AutoxideError ? (error.retryAfter ?? 1) : attempt;
      await new Promise((resolve) => setTimeout(resolve, seconds * 1000));
    }
  }
}

const key = crypto.randomUUID(); // one key for this one action, reused by every retry
const created = await withRetry(() =>
  api.post("/api/v1/content/entries", { body: { kind: "post", title: "Hello" }, idempotencyKey: key }),
);
console.log(created.id);
```

## 7. Upload a picture and show it

Send the file itself as the body of `POST /api/v1/media?name=…` (not a multipart form); the browser sets
`Content-Type` from the `File`. A new item is `private`: change `visibility` to `public` before using it
on a public page or in a post that will be published. The `url` of an item is **relative**
(`/api/v1/media/…/content?v=…`): put your server in front of it.

```ts
import { createClient } from "@autoxide/sdk";

const API_URL = "https://api.example.com";
const api = createClient({ baseUrl: API_URL, token: () => sessionStorage.getItem("access") ?? undefined });

async function uploadPicture(file: File, alt: string) {
  const uploaded = await api.post("/api/v1/media", {
    query: { name: file.name },
    body: file,
    idempotencyKey: crypto.randomUUID(),
  });
  const { etag } = await api.requestWithHeaders("get", "/api/v1/media/{id}", { path: { id: uploaded.id } });
  const media = await api.patch("/api/v1/media/{id}", {
    path: { id: uploaded.id },
    ifMatch: etag,
    body: { visibility: "public", alt },
  });
  return { id: media.id, src: new URL(media.url, API_URL).href };
}

const input = document.querySelector<HTMLInputElement>("input[type=file]");
const file = input?.files?.[0];
if (file) console.log((await uploadPicture(file, "A lake at dawn")).src);
```

A private item's `url` needs the `Authorization` header (without it the answer is `404`), so an `<img>`
cannot show it; show a preview from the `File` (`URL.createObjectURL(file)`) until it is public. Smaller copies (`variants`, with WebP)
appear a moment after the upload — see [Showing posts and pictures on a website](website.md). Files larger
than the server's upload limit go [straight to storage](direct-uploads.md).

## 8. Write a post: the body format

The `body` of a post or page is a **ProseMirror JSON document** — the shape [Tiptap](https://tiptap.dev)
and ProseMirror editors produce with `editor.getJSON()`, with the node names of Tiptap's StarterKit
(`paragraph`, `heading`, `bulletList`, `orderedList`, `listItem`, `blockquote`, `codeBlock`,
`horizontalRule`, `hardBreak`, `text`) plus tables (`table`, `tableRow`, `tableHeader`, `tableCell`).

```json
{"type": "doc", "content": [
  {"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": "Hello"}]},
  {"type": "paragraph", "content": [
    {"type": "text", "text": "Read the "},
    {"type": "text", "text": "guide", "marks": [{"type": "link", "attrs": {"href": "https://example.com"}}]}
  ]},
  {"type": "image", "attrs": {"mediaId": "0193a0c4-7d1e-7c2a-9b1e-3f2d5c6b7a81", "alt": "A lake", "caption": ""}}
]}
```

The server keeps only an allowlist, and refuses anything else with `422`, naming the place
(`body.content[2].attrs.href`):

- headings of level **2 to 4** only (the page title is the level 1);
- marks: `bold`, `italic`, `underline`, `strike`, `code`, `link` (`attrs.href` only; no `javascript:`);
- `image` takes `attrs.mediaId` (an item of the media library), `alt` and `caption` — **not** `src`; a
  Tiptap image extension needs a custom node or a mapping before saving;
- `embed` (`attrs.url` on a host the server allows, or `attrs.kind` + `attrs.id` of another module's
  record) is Autoxide's own node;
- unknown attributes are refused: turn off editor extras that add attributes (text alignment, colours,
  `orderedList` `type`) or remove them before saving.

The full list is the `BodyNode` schema in the [OpenAPI document](../openapi.json) (also at
`/api/docs` when the server turns it on), and [`CONTENT.md`](../CONTENT.md) explains drafts, publishing
and addresses. The server stores no HTML: render the JSON yourself (Tiptap's `generateHTML`, or your own
components), and use the `featured_media` / `body_media` of public answers for picture URLs.

## 9. Public pages: language and website

Public endpoints (`/api/v1/content/public/…`, `/api/v1/catalog/public/…`, `/api/v1/menus/public/…`) need
no sign-in and return only what is published.

- `locale` is **required** on most public reads — content lists, `resolve`, terms and the feed
  (`/content/public/posts`, `/content/public/resolve`…), and every catalog read
  (`/catalog/public/products`…): a call without it answers `422` (`validation_failed`, field `locale`).
  The OpenAPI document marks it required per endpoint; `/content/public/site` and `/menus/public/{key}`
  fall back to the default language.
- Each module has its own list of languages: `AUTOXIDE_CONTENT_LOCALES`, `AUTOXIDE_CATALOG_LOCALES`,
  `AUTOXIDE_MENUS_LOCALES` (`en,vi` by default; `AUTOXIDE_CONTENT_DEFAULT_LOCALE` and
  `AUTOXIDE_MENUS_DEFAULT_LOCALE` name the default). There is **no endpoint that lists them** yet: put the
  list in your app's configuration, next to the API address. A language that is not configured answers
  `422` (content: `locale_not_supported`, whose `detail` names the allowed ones; catalog:
  `catalog.unsupported_locale`).
- `site` chooses the website (an organization unit) when your pages are not served on that website's own
  domain; on a domain bound to a unit, the domain chooses it. `GET /api/v1/organizations/public/site`
  answers which website a domain is (its `unit_id` is the value for `site`).
- Public answers may be up to a minute old: see
  [Cache, and seeing a post you just published](website.md#cache-and-seeing-a-post-you-just-published).

## 10. Which endpoint for which task

The API has more than a thousand endpoints, grouped by module (each module's tag in `/api/docs`). The
ones a web app needs most:

| Task | Endpoint | Permission |
| ---- | -------- | ---------- |
| Sign in / second step / refresh / sign out | `POST /api/v1/auth/login`, `/login/mfa`, `/refresh`, `/logout` | — |
| Create an account (when allowed) | `POST /api/v1/auth/register` | — |
| Forgot password | `POST /api/v1/auth/password-reset`, then `/password-reset/confirm` with the token from the mail | — |
| Who am I, what may I do | `GET /api/v1/auth/me` | signed in |
| Change my name, language, time zone | `PATCH /api/v1/auth/me` | signed in |
| My signed-in devices | `GET /api/v1/auth/me/sessions` | signed in |
| Users and their roles | `GET /api/v1/auth/users`, `PUT /api/v1/auth/users/{id}/roles` | `auth.users.read`, `auth.roles.manage` |
| Roles | `GET`/`POST /api/v1/auth/roles` | `auth.roles.read`, `auth.roles.manage` |
| Upload, list, publish pictures | `POST`/`GET /api/v1/media`, `PATCH /api/v1/media/{id}` | `media.write`, `media.read` |
| List, create, edit posts and pages | `GET`/`POST /api/v1/content/entries`, `PATCH /api/v1/content/entries/{id}` | `content.read`, `content.write` |
| Publish, take down | `POST /api/v1/content/entries/{id}/publish`, `/unpublish` | `content.publish` |
| Categories and tags | `GET`/`POST /api/v1/content/terms` | `content.read`, `content.manage` |
| Website menus | `GET`/`POST /api/v1/menus`, `PUT /api/v1/menus/{id}`; public `GET /api/v1/menus/public/{key}` | `menus.read`, `menus.manage` |
| Website: list, page by address, frame | `GET /api/v1/content/public/posts`, `/public/resolve`, `/public/site` | — |
| Products of a shop | `GET /api/v1/catalog/public/products` | — |
| Contact forms | `POST /api/v1/forms/public/{key}/submissions` | — |
| Live updates | `GET /api/v1/kernel/events/stream` ([Server-Sent Events](realtime-events.md)) | signed in |

Shops have their own guides: [Opening a shop](shop.md) and [Building a shop's home page](storefront.md).
For everything else, search `docs/openapi.json` (or `/api/docs`) for the noun you need; each module has a
guide in [`docs/`](../MODULES.md).

## 11. The whole `api.ts`

One file for your app: the client, signing in (with the second step), refreshing safely across tabs,
signing out, permissions, saving with `If-Match`, retries, uploads and public pages. Change `API_URL` and
`LOCALES`.

```ts
// api.ts — the Autoxide API for a web app.
import { AutoxideError, createClient } from "@autoxide/sdk";

export const API_URL = "https://api.example.com";
/** The server's AUTOXIDE_CONTENT_LOCALES: no endpoint lists them. */
export const LOCALES = ["en", "vi"] as const;
const REFRESH_KEY = "autoxide.refresh_token";

let access: string | undefined;
let signedOutListener: () => void = () => {};

/** Called when the session ends (signed out elsewhere, refresh refused): show the sign-in page. */
export function onSignedOut(listener: () => void) {
  signedOutListener = listener;
}

function storedRefresh(): string | undefined {
  try {
    return localStorage.getItem(REFRESH_KEY) ?? undefined;
  } catch {
    return undefined; // storage blocked (private mode): the session lasts as long as the page
  }
}

function keep(tokens: { access_token: string; refresh_token: string } | undefined) {
  access = tokens?.access_token;
  try {
    if (tokens) localStorage.setItem(REFRESH_KEY, tokens.refresh_token);
    else localStorage.removeItem(REFRESH_KEY);
  } catch {
    // storage blocked: keep the access token in memory only
  }
}

const language = navigator.language.toLowerCase().startsWith("vi") ? "vi" : "en";

/** For sign-in, refresh, sign-out and public pages: no token. */
export const publicApi = createClient({ baseUrl: API_URL, language });

/** Runs `refresh` in one tab at a time (Web Locks), so two tabs never spend the same refresh token. */
async function oneTabAtATime(work: () => Promise<void>) {
  if (typeof navigator !== "undefined" && "locks" in navigator) {
    await navigator.locks.request("autoxide-refresh", work);
  } else {
    await work();
  }
}

async function refresh() {
  await oneTabAtATime(async () => {
    // Read inside the lock: a tab that refreshed just before saved the newest refresh token here.
    const current = storedRefresh();
    if (!current) throw new Error("signed out");
    try {
      keep(await publicApi.post("/api/v1/auth/refresh", { body: { refresh_token: current } }));
    } catch (error) {
      keep(undefined);
      signedOutListener();
      throw error;
    }
  });
}

/** For everything that needs a signed-in user. */
export const api = createClient({ baseUrl: API_URL, language, token: () => access, refresh });

export type SignIn = { signedIn: true } | { signedIn: false; mfaToken: string; methods: string[] };

export async function signIn(email: string, password: string): Promise<SignIn> {
  const answer = await publicApi.post("/api/v1/auth/login", { body: { email, password } });
  if ("mfa_token" in answer) return { signedIn: false, mfaToken: answer.mfa_token, methods: answer.methods };
  keep(answer);
  return { signedIn: true };
}

/** The second step: the 6-digit code of the authenticator app, or a recovery code. */
export async function signInWithCode(mfaToken: string, code: string) {
  keep(await publicApi.post("/api/v1/auth/login/mfa", { body: { mfa_token: mfaToken, code } }));
}

/** After a reload: true when a stored refresh token still works. */
export async function resume(): Promise<boolean> {
  if (!storedRefresh()) return false;
  try {
    await refresh();
    return true;
  } catch {
    return false;
  }
}

export async function signOut() {
  const refreshToken = storedRefresh();
  keep(undefined);
  if (refreshToken) {
    await publicApi.post("/api/v1/auth/logout", { body: { refresh_token: refreshToken } }).catch(() => undefined);
  }
}

/** `can("content.publish")`: for showing or hiding buttons. The server still checks every request. */
export async function permissions() {
  const me = await api.get("/api/v1/auth/me");
  return {
    me,
    can: (permission: string) => me.all_permissions || me.permissions.includes(permission),
    waitingForAccess: !me.all_permissions && me.permissions.length === 0,
  };
}

/** Retries a request on 429, 503 and network errors. Give writes an `idempotencyKey`. */
export async function withRetry<T>(send: () => Promise<T>, tries = 3): Promise<T> {
  for (let attempt = 1; ; attempt += 1) {
    try {
      return await send();
    } catch (error) {
      const busy =
        error instanceof AutoxideError &&
        (error.status === 503 || (error.status === 429 && error.code !== "auth.login_throttled"));
      if (attempt >= tries || !(busy || error instanceof TypeError)) throw error;
      const seconds = error instanceof AutoxideError ? (error.retryAfter ?? 1) : attempt;
      await new Promise((resolve) => setTimeout(resolve, seconds * 1000));
    }
  }
}

/** The fields a form shows next to its inputs, from a 422. */
export function fieldErrors(error: unknown): Record<string, string> {
  const out: Record<string, string> = {};
  if (error instanceof AutoxideError) for (const field of error.errors) out[field.field] = field.detail;
  return out;
}

/** Changes a post's title with the version just read; `false` when someone saved first (412). */
export async function renameEntry(id: string, title: string): Promise<boolean> {
  const { etag } = await api.requestWithHeaders("get", "/api/v1/content/entries/{id}", { path: { id } });
  try {
    await api.patch("/api/v1/content/entries/{id}", { path: { id }, ifMatch: etag, body: { title } });
    return true;
  } catch (error) {
    if (error instanceof AutoxideError && error.code === "precondition_failed") return false;
    throw error;
  }
}

/** Uploads a picture, makes it public and returns its full URL. */
export async function uploadPicture(file: File, alt: string) {
  const key = crypto.randomUUID();
  const uploaded = await withRetry(() =>
    api.post("/api/v1/media", { query: { name: file.name }, body: file, idempotencyKey: key }),
  );
  const { etag } = await api.requestWithHeaders("get", "/api/v1/media/{id}", { path: { id: uploaded.id } });
  const media = await api.patch("/api/v1/media/{id}", {
    path: { id: uploaded.id },
    ifMatch: etag,
    body: { visibility: "public", alt },
  });
  return { id: media.id, src: mediaUrl(media.url) };
}

/** Media URLs are relative to the server. */
export const mediaUrl = (url: string) => new URL(url, API_URL).href;

/** What to show at an address of the website. */
export async function page(path: string, locale: (typeof LOCALES)[number]) {
  const answer = await publicApi.get("/api/v1/content/public/resolve", { query: { locale, path } });
  switch (answer.type) {
    case "entry":
      return { kind: "entry" as const, entry: answer.entry };
    case "redirect":
      return { kind: "redirect" as const, to: answer.location, status: answer.status };
    default:
      return { kind: "gone" as const };
  }
}
```
