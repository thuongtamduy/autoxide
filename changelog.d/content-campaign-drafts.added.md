- `content`: campaign drafts from a promotion — `POST /api/v1/content/campaign-drafts` (and MCP tool
  `content.create_campaign_drafts`) drafts, in the background, a landing page built from blocks, a text banner linking
  to the promotion and an advertising `newsletter` campaign; each is a draft in its own module, never published or sent.
  Numbers in the copy must come from the promotion. New plan limit `content.ai_page_generations` (monthly) counts every
  AI page compose / edit and every campaign draft set.
