// Kiểm hành vi chạy thật của client (không chỉ kiểu): `node --test test/` — Node 22.18+ tự bỏ kiểu của file
// `.ts`, nên không cần thêm thư viện. Thư mục này không nằm trong `tsconfig.json` (không cần `@types/node`);
// kiểu của API mới đã được kiểm qua ví dụ trong docs/guides (job tsc của CI).

import assert from "node:assert/strict";
import { test } from "node:test";

import { AutoxideError, createClient } from "../src/index.ts";

interface Sent {
  url: string;
  method: string;
  headers: Record<string, string>;
}

/** A fake `fetch` that answers from `reply` and records every request. */
function fakeFetch(reply: (sent: Sent, index: number) => Response | Promise<Response>) {
  const sent: Sent[] = [];
  const fetch = (async (input: string, init: RequestInit) => {
    const request: Sent = { url: input, method: String(init.method), headers: { ...(init.headers as Record<string, string>) } };
    sent.push(request);
    return reply(request, sent.length - 1);
  }) as unknown as typeof globalThis.fetch;
  return { fetch, sent };
}

const json = (status: number, body: unknown, headers: Record<string, string> = {}) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": status >= 400 ? "application/problem+json" : "application/json", ...headers },
  });

const expired = () =>
  json(401, { type: "about:blank", title: "Unauthorized", status: 401, code: "auth.token_expired", detail: "The access token expired." });

test("requestWithHeaders trả data, status, etag và headers", async () => {
  const { fetch } = fakeFetch(() => json(200, { data: { id: "n1", title: "A" } }, { ETag: '"v1"', "X-Request-Id": "r1" }));
  const api = createClient({ baseUrl: "https://api.test/", fetch });
  const answer = await api.requestWithHeaders("get", "/api/v1/notes/{id}", { path: { id: "n1" } });
  assert.deepEqual(answer.data, { id: "n1", title: "A" });
  assert.equal(answer.status, 200);
  assert.equal(answer.etag, '"v1"');
  assert.equal(answer.headers.get("X-Request-Id"), "r1");
});

test("requestWithHeaders: 304 cho { notModified: true } cùng etag", async () => {
  const { fetch, sent } = fakeFetch(() => new Response(null, { status: 304, headers: { ETag: '"v1"' } }));
  const api = createClient({ baseUrl: "https://api.test", fetch });
  const answer = await api.requestWithHeaders("get", "/api/v1/notes/{id}", { path: { id: "n1" }, ifNoneMatch: '"v1"' });
  assert.deepEqual(answer.data, { notModified: true });
  assert.equal(answer.status, 304);
  assert.equal(sent[0]?.headers["If-None-Match"], '"v1"');
});

test("ifMatch gửi header If-Match; 412 thành AutoxideError", async () => {
  const { fetch, sent } = fakeFetch(() =>
    json(412, { type: "about:blank", title: "Precondition Failed", status: 412, code: "precondition_failed", detail: "Changed since." }),
  );
  const api = createClient({ baseUrl: "https://api.test", fetch });
  await assert.rejects(
    api.patch("/api/v1/notes/{id}", { path: { id: "n1" }, body: { title: "B" }, ifMatch: '"v1"' }),
    (error: unknown) => error instanceof AutoxideError && error.status === 412 && error.code === "precondition_failed",
  );
  assert.equal(sent[0]?.headers["If-Match"], '"v1"');
});

test("refresh: auth.token_expired thì refresh một lần rồi gửi lại với token mới", async () => {
  let token = "old";
  let refreshes = 0;
  const { fetch, sent } = fakeFetch((request) =>
    request.headers.Authorization === "Bearer new" ? json(200, { data: { ok: true } }) : expired(),
  );
  const api = createClient({
    baseUrl: "https://api.test",
    fetch,
    token: () => token,
    refresh: async () => {
      refreshes += 1;
      token = "new";
    },
  });
  assert.deepEqual(await api.get("/api/v1/auth/me"), { ok: true });
  assert.equal(refreshes, 1);
  assert.deepEqual(
    sent.map((request) => request.headers.Authorization),
    ["Bearer old", "Bearer new"],
  );
});

