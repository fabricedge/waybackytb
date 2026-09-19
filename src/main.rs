mod cdx;
mod download;
mod formats;
mod html;
mod size;
mod video_id;
mod wayback;

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use serde_json::json;
use std::path::PathBuf;

use crate::video_id::{parse as parse_video_input, VideoInput};

#[derive(Parser, Debug)]
#[command(
    name = "ytb-wayback",
    version,
    about = "Download YouTube videos preserved by the Wayback Machine",
    long_about = "ytb-wayback finds the media that the Internet Archive captured for a \
                  YouTube video and downloads it.

  Accepts a bare video ID, a youtube.com/youtu.be URL, or — most usefully — an \
  archived web.archive.org link such as:
    https://web.archive.org/web/20241125121251/https://www.youtube.com/watch?v=C1hjsVLcGFc

  The archived media is fetched through the archive's internal fake-url index \
  (http://wayback-fakeurl.archive.org/yt/<id>) which serves the stored \
  videoplayback stream directly, bypassing YouTube's modern signature ciphers.

  Directly linking an archived media file (the player's \"Copy video address\" \
  link, an oe_ videoplayback URL) downloads that exact file.

  By default the largest stored copy (highest quality) is chosen; --max-size \
  picks the largest copy that still fits the limit.",
    after_help = "Examples:
  ytb-wayback C1hjsVLcGFc
  ytb-wayback 'https://www.youtube.com/watch?v=C1hjsVLcGFc'
  ytb-wayback 'https://web.archive.org/web/20241125121251/https://www.youtube.com/watch?v=C1hjsVLcGFc'
  ytb-wayback -o /mnt/vault C1hjsVLcGFc
  ytb-wayback --list-snapshots C1hjsVLcGFc
  ytb-wayback --dry-run C1hjsVLcGFc
  ytb-wayback --max-size 150M C1hjsVLcGFc
  ytb-wayback 'https://web.archive.org/web/20111027231107oe_/http://o-o.preferred.c.youtube.com/videoplayback?itag=34&id=0b5863b152dc1857&expire=1319781600'"
)]
struct Cli {
    /// YouTube video ID, youtube.com/youtu.be URL, or archived web.archive.org URL.
    #[arg(allow_hyphen_values = true)]
    input: String,

    /// Output directory (default: downloads/<video_id>).
    #[arg(short, long, default_value_t = String::from("downloads"))]
    output: String,

    /// Force a specific Wayback snapshot (YYYYMMDDhhmmss).
    #[arg(long)]
    date: Option<String>,

    /// Download at most SIZE bytes; the largest stored copy under the limit
    /// is chosen. Accepted: 150M, 1.5G, 500MiB, 104857600 (bare = bytes).
    /// Default: largest stored copy (highest quality).
    #[arg(long, value_name = "SIZE")]
    max_size: Option<String>,

    /// Query the Wayback CDX index and print available snapshots, then exit.
    #[arg(long)]
    list_snapshots: bool,

    /// Resolve and fetch metadata only; do not download the media.
    #[arg(short, long)]
    dry_run: bool,

    /// Number of retries for transient archive errors.
    #[arg(short, long, default_value_t = 3)]
    retries: u32,

    /// Initial retry backoff, in milliseconds.
    #[arg(long, default_value_t = 2000)]
    backoff_ms: u64,

    /// Verbose progress logs.
    #[arg(short, long)]
    verbose: bool,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let input: VideoInput = parse_video_input(&cli.input).map_err(|e| {
        anyhow!(
            "{e}\n\
             hint: pass a video ID, a youtube.com/youtu.be URL, or an archived web.archive.org URL"
        )
    })?;
    let max_size = cli.max_size.as_deref().map(size::parse_size).transpose()?;

    let mut wb = wayback::Wayback::new(cli.retries, cli.backoff_ms);
    wb.verbose = cli.verbose || cli.dry_run;

    match &input {
        VideoInput::Video { id, date, inner } => {
            let date = cli.date.as_deref().or(date.as_deref());
            run_video(&cli, &mut wb, id, date, inner.as_deref(), max_size)
        }
        VideoInput::Media { url, date } => {
            let date = cli.date.as_deref().or(date.as_deref());
            run_media(&cli, &mut wb, url, date, max_size)
        }
    }
}

