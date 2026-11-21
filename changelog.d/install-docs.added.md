- Documentation: the README quickstart explains where the first administrator's password file is
  (`/var/lib/autoxide/data/initial-admin-password` in the data volume) and how to remove it, lists the
  modules the image contains, shows `If-Match` with `curl`, and notes curl 7.82 for `--json`, the
  build time from source and building behind a company proxy. `docs/DEPLOY.md` adds a minimal
  Kubernetes example (`deploy/kubernetes/autoxide.yaml`), encrypted off-site backups and
  point-in-time recovery on PostgreSQL.
