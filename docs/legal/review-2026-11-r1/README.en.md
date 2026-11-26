# Legal review, round 1 — 2026-11-26

[Tiếng Việt (full report)](README.md)

> **This review was done by an AI, not by a lawyer. It is not legal advice.** An AI session played an independent
> two-person team — a technology lawyer (personal data, e-commerce, cybersecurity) and a Vietnamese tax and e-invoice
> specialist. Legal sources were found by web search; most are law-firm newsletters and press, not the official texts.
> Every conclusion must be checked against the original texts by a qualified professional before you rely on it.

Scope: the core and modules in this repository at commit `9b7d1a6de`, from the point of view of someone deploying
Autoxide for a real business in Viet Nam. The full findings table, with legal bases, links and `file:line` references,
is in the Vietnamese report.

## Findings

| Code | Level | Finding |
| --- | --- | --- |
| LG-1 | blocks 1.0 | The MOIT checklist and the compliance guide cite Decrees 52/2013 and 85/2021, which Decree 248/2026 (in force 1 July 2026, guiding E-commerce Law 122/2025) replaced. |
| LG-2 | blocks 1.0 | Invoice dates and order tax points use the UTC date. Orders placed between 00:00 and 07:00 Viet Nam time on 1 January 2027 would still get the expired 8% VAT rate. |
| LG-3 | should fix | Seller / livestreamer / affiliate identity checks exist but nothing calls them; the guide presents them as working. |
| LG-4 | should fix | Data-subject request deadlines (72 h for everything) come from the replaced Decree 13/2023; Decree 356/2025 sets per-request deadlines. |
| LG-5 | should fix | Nothing handles children's personal data (age, guardian consent). |
| LG-6 | should fix | AI providers (and mail, SMS, storage abroad) receive personal data abroad without appearing in the processing register. |
| LG-7 | should fix | The tax pack does not reflect the 2026 household-business tax reform (Decree 68/2026). |
| LG-8 | should fix | Platform withholding is computed but never triggered; no withholding certificates or filing data. |
| LG-9 | should fix | Cancelling an invoice sent through SePay always fails; no error notice (form 04/SS). |
| LG-10 | should fix | Invoices recompute tax at issue date and can silently differ from what the buyer paid. |
| LG-11 | should fix | Promotion limits cite Articles 6–7 of Decree 81/2018, repealed by Decree 239/2026. |
| LG-12 | should fix | The guide says review moderation asks `ModerationPolicy`; the code does not. |
| LG-13 | should fix | Cybersecurity and data-localisation citations predate Cybersecurity Law 116/2025. |
| LG-14…20 | recommended | Impact-assessment drafts, sensitive-data classification, posting verification warnings, Decree 248 retention and reports, VAT-reduction citations, upcoming anti-spam decree, fiscal-year invoice retention. |
| LG-21…24 | info | Breach playbook, consent and cookie handling, money flow (no buyer funds held), document retention: consistent with the sources found. |
