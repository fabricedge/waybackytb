## Summary

<!--
what changed and why; link the issue when one exists.
Choose a conventional prefix for the title: `feat:` `fix:` `docs:` `chore:` `test:` `ci:` `refactor:`
-->

## How it works

<!-- any non-obvious implementation detail, or delete this section -->

## Test plan

<!-- exact commands run and a short summary of their output. The Wayback
  Machine is flaky under load (empty CDX, HTTP 429/503) — note any manual /
  live verification too: --dry-run, --list-snapshots, a real download with
  --max-size. -->

```
```

## Trade-offs

<!-- anything worth deciding on during review, or delete this section -->

## Checklist

- [ ] Branch is up to date with `main`; PR targets `main`.
- [ ] `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
      `cargo test --locked`, `cargo build --release --locked` pass locally.
- [ ] CHANGELOG updated under `[Unreleased]` (Keep a Changelog).
- [ ] README / `--help` updated in this PR if CLI, output, or behavior changed.
- [ ] Tests added/updated for parsing logic (`video_id`, `cdx`, `html`,
      `formats`, `size`).
- [ ] No new dependencies (or justified in the PR body).
- [ ] No secrets, tokens, or credentials committed.