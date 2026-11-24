# Workflow

## Issue and pull request workflow

Issues labelled `ready for agent` are available to be picked.

A change here reaches `rinde/rin-bevy-ui` only through a `rev` bump there, and
issues there may wait on issues here. Once the issue here is closed, update
the dependent issue there so it can proceed.

There is a GitHub Actions billing problem, this is a known problem, no
action is required. Running `just ci` is a valid substitute and is enough
for merging a PR.

## Before you call it done

1. `just fmt`.
2. `just ci`.
3. Update `README.md` if usage changed, and the docs your change touches, as
   [writing-docs.md](writing-docs.md) says.
4. Create or update the PR, and check whether it needs a rebase.

## CI billing failures

A CI job that fails because of billing or quota (for example the account
cannot start runners) is not a failure of the change. Say so in the PR rather
than changing code to chase it.

