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
};

export interface ClientOptions {
  /** Where the server is, e.g. `https://api.example.com`. */
  baseUrl: string;
  /** An access token or an API key, sent as `Authorization: Bearer …`. */
  token?: string | (() => string | undefined);
  /** Language of error messages: `en` (the default) or `vi`. */
  language?: string;
  /** A `fetch` to use instead of the global one. */
  fetch?: typeof fetch;
}

/** An error answer of the API: RFC 9457 Problem Details. Branch on `code`, show `detail`. */
export class AutoxideError extends Error {
  readonly status: number;
  readonly code: string;
  readonly detail: string;
  readonly errors: NonNullable<Problem["errors"]>;
  /** Seconds to wait before retrying, from `Retry-After` (429, 503). */
  readonly retryAfter: number | undefined;
  readonly problem: Problem;

  constructor(problem: Problem, retryAfter: number | undefined) {
    super(`${problem.code}: ${problem.detail}`);
    this.name = "AutoxideError";
    this.status = problem.status;
    this.code = problem.code;
    this.detail = problem.detail;
    this.errors = problem.errors ?? [];
    this.retryAfter = retryAfter;
    this.problem = problem;
  }
}

export interface Client {
  request<M extends Method, P extends PathsWith<M>>(
    method: M,
    path: P,
    options?: RequestOptions<P, M>,
  ): Promise<Unwrapped<Success<Operation<P, M>>> | NotModified>;
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

  async function send(method: Method, path: string, request: RawOptions = {}): Promise<unknown> {
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
    const token = typeof options.token === "function" ? options.token() : options.token;
    if (token) headers.Authorization = `Bearer ${token}`;
    if (options.language) headers["Accept-Language"] = options.language;
    if (request.idempotencyKey) headers["Idempotency-Key"] = request.idempotencyKey;
    if (request.ifNoneMatch) headers["If-None-Match"] = request.ifNoneMatch;

    let body: BodyInit | undefined;
    const raw = request.body;
    if (raw instanceof Blob || typeof raw === "string" || raw instanceof FormData) {
      body = raw;
    } else if (raw !== undefined) {
      headers["Content-Type"] = "application/json";
      body = JSON.stringify(raw);
    }

    const response = await call(`${base}${url}`, { method: method.toUpperCase(), headers, body });
    if (response.status === 304) return { notModified: true } satisfies NotModified;
    const type = response.headers.get("Content-Type") ?? "";
    if (!response.ok) {
      const problem = type.includes("json")
        ? ((await response.json()) as Problem)
        : ({ type: "about:blank", title: response.statusText, status: response.status, code: "http_error", detail: await response.text() } as Problem);
      const after = response.headers.get("Retry-After");
      throw new AutoxideError(problem, after === null ? undefined : Number(after));
    }
    if (response.status === 204) return undefined;
    if (!type.includes("json")) return response.blob();
    const json = (await response.json()) as Record<string, unknown>;
    const keys = Object.keys(json);
    return keys.length === 1 && keys[0] === "data" ? json.data : json;
  }

  // Các kiểu theo operation đã kiểm lúc gọi; bên trong chỉ cần dạng thô.
  const raw = (request: unknown): RawOptions => (request ?? {}) as RawOptions;
  const client: Client = {
    request: (method, path, request) => send(method, path, raw(request)) as never,
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
