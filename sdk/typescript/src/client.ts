// Hand-written, dependency-free client for the Autoxide HTTP API. The types come from
// `schema.ts`, generated from `docs/openapi.json` by `cargo xtask sdk`.

import type { Problem, paths } from "./schema.ts";

type Method = "get" | "put" | "post" | "patch" | "delete";

/** Paths that have an operation for `M`. */
export type PathsWith<M extends Method> = {
  [P in keyof paths]: paths[P] extends Record<M, unknown> ? P : never;
}[keyof paths];

type Operation<P extends keyof paths, M extends Method> = paths[P] extends Record<M, infer O> ? O : never;

type Parameters<O> = O extends { parameters: infer Q } ? Q : never;
type Group<O, K extends string> = Parameters<O> extends Record<K, infer G> ? G : never;
type Responses<O> = O extends { responses: infer R } ? R : never;
type Success<O> = {
  [S in keyof Responses<O>]: S extends 200 | 201 | 202 | 203 | 206 ? Responses<O>[S] : never;
}[keyof Responses<O>];

/** `{"data": X}` gives `X`; a page (`{data, pagination}`) stays whole. */
export type Unwrapped<T> = T extends { data: unknown; pagination: unknown }
  ? T
  : T extends { data: infer D }
    ? D
    : T;

/** The answer to a `GET` sent with `ifNoneMatch` when the resource has not changed (304). */
export interface NotModified {
  notModified: true;
}

/** Options as `send` reads them, without the per-operation types. */
interface RawOptions {
  path?: Record<string, unknown>;
  query?: Record<string, unknown>;
  body?: unknown;
  headers?: Record<string, string>;
  idempotencyKey?: string;
  ifNoneMatch?: string;
  ifMatch?: string;
  /** Set on the one repeat after a refresh, so a second `auth.token_expired` is not refreshed again. */
  retried?: boolean;
}

/** Options of one request. Path and query parameters are typed from the operation. */
export type RequestOptions<P extends keyof paths, M extends Method> = {
  /** Path parameters, e.g. `{ id: "…" }` for `/api/v1/notes/{id}`. */
  path?: Group<Operation<P, M>, "path">;
  /** Query parameters. */
  query?: Group<Operation<P, M>, "query">;
  /** The request body (JSON unless it is a `Blob`, a string or `FormData`). */
  body?: Operation<P, M> extends { requestBody?: infer B } ? B : never;
  /** Extra headers, e.g. `{"If-Match": etag}`. */
  headers?: Record<string, string>;
  /** Makes a write safe to retry: a random UUID per logical operation (see docs/guides/retries.md). */
  idempotencyKey?: string;
  /** The `ETag` you have: an unchanged resource answers `{ notModified: true }`. */
  ifNoneMatch?: string;
  /**
   * The `ETag` you read, sent as `If-Match`: the change applies only if nobody changed the record since
   * (otherwise `412 precondition_failed`). Get it with `requestWithHeaders`.
   */
  ifMatch?: string;
};

/** An answer with its status and headers, from `requestWithHeaders`. */
export interface WithHeaders<T> {
  /** What `request` would give: the `data`, a whole page, or `{ notModified: true }` on 304. */
  data: T;
  /** The HTTP status, e.g. `200`, `201` or `304`. */
  status: number;
  /** The `ETag` header: send it back as `ifMatch` to change the record, or as `ifNoneMatch` to re-read it. */
  etag: string | undefined;
  /** Every header of the answer (`Location`, `X-Request-Id`, `Idempotent-Replayed`…). */
  headers: Headers;
}

export interface ClientOptions {
  /** Where the server is, e.g. `https://api.example.com`. */
  baseUrl: string;
  /** An access token or an API key, sent as `Authorization: Bearer …`. */
  token?: string | (() => string | undefined);
  /** Language of error messages: `en` (the default) or `vi`. */
  language?: string;
  /** A `fetch` to use instead of the global one. */
  fetch?: typeof fetch;
  /**
   * Gets a new access token when the server answers `401 auth.token_expired`, then the request is sent
   * once more. Call `POST /api/v1/auth/refresh` here and store the new pair where `token` reads it.
   * Requests that fail together share one call: a refresh token works once, and two refreshes at the
   * same time would end the session (`auth.refresh_token_reused`). If it throws, the request rejects
   * with that error: sign the user in again. Only `auth.token_expired` triggers it.
   */
  refresh?: () => Promise<void>;
}

