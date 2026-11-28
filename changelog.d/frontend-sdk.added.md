- TypeScript SDK: `requestWithHeaders` gives an answer together with its status, `ETag` and headers, and the new
  `ifMatch` option sends `If-Match`. The new `refresh` option of `createClient` refreshes the access token when a
  request gets `401 auth.token_expired` and sends the request once more; requests that fail together share one
  refresh, since a refresh token works once. Existing calls are unchanged.
