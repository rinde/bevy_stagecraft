---
name: coordinate
description: Run the coordinator over rinde/rin-bevy-ui-tester — start ready-for-agent issues whose dependencies are done, get their pull requests reviewed, send them back and merge what agents may merge.
argument-hint: "[#issue ...] [max N] [dry-run]"
disable-model-invocation: true
context: fork
agent: coordinator
---

Run coordination passes over `rinde/rin-bevy-ui-tester` as your instructions describe, until
a pass finds nothing to do, then report.

Arguments: $ARGUMENTS
