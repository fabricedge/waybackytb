# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-09-18

### Added

- Download YouTube media preserved by the Wayback Machine via the
  `wayback-fakeurl` media index (`oe_` replay flag), bypassing YouTube's
  signature ciphers.
- Input modes: bare video ID, `youtube.com`/`youtu.be` URL, and pinned
  `web.archive.org` snapshot URL.
- Snapshot resolution: CDX `--list-snapshots`, `--date` pinning, and archive
  default (`2`) when no date is given.
- Resumable downloads with HTTP Range via `*.part` files; existing files are
  detected and skipped.
- Best-effort metadata (title, uploader, channel ID, upload date, duration,
  thumbnail) including `videoDescriptionHeaderRenderer` fallback, written to a
  `.info.json` sidecar.
- itag format table with content-type fallback for extension/audio detection;
  video-only (no audio) captures are reported and flagged in metadata.
- Retry with exponential backoff for transient archive failures (5xx/429);
  terminal 4xx fail fast.
- `--dry-run`, `--verbose`, and configurable retries/backoff.
- CI (fmt, clippy `-D warnings`, tests) and a release workflow publishing
  Linux (gnu + musl) and Windows binaries with `SHA256SUMS`.

[0.1.0]: https://github.com/fabricedge/waybackytb/releases/tag/v0.1.0