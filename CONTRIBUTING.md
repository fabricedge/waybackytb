# Contributing

Thanks for wanting to help! `ytb-wayback` is a small, focused tool — keeping
dependencies and surface area small is a feature.

## Prerequisites

- Rust 1.74+ (see `Cargo.toml` `rust-version`)
- Run checks locally before pushing:

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

## Development

```bash
cargo build --release
cargo run --release -- <video id or URL> --dry-run
cargo run --release -- <video id> --list-snapshots
```

`--dry-run` and `--list-snapshots` are your friends: they exercise the full
network flow without downloading media.

## Code layout

| Path | Purpose |
| --- | --- |
| `src/video_id.rs` | ID/URL input parsing and normalization |
| `src/cdx.rs` | CDX index query and snapshot parsing |
| `src/wayback.rs` | archive HTTP layer, fakeurl/media resolution, retries |
| `src/download.rs` | resumable range download |
| `src/html.rs` | archived watch-page metadata extraction |
| `src/formats.rs` | itag → container/codec/extension table |

## Guidelines

- No new dependencies without strong justification.
- Keep errors actionable: prefer `anyhow!` with context that explains what went
  wrong and what the user can do about it.
- Preserve resumability semantics; don't break `.part`/`Range` behavior.
- Add or update tests for any parsing logic (`video_id`, `cdx`, `html`,
  `formats`).

## Pull requests

All changes land on `main` through a pull request. `main` is protected:
direct pushes are blocked, the CI status check must pass, and the PR must be
up to date with `main` before merging.

### Branch naming

- `feature/<short-kebab-desc>` — new functionality.
- `fix/<short-kebab-desc>` — bug fixes.
- `docs/<short-kebab-desc>` — documentation only.
- `chore/<short-kebab-desc>` — maintenance, tooling, CI, policies.

Create the branch from the latest `main` and keep changes focused on a single
logical concern. Small, reviewable PRs land faster.

### Before opening

Run the same checks the CI runs, locally:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
```

### PR title

Start with a conventional-change type so the title doubles as a summary:

```text
feat:   fix:   docs:   chore:   test:   ci:   refactor:
```

Examples: `feat: add --max-size`, `fix: detect .flv content-type`,
`ci: require CHANGELOG updates`. The CI rejects titles without a valid prefix.

### Description

The PR template guides the body. Cover:

- **What** changed and **why** (link the issue when one exists).
- **How** it works, if non-obvious.
- **Test plan**: the exact commands you ran and a summary of their output.
  Because the Wayback Machine is flaky under load (empty CDX responses,
  HTTP 429/503), describe any manual/live verification too, e.g. `--dry-run`,
  `--list-snapshots`, or a real download with `--max-size`.
- **Trade-offs** worth reviewing.

### Documentation sync (required)

Any change to a CLI flag, output message, default behavior, or `src/` surface
must update, in the **same PR**:

1. `--help` (the `Cli` struct in `src/main.rs`),
2. `README.md` (usage examples, options table, "How it works"),
3. `CHANGELOG.md` under `[Unreleased]`.

A PR that changes behavior without touching these will fail CI.

### Changelog

Add an entry under `[Unreleased]` following [Keep a Changelog] sections
(`Added` / `Fixed` / `Changed` / `Removed`). Reference the user-visible
behavior, not implementation details.

[Keep a Changelog]: https://keepachangelog.com/en/1.1.0/

### Checklist

Before requesting a review (or merging, on a solo PR):

- [ ] Branch is up to date with `main`; PR targets `main`.
- [ ] `cargo fmt --check`, `clippy --all-targets -- -D warnings`,
      `cargo test --locked`, `cargo build --release --locked` pass.
- [ ] CHANGELOG entry under `[Unreleased]`.
- [ ] README / `--help` updated if CLI or behavior changed.
- [ ] Tests added/updated for parsing logic (`video_id`, `cdx`, `html`,
      `formats`, `size`).
- [ ] No new dependencies, or the necessity is justified in the PR body.
- [ ] No secrets, tokens, or credentials committed.
- [ ] Manual verification steps and results recorded in the test plan.

### Merging

Prefer **squash merge** for a clean `main` history. After merge, delete the
branch. The CI runs on every push to `main`, so a merged PR is verified
again on the default branch.

## Releases

Versions follow SemVer. To cut a release:

1. Update `version` in `Cargo.toml` and add a `CHANGELOG.md` entry.
2. Push a `vX.Y.Z` tag; the release workflow builds binaries (Linux gnu +
   musl, Windows) and publishes them to a GitHub Release.

## Questions / issues

Open an issue at
[https://github.com/fabricedge/waybackytb/issues](https://github.com/fabricedge/waybackytb/issues).