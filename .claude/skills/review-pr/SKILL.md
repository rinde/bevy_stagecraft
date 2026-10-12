---
name: review-pr
description: Review a pull request on rinde/rin-bevy-ui-tester against its issue and the repository's rules and make sure CI passed, running it locally where GitHub did not, then approve it or request changes with detailed inline comments. The reviewer agents the coordinator dispatches run this; run it by hand as /review-pr <number> [sonnet|opus].
argument-hint: <pr-number> [sonnet|opus]
allowed-tools: Read, Grep, Glob, Write, Bash(gh api:*), Bash(git fetch:*), Bash(git diff:*), Bash(git log:*), Bash(git show:*), Bash(git switch:*), Bash(git status:*), Bash(git rev-parse:*), Bash(just ci:*), Bash(just test-gpu:*)
---

# Review a pull request

The arguments, or the prompt that started you, name the pull request `<pr>`
and the reviewer variant, `sonnet` or `opus` (`sonnet` if none is named),
which goes into the review's marker below. Review pull request `<pr>` on
`rinde/rin-bevy-ui-tester`, make sure CI passed on it, and post **one** GitHub review that
either approves it or requests changes, with an inline comment on every point
you raise. You only review and run checks. You never push, merge or edit the
branch.

Talk to GitHub through `gh api` and the REST endpoints below. GraphQL is
blocked in Claude Code cloud sessions, so `gh pr view`, `gh issue view` and
friends may fail where `gh api repos/...` works.

## 1. Gather

- The pull request: `gh api repos/rinde/rin-bevy-ui-tester/pulls/<pr>`. Note `head.sha` (the
  commit you review), `base.ref`, `draft`, `body` and `user.login`.
- The diff: `gh api repos/rinde/rin-bevy-ui-tester/pulls/<pr> -H "Accept: application/vnd.github.diff"`,
  and the file list with `gh api --paginate repos/rinde/rin-bevy-ui-tester/pulls/<pr>/files`.
- The full files, not just the hunks, from a checkout of the head. As a
  reviewer agent you are in a worktree of your own under `.claude/worktrees/`:
  fetch the head and check it out there,
  `git fetch origin pull/<pr>/head:refs/remotes/origin/pr/<pr>` then
  `git switch --detach origin/pr/<pr>`, and confirm with `git status` that the
  worktree is clean and `HEAD` is `head.sha`. Never switch the branch of the
  main checkout or of another agent's worktree.
- The issue it closes: the `Closes #N` / `Fixes #N` in the body, else the
  `TESTER-<N>/` prefix of the branch. Read it with
  `gh api repos/rinde/rin-bevy-ui-tester/issues/<N>`, its comments with
  `.../issues/<N>/comments`, and note its labels. With no issue, review the
  pull request on its own description.
- Earlier reviews and threads: `gh api repos/rinde/rin-bevy-ui-tester/pulls/<pr>/reviews` and
  `gh api --paginate repos/rinde/rin-bevy-ui-tester/pulls/<pr>/comments`. On a re-review,
  check whether each earlier point was dealt with and do not raise again what
  was fixed or answered convincingly.
- CI on the head: `gh api repos/rinde/rin-bevy-ui-tester/commits/<head.sha>/check-runs`.
  See the next step for what to make of it.
- The rules: `AGENTS.md`, all of it, and a sibling of every module the diff
  touches, so you can tell whether the change matches what is already there.

## 2. Make sure CI passed

An approval means CI passed on exactly `head.sha`, on GitHub or on your
machine. GitHub's CI is the jobs **lint**, **test**, **features**,
**unused-deps** and **deny** in the check runs of `head.sha`, which together are
`just ci`, and **gpu**, which is `just test-gpu` on lavapipe.

- **All six succeeded** → CI passed on GitHub: `ci=github` in the marker.
- **One failed** with a step's output → a blocking finding: name the job and
  quote the failing lines from its log
  (`gh api repos/rinde/rin-bevy-ui-tester/actions/jobs/<id>/logs`), on the line of the
  change that causes it where you can tell.
- **One did not run** — missing, cancelled, skipped, or failed for billing
  reasons without a step's output (see **CI billing failures** in
  `AGENTS.md`) — or is still queued or running → run CI yourself, in the
  checkout of the head: `just ci`, then `just test-gpu` (lavapipe, which the
  session-start hook installs). They build Bevy, so give each command a long
  timeout (up to 45 minutes). In a worktree, reuse the main checkout's build
  rather than compiling Bevy from cold, and keep debug info small:
  `CARGO_TARGET_DIR="$(git rev-parse --path-format=absolute --git-common-dir)/../target" CARGO_PROFILE_DEV_DEBUG=line-tables-only just ci`
  (and the same for `just test-gpu`).
  - Both pass → CI passed locally: `ci=local` in the marker, and say in the
    body that GitHub's CI did not run and `just ci` and `just test-gpu`
    passed on `head.sha`.
  - A format, clippy, doc, test, feature, unused-dependency, `cargo deny` or
    GPU check fails → a blocking finding, as above, with the failing output.
  - It cannot run at all — a missing toolchain, tool or system library,
    nothing to do with the change →
    you cannot approve. Post no review; report what stopped it, so a human
    can fix the machine. You may still post a `request_changes` review if the
    code has blocking findings of its own.

