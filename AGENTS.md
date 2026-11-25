# Development notes

This file is for whoever changes the code. If it and the code disagree, the
code wins, and both documents need fixing in the same change.

## What this is

A test driver for Bevy UIs and the apps built on them. A scenario (setup,
view, scripted pointer and keyboard steps, checks) runs three ways:

- headless in `cargo test`;
- recorded offscreen for review (feature `record`);
- played in a gallery (feature `gallery`).

The crate is `crates/bevy_stagecraft`.

## The one architectural rule

The crate takes Bevy types only. Applications extend it through its extension
points, each the app's one reflected enum found by Bevy's automatic
registration (no registration call, nothing looked up by name): setups
(`#[reflect(AppSetup)]`, the world builders), views (`#[reflect(AppView)]`),
targets (`#[reflect(AppTarget)]`) and checks (`#[reflect(AppCheck)]`); and
probes over the world.

## Rules for every change

- Forbidden in production code: `unwrap`, `expect`, `panic!`, `todo!`,
  `unimplemented!`, `dbg!`, printing, indexing and slicing, `unsafe`, and the
  Bevy methods in `clippy.toml`. Details: `agent-docs/code-practices.md`.
- Before you call it done: `just fmt`, then `just ci`, then update the docs
  your change touches (`agent-docs/writing-docs.md`). A docs budget failure
  means split the file, not compress it.
- American English everywhere: code, comments and docs (behavior, color,
  initialize).
- `publish = false` stays. There is no license file and no open-source framing.
- A CI job failing on billing or quota is not a failure of the change; say so
  in the PR.

## Comments

Default to none. Rationale, history and how-it-broke stories go in the PR
description, the commit message or a test, not the source.

- Add one only when (a) the code can't say it itself, (b) a maintainer would
  make the wrong call without some history, or (c) the code is deliberately
  weird for a reason.
- One line, two at most. Imperative, stating the constraint, optionally naming
  the tempting edit that breaks it.
- Before writing one, check whether a nearby type or name already says it. If
  so, it's redundant, even when it feels like a "why".
- Never: doc that restates types, narration of what a branch does,
  design-decision explainers, conditionals or code structure, ticket IDs or
  provenance (`git blame` has it), doc on small self-describing helpers.
- Match the comment density of the surrounding file.
- On the fence? Bias to no comment or doc.
- Apply this while writing, not as a cleanup pass. Before opening a PR, check
  every added comment in the full diff, including ones from earlier sessions.
- A plain `//` or `///` line that passes these rules is fine.

## Index

Read the file when its trigger applies. Each file is under `agent-docs/`.

| File | Read it when |
| ---- | ------------ |
| [workflow.md](agent-docs/workflow.md) | you pick an issue, open a PR, or finish a change |
| [running.md](agent-docs/running.md) | you build, test or lint, or a cloud session lacks a tool |
| [code-practices.md](agent-docs/code-practices.md) | clippy complains, or you write production code |
| [bevy.md](agent-docs/bevy.md) | you add a system, touch Bevy features or the Bevy version |
| [determinism.md](agent-docs/determinism.md) | a scenario differs between runs or runners |
| [testing.md](agent-docs/testing.md) | you write a test, or a test needs a GPU |
| [dependencies.md](agent-docs/dependencies.md) | `just deny` fails or you add a dependency |
| [architecture.md](agent-docs/architecture.md) | you need how the modules fit together: waiting, errors, captured values, screenshots |
| [modules.md](agent-docs/modules.md) | you look for the module that does something |
| [architecture-tooling.md](agent-docs/architecture-tooling.md) | you change how the player, recorder, trace, session or gallery work together |
| [writing-docs.md](agent-docs/writing-docs.md) | you write or move documentation |
| [usage guides](README.md#guides) | you change behavior a user can see: update the guide for it |