/** An error answer of the API: RFC 9457 Problem Details. Branch on `code`, show `detail`. */
export class AutoxideError extends Error {
  readonly status: number;
  readonly code: string;
  readonly detail: string;
  readonly errors: NonNullable<Problem["errors"]>;
  /** Seconds to wait before retrying, from `Retry-After` (429, 503). */
  readonly retryAfter: number | undefined;
  /** The `X-Request-Id` of the answer: quote it when you report a problem. */
  readonly requestId: string | undefined;
  readonly problem: Problem;

  constructor(problem: Problem, retryAfter: number | undefined, requestId?: string) {
    super(`${problem.code}: ${problem.detail}`);
    this.name = "AutoxideError";
    this.status = problem.status;
    this.code = problem.code;
    this.detail = problem.detail;
    this.errors = problem.errors ?? [];
    this.retryAfter = retryAfter;
    this.requestId = requestId;
    this.problem = problem;
  }
}

export interface Client {
  request<M extends Method, P extends PathsWith<M>>(
    method: M,
    path: P,
    options?: RequestOptions<P, M>,
  ): Promise<Unwrapped<Success<Operation<P, M>>> | NotModified>;
  /**
   * Like `request`, and also gives the status and headers of the answer, such as its `ETag`. `data` can be
   * `{ notModified: true }` only when you pass `ifNoneMatch`.
   */
  requestWithHeaders<M extends Method, P extends PathsWith<M>>(
    method: M,
    path: P,
    options: RequestOptions<P, M> & { ifNoneMatch: string | undefined },
  ): Promise<WithHeaders<Unwrapped<Success<Operation<P, M>>> | NotModified>>;
  requestWithHeaders<M extends Method, P extends PathsWith<M>>(
    method: M,
    path: P,
    options?: RequestOptions<P, M>,
  ): Promise<WithHeaders<Unwrapped<Success<Operation<P, M>>>>>;
  get<P extends PathsWith<"get">>(path: P, options?: RequestOptions<P, "get">): Promise<Unwrapped<Success<Operation<P, "get">>>>;
  post<P extends PathsWith<"post">>(path: P, options?: RequestOptions<P, "post">): Promise<Unwrapped<Success<Operation<P, "post">>>>;
  put<P extends PathsWith<"put">>(path: P, options?: RequestOptions<P, "put">): Promise<Unwrapped<Success<Operation<P, "put">>>>;
  patch<P extends PathsWith<"patch">>(path: P, options?: RequestOptions<P, "patch">): Promise<Unwrapped<Success<Operation<P, "patch">>>>;
  delete<P extends PathsWith<"delete">>(path: P, options?: RequestOptions<P, "delete">): Promise<Unwrapped<Success<Operation<P, "delete">>>>;
  /** Every item of a list, page after page, following `pagination.next_cursor`. */
  paginate<P extends PathsWith<"get">>(
    path: P,
    options?: RequestOptions<P, "get">,
  ): AsyncGenerator<Success<Operation<P, "get">> extends { data: (infer I)[] } ? I : never>;
}