The macOS and Windows compile checks have no local equivalent; their failure is blocking like any other, and their not running is
not.

## 3. Judge

Work through all of it, and say where in the code each finding lives:

1. **Does it do what the issue asks**, all of it and nothing unrelated? A
   bare-title issue's title is the specification.
2. **Correctness.** Trace the changed behaviour from a real caller: a
   scenario played headless, recorded and in the gallery, where the change
   reaches all three. Edge cases, ordering, off-by-one, state left behind; a
   step that reports success without waiting for what it claims; a check that
   passes vacuously; a script that loads differently from how it is written
   (`to_ron` / `from_ron` round trip, `format_version`).
3. **The rules that always apply**: no dependency on a game crate and no game
   state, Bevy types only, apps extending the tester through the reflected
   extension points (`AppSetup`, `AppView`, `AppTarget`, `AppCheck`), nothing
   looked up by name; the Bevy version and the curated Bevy feature list
   untouched, with windowing and rendering behind `gallery` and `record`;
   none of the forbidden constructs in production code (`unwrap`, `expect`,
   `panic!`, `todo!`, `unimplemented!`, `dbg!`, printing, indexing and
   slicing, `unsafe`, `std::` paths that exist in `core`/`alloc`, the Bevy
   accessors `clippy.toml` bans); `allow` only in tests and always with a
   reason; every item documented, private ones included; `#[derive(Debug)]`
   on every type; named constants, not magic numbers; British spelling;
   determinism (fixed 1/60 s steps, no wall clock, no `HashMap`/`HashSet`
   iteration where the order reaches behaviour); a new advisory ignore in
   `deny.toml` has an ID, a reason and a revisit date; `publish = false`
   stays and no open-source framing.
4. **Tests.** New behaviour is tested, and the pull request says why where it
   cannot be; tests are named as sentences and drive a plain Bevy UI, never a
   game; GPU tests have `gpu` in their name, need feature `record` and are
   `#[ignore]`d, and `just test` still passes without a GPU; tests stay small
   and fast.
5. **Documentation.** `README.md` updated if usage changed; `AGENTS.md`
   updated for a new or changed module, convention or command, including the
   module table.
6. **Readability.** Code reads like the code around it; comments say why, not
   what.

Sort every finding into one of two kinds:

- **Blocking**: wrong behaviour, a broken rule above, a missing test for new
  behaviour, missing docs, red CI caused by
  the change or failing locally.
- **Nit**: style, naming, a clearer way to write it. Optional by definition.

## 4. Decide

- Any blocking finding → the verdict is `request_changes`.
- None, and CI passed on `head.sha` on GitHub or locally → the verdict is
  `approve`, nits and all.

If the issue carries the **`require-human-review`** label, a human must approve
this work. Still review it fully and still record your verdict, but never
submit an `APPROVE` review: submit `COMMENT` for an approve verdict, and say at
the top of the body that a human must approve and merge it.

## 5. Post one review

Write the review as JSON to a file outside the repository, such as
`/tmp/review-<pr>.json`:

```json
{
  "commit_id": "<head.sha>",
  "event": "APPROVE",
  "body": "<!-- agent-review verdict=approve reviewer=sonnet sha=<head.sha> ci=github -->\n## Approved\n...",
  "comments": [
    { "path": "crates/rin_bevy_ui_tester/src/player.rs", "line": 42, "side": "RIGHT",
      "body": "**Blocking:** ..." }
  ]
}
```

then post it with
`gh api repos/rinde/rin-bevy-ui-tester/pulls/<pr>/reviews --input /tmp/review-<pr>.json`.

- `event` is `APPROVE` or `REQUEST_CHANGES` to match the verdict, or `COMMENT`
  as above.
- **The body's first line is the marker** the coordinator reads, exactly:
  `<!-- agent-review verdict=<approve|request_changes> reviewer=<sonnet|opus> sha=<head.sha> ci=<github|local|none> -->`,
  where `ci` says where CI passed on `head.sha`: `github`, `local` when you
  ran `just ci` yourself, or `none` when it did not pass anywhere (only with
  `request_changes`).
- Then a heading with the verdict, a short summary of what the change does and
  how well, the blocking findings as a numbered list, the nits, and a line on
  what you checked: CI on `head.sha`, on GitHub or by `just ci` locally, and
  its result; tests; docs.
- One inline comment per finding, on the line it concerns, starting
  `**Blocking:**` or `**Nit:**`. Explain the problem, why it matters here, and
  what to do instead, with a suggested fix where it is short (a ```suggestion
  block replaces the commented line). A comment's `line` must be a line of the
  diff on the `RIGHT` side; a finding that has no such line goes in the body.
- If GitHub refuses the review because a comment's line is not in the diff
  (HTTP 422), move that comment into the body and post again. If it refuses
  `APPROVE` or `REQUEST_CHANGES` because you are the pull request's author,
  post the same review as `COMMENT`; the marker still carries the verdict.

Finish by saying, in a line or two, the verdict, the number of blocking
findings and nits, and the review's URL.
