# Writing docs

Conventions that keep the docs short. `AGENTS.md` is loaded into every agent
session, so what is read on every task must stay small.

## Where a fact lives

- What a type or module does, its invariants and edge cases: rustdoc (`//!` and
  `///`). `cargo doc -D warnings` checks its links, and it sits next to the code
  it describes.
- How to use the crate: `README.md` and `agent-docs/usage/`.
- How to work on the repository (process, conventions, cross-module design):
  `agent-docs/`.
- `AGENTS.md`: only the index and the rules every change needs.

A fact is written in one place and linked from the others, never copied.

## Budgets

| File | Lines | Size |
| ---- | ----- | ---- |
| `AGENTS.md` | 100 | 6 KB |
| `README.md` | 200 | 10 KB |
| any file in `agent-docs/` | 250 | 16 KB |

No line is over 200 characters outside fenced code blocks. A later ticket
enforces this. A file that outgrows its budget is split by topic, with an index
entry for each part, not trimmed into shorthand.

`module-details.md` is over budget for now: it is a temporary holding place
for the modules not yet dissolved into rustdoc.

## Updating

A change updates the topic file it touches and, only if a file was added,
removed or its trigger changed, the index line in `AGENTS.md`. Module status
columns ("done") and changelog-style "replaces the old X" notes do not belong in
the docs; git history has them.
