# ytb-wayback

Download YouTube videos that the **Wayback Machine** (web.archive.org) has
preserved.

When a YouTube video's page is archived but the video dies (channel deleted,
copyright takedown, age-blocked, etc.), the video itself is frequently *also*
captured by the Internet Archive — it just isn't obvious to find or fetch.
`ytb-wayback` is a small Rust CLI that locates that archived media and pulls
it down, with resumable downloads and a JSON metadata sidecar.

## Why this works

The archive stores the raw `videoplayback` stream it captured, and keeps an
internal index of them under the fake host
`wayback-fakeurl.archive.org/yt/<video_id>` (this is how the archive's
"video download" button on archived YouTube pages works).

`ytb-wayback` asks the archive for that URL with the `oe_` (original encoding /
raw media) flag:

```
https://web.archive.org/web/{YYYYMMDDhhmmss}oe_/http://wayback-fakeurl.archive.org/yt/{id}
```

which redirects to the stored stream. Crucially this **bypasses YouTube's
signature ciphers**: modern archived pages only contain `s=` signatures that
can no longer be decoded, but the archive's own copy of the media is already
signed and served as-is. The stream supports HTTP Range requests, so
interrupted downloads resume via a `.part` file.

Metadata (title, uploader, upload date, ...) is best-effort, scraped from the
archived watch page (`ytInitialPlayerResponse.videoDetails` and the
`videoDescriptionHeaderRenderer`).

## Install / build

```bash
cargo build --release
# binary at ./target/release/ytb-wayback
```

## Usage

```text
ytb-wayback <INPUT> [options]

  INPUT   a bare video ID, a youtube.com/youtu.be URL, or an archived
          web.archive.org URL
```

Examples:

```bash
# Download from any archived capture (archive picks the closest one)
ytb-wayback C1hjsVLcGFc

# Same, via a YouTube link
ytb-wayback 'https://www.youtube.com/watch?v=C1hjsVLcGFc'

# The most useful form: pin the snapshot and let it do everything
ytb-wayback 'https://web.archive.org/web/20241125121251/https://www.youtube.com/watch?v=C1hjsVLcGFc'

# Pin a snapshot by timestamp
ytb-wayback --date 20111027231107 C1hjsVLcGFc

# List what the archive has, without downloading
ytb-wayback --list-snapshots C1hjsVLcGFc

# Resolve and show metadata / target stream only
ytb-wayback C1hjsVLcGFc --dry-run

# Output elsewhere
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
`size_bytes`, `bytes_written`, `content_type`, `downloaded_at`.

> The `downloads/` directory and `*.part` files are git-ignored.

## Limitations

- **Media must be captured.** The archive only holds streams it recorded; a
  page capture without a media capture means nothing to download (the tool
  says so explicitly).
- **Metadata is best-effort.** Archived pages vary wildly across the years
  (consent walls, blank `<title>`, JS-rendered fields). When nothing usable is
  found, the video ID is used as the title.
- **Not all formats are archived.** Early captures often only hold one format
  (typically 360p–720p webm or mp4 at the time of capture). `--list-snapshots`
  shows what exists.
- A watch page may be captured while the media was **not** (e.g. HTTP 301/302
  redirects, robot-blocked captures).

## License

MIT