fn run_video(
    cli: &Cli,
    wb: &mut wayback::Wayback,
    id: &str,
    date: Option<&str>,
    inner: Option<&str>,
    max_size: Option<u64>,
) -> Result<()> {
    if let Some(d) = date {
        wb.log(format!("forcing snapshot date: {d}"));
    }

    if cli.list_snapshots {
        return cdx::print_snapshots(wb, id);
    }

    // Choose which stored copy to replay. A user-pinned snapshot fixes the
    // copy implicitly, so copy selection only applies otherwise.
    let selected: Option<wayback::MediaCapture> = if date.is_some() {
        None
    } else {
        match wb.media_captures(id) {
            Ok(caps) => wayback::Wayback::select_capture(&caps, max_size).cloned(),
            Err(e) => {
                wb.log(format!(
                    "could not list stored media copies, falling back to archive default: {e:#}"
                ));
                None
            }
        }
    };

    let stream_url = match &selected {
        Some(c) => {
            wb.log(format!(
                "chosen stored copy: {} · {} · {}",
                c.timestamp,
                c.mimetype,
                download::size_human(c.length)
            ));
            wb.locate_stream(id, Some(&c.timestamp))?
        }
        None => wb.locate_stream(id, date)?,
    };
    if let (Some(c), Some(limit)) = (&selected, max_size) {
        if c.length > limit {
            wb.log(format!(
                "no stored copy under {}; downloading the smallest copy ({})",
                download::size_human(limit),
                download::size_human(c.length)
            ));
        }
    }
    let snapshot =
        wayback::Wayback::stream_timestamp(&stream_url).or_else(|| date.map(str::to_string));

    // Best-effort metadata from the archived watch page.
    let meta = wb
        .fetch_archived_page(id, date, inner)?
        .map(|body| html::extract_metadata(&body))
        .unwrap_or_default();
    let title = fallback_title(&meta, id);

    let out_dir = PathBuf::from(&cli.output).join(id);
    let base_name = html::sanitize_filename(&format!("{title} ({id})"));

    if cli.dry_run {
        println!("kind:         video (page link)");
        println!("video:        {id}");
        println!(
            "snapshot:     {}",
            snapshot.as_deref().unwrap_or("(archive default)")
        );
        println!("title:        {title}");
        if let Some(u) = &meta.uploader {
            println!("uploader:     {u}");
        }
        if meta.is_live == Some(true) {
            println!("type:         live broadcast");
        }
        if let Some(s) = &meta.live_start {
            println!("live start:   {s}");
        }
        if let Some(e) = &meta.live_end {
            println!("live end:     {e}");
        }
        if let Some(d) = &meta.upload_date {
            println!("upload date:  {d}");
        }
        if let Some(c) = &selected {
            println!(
                "stored copy:  {} · {} · {}",
                c.timestamp,
                c.mimetype,
                download::size_human(c.length)
            );
        }
        println!("stream:       {stream_url}");
        println!("output dir:   {}", out_dir.display());
        return Ok(());
    }

    // Download the stream.
    let dl = download::download_stream(wb, &stream_url, &out_dir, &base_name)?;

    // Metadata sidecar.
    write_info_json(
        &InfoCtx {
            id: Some(id),
            kind: "video",
            snapshot_date: snapshot.as_deref(),
            max_size,
            stream_url: &stream_url,
            out_dir: &out_dir,
        },
        &meta,
        &dl,
    )?;

    println!(
        "saved {:>10} -> {}",
        download::size_human(dl.bytes_total),
        dl.path.display()
    );
    if dl.bytes_resumed > 0 {
        println!("  (resumed {})", download::size_human(dl.bytes_resumed));
    }
    if let Some(itag) = &dl.itag {
        if let Some(f) = formats::from_itag(itag) {
            let audio = if f.acodec == "none" {
                "video-only, no audio track".to_string()
            } else {
                format!("video+audio ({}/{})", f.vcodec, f.acodec)
            };
            println!("  format {} · {} ({})", itag, f.note(), audio);
        }
    }

    // Clean a leftover part file if the final exists (e.g. crashed between).
    let part = out_dir.join(format!("{base_name}.download.part"));
    if part.exists() && dl.path.exists() {
        let _ = std::fs::remove_file(&part);
    }

    Ok(())
}

