---
name: implement-issue
description: Implement one GitHub issue of rinde/rin-bevy-ui-tester end to end — branch, code, checks, pull request — or, given its pull request, address the review, CI failures and merge conflicts on it. The implementer agents the coordinator dispatches run this; run it by hand as /implement-issue <issue> [pr].
argument-hint: <issue-number> [pr-number]
---

# Implement an issue

The arguments, or the prompt that started you, name the issue `<issue>` and
perhaps its pull request `<pr>`. You own the issue on `rinde/rin-bevy-ui-tester` until its
pull request is ready to merge. You never merge, approve or review your own pull request, and
never push to `main`.

Talk to GitHub through `gh api` and the REST endpoints below. GraphQL is
blocked in Claude Code cloud sessions, so `gh pr create`, `gh issue view` and
friends may fail where `gh api repos/...` works.

## 1. Read and decide whether you can start

- The issue, its labels and comments: `gh api repos/rinde/rin-bevy-ui-tester/issues/<issue>`
  and `.../issues/<issue>/comments`. A bare-title issue's title is the
  specification.
- An existing pull request: if none was given, look for an open one whose head
  branch starts `issue-<issue>-` or whose body says `Closes #<issue>`
  (`gh api "repos/rinde/rin-bevy-ui-tester/pulls?state=open&per_page=100"`). If one exists,
  go to **Follow up** below instead.
- **Dependencies.** Every issue in
  `gh api repos/rinde/rin-bevy-ui-tester/issues/<issue>/dependencies/blocked_by`, every open
  sub-issue in `.../issues/<issue>/sub_issues`, and every issue the body names
  as `Depends on #N`, `Blocked by #N` or `After #N` must be closed as completed.
  If one is not, stop and report which.
- **Ambiguity.** If the issue is genuinely ambiguous — two reasonable readings
  that lead to different code — do not guess. Comment on the issue with the
  specific question and the readings you see, remove `ready-for-agent`, add
  `Needs refinement`, and stop.

## 2. Start

1. Add the `In progress` label:
   `gh api repos/rinde/rin-bevy-ui-tester/issues/<issue>/labels -f "labels[]=In progress"`.
2. Branch from a fresh `main`, named `issue-<issue>-<short-slug>`:
   `git fetch origin main && git switch -c issue-<issue>-<slug> origin/main`.
   You are usually in a worktree of your own under `.claude/worktrees/`;
   another implementer may be working in the main checkout or a sibling
   worktree, so touch nothing outside yours.
3. Read `AGENTS.md` in full, then the module you are changing (its row in
   "Where to find what" says what it holds) and a sibling module that does
   something similar.

A worktree has its own `target/`, so its first build is a cold Bevy compile.
To reuse the main checkout's build, prefix cargo and `just` commands with
`CARGO_TARGET_DIR="$(git rev-parse --path-format=absolute --git-common-dir)/../target"`.
On a tight disk, also set `CARGO_PROFILE_DEV_DEBUG=line-tables-only`, as CI
does.

## 3. Implement

Follow the rules in `AGENTS.md` without exception: no dependency on a game
crate and no game state (apps extend the tester through its reflected
extension points); the Bevy version and the hand-curated Bevy feature list
stay as they are; none of the forbidden constructs in production code, and the
fallible idiom instead; every item documented, private ones included; British
spelling; determinism (fixed 1/60 s steps, no wall clock, no unordered
iteration that reaches behaviour). Name tests as sentences and drive a plain
Bevy UI, never a game; a test that needs a GPU has `gpu` in its name, needs
feature `record` and is `#[ignore]`d. Keep the change to what the issue asks;
note anything else worth doing in the pull request rather than doing it.

## 4. Before you call it done

1. `just fmt`
2. `just ci` (lint, test, features, unused-deps, deny)
3. `just test-gpu` if the change touches the recorder, capture, index, overlay
   or anything a GPU test covers (lavapipe: `WGPU_BACKEND=vulkan`)
4. New behaviour has a test, headless where possible
5. `README.md` updated if usage changed, and `AGENTS.md` (its module table
   included) if a module, convention or command changed
6. Anything drawn with text was looked at once in the recorder's PNGs
   (`target/tmp/recorder-*`), if the change draws text

All green, then commit with a message that says what the change does, in the
imperative (`Wait for a target to stop moving before a click`), and push with
`git push -u origin <branch>`.

## 5. Open the pull request

```sh
gh api repos/rinde/rin-bevy-ui-tester/pulls -f base=main -f head=<branch> \
  -f title="<imperative summary>" -f body="$(cat /tmp/pr-body-<issue>.md)"
```

The body starts with `Closes #<issue>`, then says what changed and why, how it
was tested (the commands you ran and that they passed), and anything a
headless test could not exercise because it needs a window, a display or a
GPU. If the change will need a `rev` bump in `rinde/rin-bevy-ui` to be used
there, say so. The
coordinator has it reviewed once you report it.

## Follow up

Given a pull request, bring it to a mergeable state:

1. Check out its branch in your worktree:
   `git fetch origin <branch> && git switch <branch>` (or
   `git switch -c <branch> origin/<branch>`), and `git pull` to be current.
2. **Merge conflict** (`mergeable` is false): `git merge origin/main`, resolve,
   re-run the checks, push. Never rebase, amend or force-push a pushed branch.
3. **Red CI**: read the failing job's log
   (`gh api repos/rinde/rin-bevy-ui-tester/commits/<sha>/check-runs`, then the job's
   `details_url` or `gh api repos/rinde/rin-bevy-ui-tester/actions/jobs/<id>/logs`),
   reproduce it locally, fix the cause, push. A job that failed for billing
   reasons is not yours to fix. Never skip, disable or loosen a test.
4. **Review**: read every review (`.../pulls/<pr>/reviews`) and inline comment
   (`.../pulls/<pr>/comments`), from agents and humans. Fix every `Blocking`
   point; take the nits that are plainly right. For a point you do not take,
   reply on its thread with why:
   `gh api repos/rinde/rin-bevy-ui-tester/pulls/<pr>/comments/<id>/replies -f body="..."`.
   Push the fixes, then reply on each thread you fixed naming the commit.
5. Re-run **Before you call it done** before every push.

## Report

End with: the issue, the pull request's number and URL, what you did, what the
checks said, and anything left open or that a human needs to decide.