test("refresh: nhiều request hết hạn cùng lúc chỉ refresh một lần", async () => {
  let token = "old";
  let refreshes = 0;
  const { fetch } = fakeFetch((request) =>
    request.headers.Authorization === "Bearer new" ? json(200, { data: { ok: true } }) : expired(),
  );
  const api = createClient({
    baseUrl: "https://api.test",
    fetch,
    token: () => token,
    refresh: async () => {
      refreshes += 1;
      await new Promise((resolve) => setTimeout(resolve, 20));
      token = "new";
    },
  });
  const answers = await Promise.all([api.get("/api/v1/auth/me"), api.get("/api/v1/auth/me"), api.get("/api/v1/auth/me")]);
  assert.equal(answers.length, 3);
  assert.equal(refreshes, 1);
});

test("refresh: request về muộn sau khi đã refresh xong thì chỉ gửi lại, không refresh nữa", async () => {
  let token = "old";
  let refreshes = 0;
  let releaseSlow: () => void = () => {};
  const slowGate = new Promise<void>((resolve) => {
    releaseSlow = resolve;
  });
  const { fetch } = fakeFetch(async (request, index) => {
    if (request.headers.Authorization === "Bearer new") return json(200, { data: { ok: true } });
    if (index === 1) await slowGate; // request thứ hai về muộn, sau khi refresh của request thứ nhất xong
    return expired();
  });
  const api = createClient({
    baseUrl: "https://api.test",
    fetch,
    token: () => token,
    refresh: async () => {
      refreshes += 1;
      token = "new";
    },
  });
  const first = api.get("/api/v1/auth/me");
  const second = api.get("/api/v1/auth/me");
  await first;
  releaseSlow();
  await second;
  assert.equal(refreshes, 1);
});

test("refresh: lỗi 401 khác (auth.session_ended) không refresh", async () => {
  let refreshes = 0;
  const { fetch } = fakeFetch(() =>
    json(401, { type: "about:blank", title: "Unauthorized", status: 401, code: "auth.session_ended", detail: "Signed out." }),
  );
  const api = createClient({
    baseUrl: "https://api.test",
    fetch,
    token: "t",
    refresh: async () => {
      refreshes += 1;
    },
  });
  await assert.rejects(api.get("/api/v1/auth/me"), (error: unknown) => error instanceof AutoxideError && error.code === "auth.session_ended");
  assert.equal(refreshes, 0);
});

test("refresh: refresh thất bại thì request báo lỗi của refresh; chỉ gửi lại một lần", async () => {
  const { fetch, sent } = fakeFetch(() => expired());
  const failing = createClient({
    baseUrl: "https://api.test",
    fetch,
    token: "t",
    refresh: async () => {
      throw new Error("signed out");
    },
  });
  await assert.rejects(failing.get("/api/v1/auth/me"), /signed out/);
  assert.equal(sent.length, 1);

  let token = "a";
  const again = fakeFetch(() => expired());
  const looping = createClient({
    baseUrl: "https://api.test",
    fetch: again.fetch,
    token: () => token,
    refresh: async () => {
      token += "a";
    },
  });
  await assert.rejects(looping.get("/api/v1/auth/me"), (error: unknown) => error instanceof AutoxideError && error.code === "auth.token_expired");
  assert.equal(again.sent.length, 2);
});

test("không có refresh thì auth.token_expired báo lỗi như cũ", async () => {
  const { fetch, sent } = fakeFetch(() => expired());
  const api = createClient({ baseUrl: "https://api.test", fetch, token: "t" });
  await assert.rejects(api.get("/api/v1/auth/me"), (error: unknown) => error instanceof AutoxideError && error.status === 401);
  assert.equal(sent.length, 1);
});
