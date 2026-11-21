- Request logs and traces record the matched route template (`http.route`, e.g. `/api/v1/notes/{id}`) instead of the
  raw path; the span field `path` is removed.
