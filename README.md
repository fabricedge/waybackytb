# ytb-wayback

[![CI](https://img.shields.io/github/actions/workflow/status/fabricedge/waybackytb/ci.yml?branch=main&logo=github&label=CI)](https://github.com/fabricedge/waybackytb/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/fabricedge/waybackytb?logo=github&label=Release)](https://github.com/fabricedge/waybackytb/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/fabricedge/waybackytb/total?label=Downloads)](https://github.com/fabricedge/waybackytb/releases)
[![License: MIT](https://img.shields.io/github/license/fabricedge/waybackytb)](LICENSE)

Download YouTube videos preserved by the **Wayback Machine**
(web.archive.org), as a small, fast, dependency-light Rust CLI.

When a YouTube video dies (channel deleted, copyright takedown, age block,
geo-restriction), the Internet Archive often still holds the original media —
it is just not obvious to find or fetch. `ytb-wayback` locates that archived
stream and pulls it down, with resumable downloads and a JSON metadata
sidecar.

## Download binaries

Pre-built binaries are attached to each
[GitHub Release](https://github.com/fabricedge/waybackytb/releases/latest),
including a `SHA256SUMS.txt` for verification.

| Platform | Artifact |
| --- | --- |
| Linux x86\_64 (glibc ≥ 2.31) | `ytb-wayback-v0.1.0-x86_64-unknown-linux-gnu.tar.gz` |
| Linux x86\_64 (static musl) | `ytb-wayback-v0.1.0-x86_64-unknown-linux-musl.tar.gz` |
| Windows x86\_64 | `ytb-wayback-v0.1.0-x86_64-pc-windows-gnu.zip` |

### Install

```bash
# Linux (extract to ~/.local/bin or anywhere in PATH)
curl -fsSL -o /tmp/ytb-wayback.tar.gz \
  https://github.com/fabricedge/waybackytb/releases/download/v0.1.0/ytb-wayback-v0.1.0-x86_64-unknown-linux-musl.tar.gz
tar -xzf /tmp/ytb-wayback.tar.gz -C ~/.local/bin
```

```powershell
# Windows (extract ytb-wayback.exe and add it to PATH)
curl.exe -fsSL -o ytb-wayback.zip `
  https://github.com/fabricedge/waybackytb/releases/download/v0.1.0/ytb-wayback-v0.1.0-x86_64-pc-windows-gnu.zip
tar -xf ytb-wayback.zip
```

### Build from source

Requires Rust **1.74+** (edition 2021).

```bash
cargo install --path .          # installs ytb-wayback
# or
cargo build --release           # binary at ./target/release/ytb-wayback
```

## Usage

```text
ytb-wayback <INPUT> [options]

  INPUT   a bare video ID, a youtube.com/youtu.be URL, or an archived
          web.archive.org URL
```

```bash
# Download from the closest archived capture
ytb-wayback C1hjsVLcGFc

# Same, via a YouTube link
ytb-wayback 'https://www.youtube.com/watch?v=C1hjsVLcGFc'

# Pin the snapshot: the most useful form, resolves everything for you
ytb-wayback 'https://web.archive.org/web/20241125121251/https://www.youtube.com/watch?v=C1hjsVLcGFc'

# Pin a snapshot by timestamp
ytb-wayback --date 20111027231107 C1hjsVLcGFc

# See what the archive has, without downloading
ytb-wayback --list-snapshots C1hjsVLcGFc

# Resolve and show metadata / target stream only
ytb-wayback C1hjsVLcGFc --dry-run

# Output somewhere else
ytb-wayback -o /mnt/vault C1hjsVLcGFc
```

### Options

| Option | Description |
| --- | --- |
| `-o, --output <DIR>` | Output directory (default `downloads`; videos go to `downloads/<id>/`). |
| `--date <TS>` | Force a specific Wayback snapshot (`YYYYMMDDhhmmss`). |
| `--list-snapshots` | Query the CDX index and print available captures, then exit. |
| `-d, --dry-run` | Resolve the stream and metadata; download nothing. |
| `-r, --retries <N>` | Retries for transient archive errors (default 3). |
| `--backoff-ms <MS>` | Initial retry backoff (default 2000, doubling). |
| `-v, --verbose` | Log HTTP progress. Implied by `--dry-run`. |

## How it works

The archive stores the raw `videoplayback` stream it captured and keeps an
internal index of them under the fake host
`wayback-fakeurl.archive.org/yt/<video_id>` (the same mechanism behind the
archive's "video download" button on archived YouTube pages).

`ytb-wayback` asks the archive for that URL with the `oe_` (original
encoding / raw media) replay flag:

```
https://web.archive.org/web/{YYYYMMDDhhmmss}oe_/http://wayback-fakeurl.archive.org/yt/{id}
```

which redirects to the stored stream. This **bypasses YouTube's signature
ciphers**: modern archived pages only contain `s=` signatures that can no
longer be decoded, but the archive's own copy of the media is already signed
and served as-is. The stream supports HTTP Range requests, so interrupted
downloads resume automatically via a `.part` file.

Metadata (title, uploader, upload date, ...) is scraped best-effort from the
archived watch page (`ytInitialPlayerResponse.videoDetails` and the
`videoDescriptionHeaderRenderer` fallback). Most captured formats are
**progressive** (video + audio muxed); some DASH captures are video-only and
are reported as such.

## Output

```text
downloads/C1hjsVLcGFc/
├── Skrillex - With, You Friends (C1hjsVLcGFc).webm      # the media
└── Skrillex - With, You Friends (C1hjsVLcGFc).info.json # metadata sidecar
```

The extension comes from the captured stream's `itag` (format table in
`src/formats.rs`) or, failing that, its `Content-Type`.

`info.json` contains: `video_id`, `snapshot_date`, `title`, `uploader`,
`channel_id`, `upload_date`, `duration_seconds`, `thumbnail`,
`archive_stream_url` (the exact `oe_` stream fetched), `itag`, `ext`,
`has_audio`, `size_bytes`, `bytes_written`, `content_type`, `downloaded_at`.

> The `downloads/` directory and `*.part` files are git-ignored.

## Limitations

- **Media must be captured.** The archive only holds streams it recorded; a
  page capture without a media capture means nothing to download (the tool
  says so explicitly).
- **Metadata is best-effort.** Archived pages vary wildly across the years
  (consent walls, blank `<title>`, JS-rendered fields). When nothing usable is
  found, the video ID is used as the title.
- **Not all formats are archived.** Early captures often only hold one format
  (typically 360p–720p webm or mp4 at the time of capture).
  `--list-snapshots` shows what exists.
- A watch page may be captured while the media was **not** (e.g. HTTP 301/302
  redirects, robot-blocked captures).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Changes are tracked in
[CHANGELOG.md](CHANGELOG.md).

## License

MIT © fabricedge — see [LICENSE](LICENSE).