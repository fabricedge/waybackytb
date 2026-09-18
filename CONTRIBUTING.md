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

## Releases

Versions follow SemVer. To cut a release:

1. Update `version` in `Cargo.toml` and add a `CHANGELOG.md` entry.
2. Push a `vX.Y.Z` tag; the release workflow builds binaries (Linux gnu +
   musl, Windows) and publishes them to a GitHub Release.

## Questions / issues

Open an issue at
[https://github.com/fabricedge/waybackytb/issues](https://github.com/fabricedge/waybackytb/issues).