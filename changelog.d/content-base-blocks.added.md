- `content`: base blocks for block-layout pages — columns, spacer, divider, text, button, FAQ, image, gallery,
  video, hero, announcement bar, countdown, email sign-up popup (always with consent and the privacy policy link),
  embedded record, search box and banner placement (`content.placement`, its scheduled banners in `blocks_data`).
- `compliance`: the legal footer block `compliance.legal_footer` (owner information and the ministry logo).
  `GET /api/v1/content/public/resolve` returns it as `legal_footer` below every block-layout page; a page cannot
  place or remove it.
