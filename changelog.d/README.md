# Changelog fragments

Do not edit `CHANGELOG.md` in a pull request. Add a file here instead, so that pull requests
written in parallel never conflict on the same lines (decision D-20261119-changelog-fragments).

- **Name:** `<name>.<section>.md`. `<name>` is lowercase letters, digits and `-` (the branch or
  the task, e.g. `step45-checkout`); `<section>` is one of `added`, `changed`, `deprecated`,
  `removed`, `fixed`, `security` ([Keep a Changelog](https://keepachangelog.com/en/1.1.0/)).
- **Content:** one or more English bullet points exactly as they should read in the changelog.
  A continuation line is indented by two spaces. No headings: the file name picks the section.

```markdown
- Module `orders`: checkout reserves stock, promotions and the payment in one saga; an expired
  order releases all three.
```

`cargo xtask check` (part of `./gate.sh`) rejects a bad name or content. At release time
`cargo xtask changelog` folds every fragment into `## [Unreleased]` of `CHANGELOG.md`, at the end of
its section, and deletes the fragments (see `docs/RELEASING.md`).
