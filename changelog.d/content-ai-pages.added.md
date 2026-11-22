- `content`: AI page builder — `POST /api/v1/content/entries/{id}/ai/compose` builds a page's block layout from a
  description and `…/ai/edit` changes it in words; both run in the background, go through the same layout checks as
  a person's (retried at most twice with the errors), and end as a draft marked `ai_assisted`, never published.
  `GET …/ai/thread` keeps the conversation with the revision before each change to undo it.
