- Module `orders`: product reviews by buyers who received the item (one per order line, published at
  once, editable for 30 days), public list with star and photo filters and a "helpful" sort, star
  counts, "helpful" votes (one per person, not for your own review nor by the shop's moderators),
  hiding only with a listed reason (the author is told), shop replies, AI summary drafts that are
  public only once staff approve them, and a `verified_buyer` badge on product questions.
- Module `orders`: cash-on-delivery protection — delivery results per buyer kept as a per-tenant
  keyed hash of the phone number, rules with effective dates that require paying in advance for
  risky buyers or large orders, and time-limited block lists with listed reasons.
- Module `orders`: sales report by shop, channel, product, day or month within your scope, and order
  export through the shared list export.
