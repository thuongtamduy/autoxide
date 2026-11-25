- Multi-tenant mode: `kernel.public_url` and mail link settings (`<module>.link.*`) must be an `https`
  address, on the default port, on a domain the tenant has verified (or the platform's own host).
  Other values answer `422 kernel.public_url_not_allowed` / `kernel.link_host_not_allowed`; a value
  stored before is kept but ignored (mail uses the server's links) and flagged with
  `problem: kernel.public_url_unverified` / `kernel.link_unverified` in `GET /api/v1/kernel/settings`.
  **Action needed:** verify the domain (`POST /api/v1/kernel/domains`) before setting
  `kernel.public_url`. Single-tenant mode keeps the previous rule; a new `kernel.public_url` there
  must be `https` except on `localhost` / `127.0.0.1`.
