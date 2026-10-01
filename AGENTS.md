# Agent notes

- `just ci` runs everything CI runs. Run it before pushing.

## Dependencies and advisories (`just deny`)

`just deny` runs `cargo deny check` against `deny.toml`: advisories, bans,
licences and sources. It is part of `just ci`.

When `just deny` reports a new advisory:

1. Fix it first: update or replace the dependency (`cargo update -p <crate>`).
2. If it cannot be fixed, add it to `advisories.ignore` in `deny.toml` with the
   advisory ID, the reason, and a date to revisit. No ignore without all three.

A new duplicate crate version is a warning: avoid it, or add it to
`bans.skip` with a reason. A new licence must be checked, then added to the
allow-list explicitly. Only crates.io is allowed as a source.
