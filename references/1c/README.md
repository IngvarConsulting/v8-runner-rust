# 1C Reference Corpus

This directory contains raw or lightly structured reference material about 1C tools and command
surfaces. It is useful when implementing low-level integrations, but it is not the source of
truth for `v8-runner` behavior, contracts, or supported scenarios.

## Included Material

- `designer-startup/`: startup parameters and mode selection notes.
- `designer-agent/`: agent-mode notes.
- `designer-batch/`: batch command notes.
- `designer-spec.md`: raw designer command reference excerpt.
- `ibcmd-commands-full.md`: raw `ibcmd` command reference.
- `confirmed-runtime-measurements.md`: facts obtained by running the real tools — exit codes,
  exact messages, readiness windows. They cannot be re-derived without a 1C installation.

## Search Hygiene

This directory is excluded from default `rg` searches by `.rgignore` so routine repository
searches stay focused on the project itself.
