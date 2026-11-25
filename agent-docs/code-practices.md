# Code practices

The lint regime lives in `[workspace.lints]` in `Cargo.toml` and `clippy.toml`;
every crate inherits it. Clippy `all`, `pedantic`, `nursery` and `cargo` are
denied.

**Forbidden in production code:** `unwrap`, `expect`, `panic!`, `todo!`,
`unimplemented!`, `dbg!`, `print!`/`println!`/`eprintln!`, indexing and
slicing, `unsafe`, `std::` paths that exist in `core`/`alloc`, and the Bevy
methods in `clippy.toml` (`Query::single`, `single_mut`, `World::resource`,
`resource_mut`).

The fallible idiom instead: return `Result` or `Option` and propagate with `?`;
use `.get(i)` rather than `[i]`; use `let Some(x) = ... else { return ... };`;
use `World::get_resource` and `Query::iter().next()` (or `get`) and handle the
`None`; report problems through errors or Bevy's `warn!`/`error!`
logging, not stdout.

- `allow` is permitted only in tests, always with `reason = "..."`.
- No item needs a doc; docs and comments follow
  [the comment rules](../AGENTS.md#comments).
- `#[must_use]` on accessors, `const fn` where it compiles, and
  `#[derive(Debug)]` on every type.
- Named constants instead of magic numbers.