/** A client for one server. */
export function createClient(options: ClientOptions): Client {
  const call = options.fetch ?? globalThis.fetch.bind(globalThis);
  const base = options.baseUrl.replace(/\/+$/, "");

  // Lần refresh đang chạy (nếu có): mọi request gặp `auth.token_expired` cùng lúc chờ chung một lần, vì
  // refresh token chỉ dùng được một lần — hai lần refresh song song làm server kết thúc cả phiên.
  let refreshing: Promise<void> | undefined;
  const currentToken = () => (typeof options.token === "function" ? options.token() : options.token);

  async function exchange(method: Method, path: string, request: RawOptions): Promise<WithHeaders<unknown>> {
    let url = path.replace(/\{([^}]+)\}/g, (_, name: string) => {
      const value = request.path?.[name];
      if (value === undefined) throw new Error(`missing path parameter \`${name}\` for ${path}`);
      return encodeURIComponent(String(value));
    });
    const query = new URLSearchParams();
    for (const [name, value] of Object.entries(request.query ?? {})) {
      if (value !== undefined && value !== null) query.set(name, String(value));
    }
    if ([...query.keys()].length > 0) url += `?${query}`;

    const headers: Record<string, string> = { Accept: "application/json, application/problem+json", ...request.headers };
    const token = currentToken();
    if (token) headers.Authorization = `Bearer ${token}`;
    if (options.language) headers["Accept-Language"] = options.language;
    if (request.idempotencyKey) headers["Idempotency-Key"] = request.idempotencyKey;
    if (request.ifNoneMatch) headers["If-None-Match"] = request.ifNoneMatch;
    if (request.ifMatch) headers["If-Match"] = request.ifMatch;

    let body: BodyInit | undefined;
    const raw = request.body;
    if (raw instanceof Blob || typeof raw === "string" || raw instanceof FormData) {
      body = raw;
    } else if (raw !== undefined) {
      headers["Content-Type"] = "application/json";
      body = JSON.stringify(raw);
    }

    const response = await call(`${base}${url}`, { method: method.toUpperCase(), headers, body });
    const answer = (data: unknown): WithHeaders<unknown> => ({
      data,
      status: response.status,
      etag: response.headers.get("ETag") ?? undefined,
      headers: response.headers,
    });
    if (response.status === 304) return answer({ notModified: true } satisfies NotModified);
    const type = response.headers.get("Content-Type") ?? "";
    if (!response.ok) {
      const problem = type.includes("json")
        ? ((await response.json()) as Problem)
        : ({ type: "about:blank", title: response.statusText, status: response.status, code: "http_error", detail: await response.text() } as Problem);
      const after = response.headers.get("Retry-After");
      const error = new AutoxideError(problem, after === null ? undefined : Number(after), response.headers.get("X-Request-Id") ?? undefined);
      if (error.status !== 401 || error.code !== "auth.token_expired" || !options.refresh || request.retried) throw error;
      // Một request khác đã refresh xong trong lúc request này đang đi: chỉ cần gửi lại với token mới.
      if (currentToken() === token) {
        refreshing ??= options.refresh().finally(() => {
          refreshing = undefined;
        });
        await refreshing;
      }
      return exchange(method, path, { ...request, retried: true });
    }
    if (response.status === 204) return answer(undefined);
    if (!type.includes("json")) return answer(await response.blob());
    const json = (await response.json()) as Record<string, unknown>;
    const keys = Object.keys(json);
    return answer(keys.length === 1 && keys[0] === "data" ? json.data : json);
  }

  async function send(method: Method, path: string, request: RawOptions = {}): Promise<unknown> {
    return (await exchange(method, path, request)).data;
  }

  // Các kiểu theo operation đã kiểm lúc gọi; bên trong chỉ cần dạng thô.
  const raw = (request: unknown): RawOptions => (request ?? {}) as RawOptions;
  const client: Client = {
    request: (method, path, request) => send(method, path, raw(request)) as never,
    requestWithHeaders: (method, path, request) => exchange(method, path, raw(request)) as never,
    get: (path, request) => send("get", path, raw(request)) as never,
    post: (path, request) => send("post", path, raw(request)) as never,
    put: (path, request) => send("put", path, raw(request)) as never,
    patch: (path, request) => send("patch", path, raw(request)) as never,
    delete: (path, request) => send("delete", path, raw(request)) as never,
    async *paginate(path, request) {
      let cursor: string | null | undefined;
      do {
        const options = raw(request);
        const query = { ...options.query, ...(cursor ? { cursor } : {}) };
        const page = (await send("get", path, { ...options, query })) as {
          data: unknown[];
          pagination: { next_cursor?: string | null };
        };
        for (const item of page.data) yield item as never;
        cursor = page.pagination.next_cursor;
      } while (cursor);
    },
  };
  return client;
}
