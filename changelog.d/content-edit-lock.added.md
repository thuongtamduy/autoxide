- `content`: edit locks for pages and posts — `POST /api/v1/content/entries/{id}/edit-lock` takes or renews a
  2-minute lock, `GET` shows who is editing and until when, `DELETE` releases it. Someone else's live lock gives
  `409 content.edit_locked`. The lock is advisory: saving still needs a matching `If-Match`.
