# The module ecosystem

**English** · [Tiếng Việt](MODULE_ECOSYSTEM.vi.md)

Autoxide's core is small and neutral; business features live in modules. Modules can live in this repository (the
official ones) or in **your own repository**, built against the stable contract
([`docs/COMPATIBILITY.md`](COMPATIBILITY.md)). This page says how to name, describe, publish and list a module so
that others can find it and know it will work. How to write one: [`docs/MODULE_GUIDE.md`](MODULE_GUIDE.md) and
[`docs/guides/writing-a-module.md`](guides/writing-a-module.md).

## Naming

- The crate is named **`autoxide-<name>`** (lowercase, hyphen-separated), for example `autoxide-bookmarks`. Tables start with
  `<name>_` ([the ownership rule](MODULE_GUIDE.md)), permissions with `<name>.`, error codes with `<name>.`.
- Names of the official modules in this repository are reserved. For your own, check
  <https://crates.io> and the [community list](#the-community-list) first: pick a name that says what the module does
  (`autoxide-shipping-ghn`, not `autoxide-best`). If a name is already taken by a module with the same job, consider
  contributing to it.
- A name with the `autoxide-` prefix says "this module plugs into Autoxide". It does not say "made or endorsed by the
  Autoxide project": see [`TRADEMARK.md`](../TRADEMARK.md).

## Discoverability

- Add the GitHub topic **`autoxide-module`** to the repository (Settings → About → Topics). The list of repositories
  with that topic is the always-fresh catalogue: <https://github.com/topics/autoxide-module>.
- Add the crates.io keywords `autoxide` and `autoxide-module` (`keywords` in `Cargo.toml`; crates.io allows five).
- Put `autoxide-module.toml` (below) at the root of the repository, and one line in the README: which Autoxide
  versions it works with.

## The module manifest (`autoxide-module.toml`)

A proposed file that describes a module for people and tools. It is **declarative metadata**: the Rust code stays the
source of truth, and a check in the CLI (`autoxide check`, planned in step 71 G6) will compare the file with the code. Until then the
file is read by humans and by the community list.

```toml
# autoxide-module.toml — at the root of the module repository
manifest_version = 1

[module]
name = "bookmarks"                      # the module name; the crate is autoxide-<name>
crate = "autoxide-bookmarks"
summary = "Save and tag links."        # one line, English
license = "MIT OR Apache-2.0"
repository = "https://github.com/example/autoxide-bookmarks"
tier = "community"                      # official | community  (stable/experimental is the contract tier of the core)

[compatibility]
autoxide = ">=0.19, <0.21"              # core versions this release was tested with (SemVer range)
rust-version = "1.99"                   # minimum Rust version
databases = ["sqlite", "postgres"]      # both are required for the list below

[provides]
permissions = ["bookmarks.read", "bookmarks.write"]
tables_prefix = "bookmarks_"
mcp_tools = ["bookmarks.list", "bookmarks.create"]
events = ["bookmarks.created"]

[requires]
# traits of other modules this one needs (context.require::<dyn …>())
modules = []
```

Rules: `databases` must list both `sqlite` and `postgres` to be accepted in the community list (the core's first rule);
`autoxide` is a range of released versions that you have tested, not "any".

## Version compatibility

A module depends on a released Autoxide (`autoxide-kernel`, `autoxide-testkit`, `autoxide` pinned to a tag or version).

| Autoxide core | Module contract            | What a module author should do                                            |
| ------------- | -------------------------- | ------------------------------------------------------------------------- |
| `0.x`         | May change in a minor release (with `#[deprecated]` first) | Pin an exact minor (`=0.19`) and test on each new minor |
| `1.x`         | Stable: breaks only in a major release | Depend on `1` (caret); declare `>=1.0, <2.0`                    |
| `2.0`         | New major                  | Port, release a new major of the module                                  |

A module release that works with several core versions lists the range in `[compatibility]` and in CI (test against
the lowest and the newest version in the range). Raise the range only after the test suite passes on the new version.

## Listing your module

The **community list** has no gatekeeper beyond these checks, so that anyone can publish:

1. The module is in a public repository under an OSI-approved open-source license.
2. `cargo xtask check-module <dir>` from an Autoxide checkout passes on it (the module rules of the core: ownership of
   tables, error codes, translations, OpenAPI documentation).
3. Its tests pass on **both SQLite and PostgreSQL**, using `autoxide-testkit`.
4. It has a README in English (a Vietnamese one is welcome), a `SECURITY.md` or a link to how to report problems, and
   `autoxide-module.toml`.
5. The repository carries the topic `autoxide-module`.

To appear on the curated list below, open a pull request that adds one row (alphabetical by name) to this file. A
maintainer checks the five points above, not the quality of the code, and may remove a row that stops meeting them.
The list is not an endorsement.

### The community list

| Module | Summary | Autoxide versions | Repository |
| ------ | ------- | ----------------- | ---------- |
| _none yet_ | The reference example is [`examples/external-module`](../examples/external-module/README.md) | `0.19` | in this repository |

## Official and community modules

Official modules are in `modules/` of this repository, maintained by the Autoxide maintainers, held to the full gate and
labelled by contract tier (stable or experimental). Community modules are maintained by their authors and carry
no promise from the Autoxide project. A community module that becomes widely used can be proposed to be adopted as an
official module with an [RFC](rfcs/README.md).
