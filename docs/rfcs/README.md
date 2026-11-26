# RFCs

An RFC (request for comments) is a short design document, discussed in a pull request before the code is written. The
process is described in [`GOVERNANCE.md`](../../GOVERNANCE.md#proposing-a-change-the-rfc-process). RFCs are written in
English.

## When you need one

Write an RFC for:

- a new capability of the core, or a change of the public contract (kernel, testkit, a module's `api`, REST/MCP, settings);
- a convention change, or swapping a library;
- a change that is expensive to undo (data model, migrations, wire format);
- a new kind of module, or a change in how modules plug in.

You do not need one for bug fixes, documentation, a module that stays inside the current contract, or anything the
[roadmap](../ROADMAP.public.md) already describes in enough detail. If unsure, open an issue and ask.

## Status

| Status        | Meaning                                                    |
| ------------- | ---------------------------------------------------------- |
| `draft`       | Open pull request, under discussion                        |
| `accepted`    | Merged; will be (or is being) implemented                  |
| `implemented` | The change has shipped; link the release                   |
| `rejected`    | Closed with the reason, kept for the record                |
| `superseded`  | Replaced by a later RFC, which is linked                   |

## Index

| RFC                                  | Title | Status |
| ------------------------------------ | ----- | ------ |
| [0000](0000-template.md)             | Template | —   |
