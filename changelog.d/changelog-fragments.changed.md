- Contributing: pull requests no longer edit `CHANGELOG.md`; they add a fragment to `changelog.d/`
  (`<name>.<section>.md`), folded into the changelog at release time by `cargo xtask changelog`.
