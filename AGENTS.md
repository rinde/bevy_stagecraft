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
  your change touches (`agent-docs/writing-docs.md`).
- British spelling (behaviour, colour, initialise). Every item is documented.
- `publish = false` stays. There is no licence file and no open-source framing.
- A CI job failing on billing or quota is not a failure of the change; say so
  in the PR.

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
| [modules.md](agent-docs/modules.md) | you look for the module that does something |
| [module-details.md](agent-docs/module-details.md) | you change a module and need its detailed behaviour |
| [writing-docs.md](agent-docs/writing-docs.md) | you write or move documentation |
