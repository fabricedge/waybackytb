mod cdx;
mod download;
mod formats;
mod html;
mod video_id;
mod wayback;

use anyhow::{anyhow, Context, Result};
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
  videoplayback stream directly, bypassing YouTube's modern signature ciphers.",
    after_help = "Examples:
  ytb-wayback C1hjsVLcGFc
  ytb-wayback 'https://www.youtube.com/watch?v=C1hjsVLcGFc'
  ytb-wayback 'https://web.archive.org/web/20241125121251/https://www.youtube.com/watch?v=C1hjsVLcGFc'
  ytb-wayback -o /mnt/vault C1hjsVLcGFc
  ytb-wayback --list-snapshots C1hjsVLcGFc
  ytb-wayback --dry-run C1hjsVLcGFc"
)]
struct Cli {
    /// YouTube video ID, youtube.com/youtu.be URL, or archived web.archive.org URL.
    input: String,

    /// Output directory (default: downloads/<video_id>).
    #[arg(short, long, default_value_t = String::from("downloads"))]
    output: String,

    /// Force a specific Wayback snapshot (YYYYMMDDhhmmss).
    #[arg(long)]
    date: Option<String>,

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
    let video: VideoInput = parse_video_input(&cli.input)
        .map_err(|e| {
            anyhow!(
                "{e}\n\
                 hint: pass a video ID, a youtube.com/youtu.be URL, or an archived web.archive.org URL"
            )
        })?;

    let mut wb = wayback::Wayback::new(cli.retries, cli.backoff_ms);
    wb.verbose = cli.verbose || cli.dry_run;

    let date = cli.date.as_deref().or(video.date.as_deref());
    if date.is_some() {
        wb.log(format!("forcing snapshot date: {:?}", date));
    }

    if cli.list_snapshots {
        return cdx::print_snapshots(&wb, &video.id);
    }

    // 1. Locate the archived media stream.
    let stream_url = wb.locate_stream(&video.id, date)?;

    // 2. Best-effort metadata from the archived watch page.
    let meta = wb
        .fetch_archived_page(&video.id, date)?
        .map(|body| html::extract_metadata(&body))
        .unwrap_or_default();
    let title = meta
        .title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| video.id.clone());

    let out_dir = PathBuf::from(&cli.output).join(&video.id);
    let base_name = html::sanitize_filename(&format!("{title} ({})", video.id));

    if cli.dry_run {
        println!("video:        {}", video.id);
        println!("snapshot:     {}", date.unwrap_or("(archive default)"));
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

    // 3. Download the stream.
    let dl = download::download_stream(&wb, &stream_url, &out_dir, &base_name)?;

    // 4. Write the metadata sidecar.
    write_info_json(&video, date, &meta, &stream_url, &out_dir, &dl)?;

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
            println!(
                "  format {} · {} ({}, {}/{})",
                itag,
                f.note(),
                dl.ext,
                f.vcodec,
                f.acodec
            );
        }
    }

    // Clean a leftover part file if the final exists (e.g. crashed between).
    let part = out_dir.join(format!("{base_name}.download.part"));
    if part.exists() && dl.path.exists() {
        let _ = std::fs::remove_file(&part);
    }

    Ok(())
}

fn write_info_json(
    video: &VideoInput,
    effective_date: Option<&str>,
    meta: &html::VideoMetadata,
    stream_url: &str,
    out_dir: &std::path::Path,
    dl: &download::DownloadResult,
) -> Result<()> {
    let title = meta
        .title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| video.id.clone());
    let info = json!({
        "video_id": video.id,
        "snapshot_date": effective_date,
        "title": title,
        "uploader": meta.uploader,
        "channel_id": meta.channel_id,
        "upload_date": meta.upload_date,
        "duration_seconds": meta.duration,
        "thumbnail": meta.thumbnail,
        "archive_stream_url": stream_url,
        "itag": dl.itag,
        "ext": dl.ext,
        "size_bytes": dl.bytes_total,
        "bytes_written": dl.bytes_written,
        "content_type": dl.content_type,
        "downloaded_at": now_utc(),
    });
    let file_name = html::sanitize_filename(&format!("{title} ({})", video.id));
    let path = out_dir.join(format!("{file_name}.info.json"));
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
