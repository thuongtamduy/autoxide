- First-run polish from a newcomer's walkthrough of 0.20.0: `autoxide demo` checks the port before it prints the
  password or creates anything, and the error says how to pick another one; its banner shows the full path of the
  database and prints `127.0.0.1` in Docker too. `/api/docs` while the reference is off answers a Problem with the
  stable code `api_docs_disabled` and names `AUTOXIDE_HTTP_API_DOCS=true`. A JSON body with a missing field names it
  ("the required field `password` is missing") instead of repeating serde's sentence (the code stays `invalid_body`).
  An unknown permission name points to `GET /api/v1/auth/permissions`. The warning for a missing
  `AUTOXIDE_AUTH_TOKEN_SECRET` says what breaks and how to set it.
- Release archives now include `README.vi.md`, which `README.md` links to. Docs: how to read the result of
  `gh attestation verify`, the ready-made binary path in `docs/DEPLOY.md`, `AUTOXIDE_HTTP_API_DOCS=true` for
  `/api/docs`, and a first create-and-list request in `docs/GETTING-STARTED.md`.
