# Bookings (module `bookings`)

The `bookings` module takes appointments for services: spas, salons, clinics, studios, classes, sports courts.
Customers pick a service, a staff member (or anyone free) and a free time; the shop manages staff, rooms and
equipment, working hours, time off, reminders, cancellations and no-shows by its own policy, and can hold a booking
with a deposit paid through its licensed payment partner. It needs none of the selling modules.

Vietnamese: [`BOOKINGS.vi.md`](BOOKINGS.vi.md).

## Permissions

| Permission                 | Allows                                                                                  |
| -------------------------- | --------------------------------------------------------------------------------------- |
| `bookings.services.manage` | Services, staff, rooms, equipment, weekly schedules, time off and extra hours           |
| `bookings.manage`          | Every booking of the unit: book for customers, confirm, check in, no-show, cancel, move |
| `bookings.read`            | Seeing every booking of the unit, without changing them                                 |
| `bookings.staff`           | Your own appointments (resources linked to your account): check in, done, no-show     |

All four work per organization unit (a shop or a branch). Customers need no permission: any signed-in user books
for themselves and sees their bookings at `GET /api/v1/bookings/me/bookings`. Guests book with a code sent to their
phone.

## Set up

1. **Time zone.** Working hours are local to the unit: set `organizations.timezone` (e.g. `Asia/Ho_Chi_Minh`) on the
   unit or its parent. Without it, UTC.
2. **Staff, rooms, equipment** — `POST /api/v1/bookings/resources` with `kind` `staff`, `room` or `equipment`. Link a
   staff member to their account (`user_id`) so they see their own appointments.
3. **Weekly schedule** — `PUT /api/v1/bookings/resources/{id}/schedule` with ranges such as
   `{"weekday": 1, "start_minute": 540, "end_minute": 1020}` (Monday 09:00–17:00 local time). Staff without a
   schedule are never free; rooms and equipment without one are always open.
4. **Time off and extra hours** — `POST /api/v1/bookings/exceptions` with `kind` `closed` (holiday, leave) or `open`
   (an extra shift), `starts_at`, `ends_at` (UTC).
5. **Services** — `POST /api/v1/bookings/services`: `duration_minutes`, `buffer_before_minutes` (setup) and
   `buffer_after_minutes` (cleaning), `price`, `deposit` (`none`, `percent` of the price, or a fixed `amount`),
   `capacity` (1 for a haircut, more for a class), `performers` (who can give it) and `required` (rooms or machines
   every booking also needs).

Durations and buffers are in steps of 5 minutes.

## Policy per unit

Organization settings, inherited down the tree:

| Setting                                | Default | Meaning                                                                      |
| -------------------------------------- | ------- | ---------------------------------------------------------------------------- |
| `bookings.free_cancel_hours`           | 24      | Cancel at least this many hours before the start: the whole deposit is refunded |
| `bookings.late_cancel_retain_percent`  | 100     | Share of the deposit kept on a later cancellation                            |
| `bookings.no_show_retain_percent`      | 100     | Share of the deposit kept on a no-show                                       |
| `bookings.require_approval`            | false   | Bookings without a deposit wait for the shop to confirm them                 |
| `bookings.hold_minutes`                | 10      | Minutes a booking is held while the customer pays the deposit                |

The availability answer repeats these terms, so the shop can show the price, deposit and cancellation conditions
before the customer books, as consumer protection law asks.

## Free times