fn run_media(
    cli: &Cli,
    wb: &mut wayback::Wayback,
    url: &str,
    date: Option<&str>,
    max_size: Option<u64>,
) -> Result<()> {
    if cli.list_snapshots {
        bail!("--list-snapshots applies to videos; a direct media link is downloaded as-is");
    }
    if let Some(d) = date {
        wb.log(format!("forcing snapshot date: {d}"));
    }
    if max_size.is_some() {
        wb.log(
            "--max-size is ignored for direct media links: the archive stores only the linked file",
        );
    }

    // Best-effort video ID from the link, for naming and metadata.
    let media_id = media_video_id(url);

    let (stream_url, _ctype) = match date {
        Some(d) => {
            let pinned = wayback::Wayback::pin_date(url, d)?;
            wb.probe(&pinned)?
        }
        None => wb.probe(url)?,
    };
    let snapshot =
        wayback::Wayback::stream_timestamp(&stream_url).or_else(|| date.map(str::to_string));

    let meta = match &media_id {
        Some(id) => wb
            .fetch_archived_page(id, date, None)?
            .map(|body| html::extract_metadata(&body))
            .unwrap_or_default(),
        None => html::VideoMetadata::default(),
    };

    let id_or_media = media_id.as_deref().unwrap_or("media");
    let title = fallback_title(&meta, id_or_media);

    let out_dir = PathBuf::from(&cli.output).join(id_or_media);
    let base_name = html::sanitize_filename(&format!("{title} ({id_or_media})"));

    if cli.dry_run {
        println!("kind:         media (direct archived link)");
        if let Some(id) = &media_id {
            println!("video:        {id}");
        }
        println!(
            "snapshot:     {}",
            snapshot.as_deref().unwrap_or("(archive default)")
        );
        println!("title:        {title}");
        if let Some(u) = &meta.uploader {
            println!("uploader:     {u}");
        }
        if let Some(d) = &meta.upload_date {
            println!("upload date:  {d}");
        }
        println!("stream:       {stream_url}");
        println!("output dir:   {}", out_dir.display());
        return Ok(());
    }

    let dl = download::download_stream(wb, &stream_url, &out_dir, &base_name)?;

    write_info_json(
        &InfoCtx {
            id: media_id.as_deref(),
            kind: "media",
            snapshot_date: snapshot.as_deref(),
            max_size,
            stream_url: &stream_url,
            out_dir: &out_dir,
        },
        &meta,
        &dl,
    )?;

    println!(
        "saved {:>10} -> {}",
        download::size_human(dl.bytes_total),
        dl.path.display()
    );
    if dl.bytes_resumed > 0 {
        println!("  (resumed {})", download::size_human(dl.bytes_resumed));
    }
    if let Some(itag) = &dl.itag {
        if let Some(f) = formats::from_itag(itag) {
            let audio = if f.acodec == "none" {
                "video-only, no audio track".to_string()
            } else {
                format!("video+audio ({}/{})", f.vcodec, f.acodec)
            };
            println!("  format {} · {} ({})", itag, f.note(), audio);
        }
    }

    Ok(())
}

/// The 11-char video ID embedded in a `videoplayback` link, if any.
fn media_video_id(url: &str) -> Option<String> {
    let re = regex::Regex::new(r"[?&]video_id=([0-9A-Za-z_\-]{11})").ok()?;
    re.captures(url).map(|c| c[1].to_string())
}

fn fallback_title(meta: &html::VideoMetadata, id: &str) -> String {
    meta.title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| id.to_string())
}

struct InfoCtx<'a> {
    id: Option<&'a str>,
    kind: &'a str,
    snapshot_date: Option<&'a str>,
    max_size: Option<u64>,
    stream_url: &'a str,
    out_dir: &'a std::path::Path,
}

fn write_info_json(
    ctx: &InfoCtx,
    meta: &html::VideoMetadata,
    dl: &download::DownloadResult,
) -> Result<()> {
    let id_str = ctx.id.unwrap_or("media");
    let title = meta
        .title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| id_str.to_string());
    let info = json!({
        "video_id": ctx.id,
        "input_kind": ctx.kind,
        "snapshot_date": ctx.snapshot_date,
        "max_size": ctx.max_size,
        "title": title,
        "uploader": meta.uploader,
        "channel_id": meta.channel_id,
        "upload_date": meta.upload_date,
        "duration_seconds": meta.duration,
        "thumbnail": meta.thumbnail,
        "is_live": meta.is_live,
        "live_start": meta.live_start,
        "live_end": meta.live_end,
        "archive_stream_url": ctx.stream_url,
        "itag": dl.itag,
        "ext": dl.ext,
        "has_audio": dl.itag
            .as_deref()
            .and_then(formats::from_itag)
            .map(|f| f.acodec != "none")
            .unwrap_or(true),
        "size_bytes": dl.bytes_total,
        "bytes_written": dl.bytes_written,
        "content_type": dl.content_type,
        "downloaded_at": now_utc(),
    });
    let file_name = html::sanitize_filename(&format!("{title} ({id_str})"));
    let path = ctx.out_dir.join(format!("{file_name}.info.json"));
    let bytes = serde_json::to_string_pretty(&info).context("serializing metadata")?;
    std::fs::write(&path, bytes + "\n").with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Simple UTC timestamp string without pulling in a chrono dependency.
fn now_utc() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = now / 86_400;
    let (y, m, d) = civil_from_days(days as i64);
    let secs_of_day = now % 86_400;
    let (hh, mm, ss) = (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    );
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02} UTC")
}

/// Convert days since 1970-01-01 to a (year, month, day) civil date.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_900), (2024, 6, 26));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(15_000), (2011, 1, 26));
    }
}