`GET /api/v1/bookings/public/availability?service_id=…&from=…&to=…` (no sign-in; `resource_id` and `org_id` are
optional; at most 31 days at a time) lists free start times every 15 minutes of local time: the weekly schedule plus
extra hours, minus time off, minus what is already booked with its buffers. Each time has `starts_at` (UTC),
`local_start` (with the unit's offset) and `seats_left`. The answer is cached for 15 seconds, so a time shown may be
taken by the time you book.

## Booking

- **Customers** — `POST /api/v1/bookings/public/bookings` with `service_id`, `starts_at` (from the availability),
  optional `resource_id` (omitted: the first staff member free), `customer` (`name`, `phone`, `email`).
  - No deposit: `confirmed` at once, or `pending` when the unit approves bookings (`POST /bookings/{id}/confirm`).
  - Deposit: `held` for `bookings.hold_minutes`; the answer has a `checkout` (`checkout_url`, `qr_payload`) of the
    payment partner. When the partner reports the payment the booking becomes `confirmed`. Unpaid holds are released
    every minute and the time is free again; money that arrives after the hold ended is refunded in full.
- **Guests** — first `POST /api/v1/bookings/public/phone-codes` with `phone`; send the `challenge_id` and the 6 digits
  received by SMS as `phone_code` in the booking. A code works once, for 10 minutes. Guests cancel with a new code at
  `POST /api/v1/bookings/public/bookings/{id}/cancel`.
- **The shop** — `POST /api/v1/bookings/bookings` (phone calls, walk-ins): confirmed at once, no deposit.

Two people never get the same time: every booking takes 5-minute cells of each staff member, room and machine it
uses, under a unique key, in one transaction. The second request gets `409 bookings.slot_taken` — on SQLite and
PostgreSQL alike. A class counts its seats with one conditional update and answers `409 bookings.capacity_full` when
full.

## After booking

| Action | Who | Endpoint |
|---|---|---|
| Check in, done | shop, own staff | `POST /api/v1/bookings/bookings/{id}/check-in`, `/complete` |
| No-show (after the start) | shop, own staff | `POST /api/v1/bookings/bookings/{id}/no-show` — keeps `bookings.no_show_retain_percent` of the deposit |
| Cancel | shop (any time, full refund), customer (until the start, by policy) | `POST /api/v1/bookings/bookings/{id}/cancel` |
| Move | shop (any time), customer (within the free window) | `POST /api/v1/bookings/bookings/{id}/reschedule` |
| Calendar file | shop, staff, customer | `GET /api/v1/bookings/bookings/{id}/calendar.ics`, `GET /api/v1/bookings/me/bookings/{id}/calendar.ics` |

Refunds go through the payment partner, never from the platform's own money (`ledger`), with a fixed reference: a
retried refund is not paid twice.

## Reminders

With the `notifications` module, customers get a transactional reminder (`bookings.reminder`) 24 hours and 2 hours
before the start, on the channels they have (app inbox, email, SMS, Zalo ZNS, push). Bookings made within 24 hours
only get the 2-hour one. A reminder is never sent twice, even when the job runs again; a moved booking is reminded
again at its new time. The message links to the booking's calendar file.

## For AI agents (MCP)

| Tool | Permission |
|---|---|
| `bookings.availability` | — (read only) |
| `bookings.list_bookings` | `bookings.read` or `bookings.manage` (read only) |
| `bookings.create_booking` | `bookings.manage` |
| `bookings.reschedule_booking` | `bookings.manage` |
| `bookings.cancel_booking` | `bookings.manage` |

## Errors

| Code | Status | Meaning |
|---|---|---|
| `bookings.slot_taken` | 409 | The time was just taken; ask for the free times again |
| `bookings.capacity_full` | 409 | The class is full |
| `bookings.outside_hours` | 422 | Outside the working hours of the staff member or room |
| `bookings.too_late_to_cancel` | 409 | The booking has started |
| `bookings.too_late_to_change` | 409 | Customers move bookings only within the free cancellation window |
| `bookings.invalid_status` | 409 | Not possible in the booking's status |
| `bookings.code_invalid` | 422 | Guest code wrong, used or expired |
| `bookings.payments_unavailable` | 503 | The service asks for a deposit but payments are not set up |

## Legal notes

Check with the original texts or a legal professional: consumer protection (Law 19/2023/QH15, Decree 55/2024) asks
that price, deposit and cancellation terms are shown before booking; deposits go through a licensed payment
service provider (Decree 52/2024); clinics have their own rules on medical records (Law 15/2023/QH15) — `bookings`
stores no health information, notes are short text only.
