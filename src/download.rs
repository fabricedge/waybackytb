//! Streaming download of the archived media file, with partial-fetch resume
//! (`.part` files) and a progress bar.

use crate::formats;
use crate::wayback::Wayback;
use anyhow::{bail, Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use ureq::http::header;
use ureq::Body;

type Resp = ureq::http::Response<Body>;

/// Outcome of a successful media download.
#[derive(Debug, Clone)]
pub struct DownloadResult {
    /// Absolute path of the saved file.
    pub path: PathBuf,
    /// File extension, e.g. `webm`.
    pub ext: String,
    /// Byte count written on this run (not counting resumed bytes).
    pub bytes_written: u64,
    /// Bytes already present in the resumed `.part` file.
    pub bytes_resumed: u64,
    /// Total bytes on disk after completion.
    pub bytes_total: u64,
    /// `Content-Type` the archive reported.
    pub content_type: String,
    /// `itag` detected from the stream URL, if any.
    pub itag: Option<String>,
}

fn human(n: f64) -> String {
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut val = n;
    let mut i = 0;
    while val >= 1024.0 && i < units.len() - 1 {
        val /= 1024.0;
        i += 1;
    }
    match i {
        0 => format!("{val:.0} {}", units[i]),
        _ => format!("{val:.2} {}", units[i]),
    }
}

fn infer_ext(url: &str, content_type: &str) -> (Option<String>, String) {
    let itag = formats::itag_from_url(url);
    let ext = itag
        .as_ref()
        .and_then(|t| formats::from_itag(t))
        .map(|f| f.ext.to_string())
        .or_else(|| formats::ext_from_content_type(content_type).map(str::to_string))
        .unwrap_or_else(|| "bin".to_string());
    (itag, ext)
}

/// Download (or resume) the archived media stream `url` into `out_dir`.
///
/// `base_name` is the sanitised filename *without extension*; the final file
/// is `<base_name>.<ext>`, with the extension derived from the stream's itag
/// or content type. In-flight data lives in `<base_name>.download.part` so an
/// interrupted run can be resumed later with `-r`-style retries.
pub fn download_stream(
    wb: &Wayback,
    url: &str,
    out_dir: &Path,
    base_name: &str,
) -> Result<DownloadResult> {
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("cannot create output directory {}", out_dir.display()))?;

    let part = part_path(out_dir, base_name);
    let resumed = existing_len(&part);

    wb.log(format!(
        "requesting stream (resume at byte {}) ...",
        resumed
    ));

    let resp = if resumed > 0 {
        request_range(wb, url, resumed)?
    } else {
        request_full(wb, url)?
    };

    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_lowercase();

    // The archive sometimes answers the media request with an HTML error page.
    if is_html(&content_type) || resp.status().as_u16() == 404 {
        bail!(
            "the archive returned `{content_type}` instead of media (stream no longer available)"
        );
    }

    let (itag, ext) = infer_ext(url, &content_type);

    // Skip a fully downloaded file.
    let final_path = out_dir.join(format!("{base_name}.{ext}"));
    if final_path.exists() && part.exists() {
        let _ = std::fs::remove_file(&part);
    }
    if final_path.exists() {
        let bytes_total = std::fs::metadata(&final_path)?.len();
        wb.log(format!("already downloaded: {}", final_path.display()));
        return Ok(DownloadResult {
            path: final_path,
            ext: ext.clone(),
            bytes_written: 0,
            bytes_resumed: bytes_total,
            bytes_total,
            content_type,
            itag,
        });
    }

    let total_new: Option<u64> = resp
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());

    let resp_status = resp.status().as_u16();
    let start_from = match resp_status {
        206 => resumed,
        _ => 0, // server ignored Range or answered 200 -> start over
    };

    if start_from == 0 && resumed > 0 {
        let _ = std::fs::remove_file(&part);
    }

    let mut out = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&part)
        .with_context(|| format!("cannot open {}", part.display()))?;

    let pb = progress_bar(total_new.map(|t| t + start_from), start_from);
    let mut reader = resp.into_body().into_reader();
    let mut buf = vec![0u8; 256 * 1024];
    let mut written: u64 = 0;
    loop {
        let n = match reader.read(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                drop(out);
                pb.finish_and_clear();
                let _ = std::fs::remove_file(&part);
                bail!("download interrupted: {e}");
            }
        };
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).context("write to disk failed")?;
        written += n as u64;
        pb.inc(n as u64);
    }
    pb.finish_and_clear();
    drop(out);

    std::fs::rename(&part, &final_path)
        .with_context(|| format!("renaming {part:?} to {}", final_path.display()))?;

    Ok(DownloadResult {
        path: final_path,
        ext,
        bytes_written: written,
        bytes_resumed: start_from,
        bytes_total: start_from + written,
        content_type,
        itag,
    })
}

fn request_range(wb: &Wayback, url: &str, from: u64) -> Result<Resp> {
    wb.agent()
        .get(url)
        .header("Accept", "video/*")
        .header("Range", &format!("bytes={from}-"))
        .call()
        .context("resume request failed")
}

fn request_full(wb: &Wayback, url: &str) -> Result<Resp> {
    wb.agent()
        .get(url)
        .header("Accept", "video/*")
        .call()
        .context("stream request failed")
}

fn progress_bar(total: Option<u64>, started: u64) -> ProgressBar {
    let pb = if let Some(total) = total {
        ProgressBar::new(total)
    } else {
        ProgressBar::new_spinner()
    };
    let template = "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, eta {eta})";
    let style = ProgressStyle::with_template(template)
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .progress_chars("=>-");
    pb.set_style(style);
    if started > 0 {
        pb.set_position(started);
    }
    pb
}

fn part_path(out_dir: &Path, base_name: &str) -> PathBuf {
    out_dir.join(format!("{base_name}.download.part"))
}

fn existing_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn is_html(ct: &str) -> bool {
    ct.contains("html") || ct.starts_with("text/") || ct.is_empty()
}

pub fn size_human(n: u64) -> String {
    human(n as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ext_inference() {
        assert_eq!(
            infer_ext("https://x/videoplayback?itag=45", "video/webm").1,
            "webm"
        );
        assert_eq!(
            infer_ext("https://x/videoplayback?itag=22", "video/webm").1,
            "mp4"
        );
        assert_eq!(infer_ext("https://x/videoplayback", "video/mp4").1, "mp4");
        assert_eq!(infer_ext("https://x/videoplayback", "text/html").1, "bin");
        assert_eq!(
            infer_ext("https://x/videoplayback?itag=45", "video/webm")
                .0
                .as_deref(),
            Some("45")
        );
    }

    #[test]
    fn html_detection() {
        assert!(is_html("text/html; charset=utf-8"));
        assert!(is_html("application/xhtml+xml"));
        assert!(is_html(""));
        assert!(!is_html("video/webm"));
        assert!(!is_html("video/mp4"));
    }

    #[test]
    fn human_sizes() {
        assert_eq!(size_human(0), "0 B");
        assert_eq!(size_human(1024), "1.00 KiB");
        assert!(size_human(5 * 1024 * 1024).ends_with("MiB"));
        assert!(size_human(2 * 1024 * 1024 * 1024).ends_with("GiB"));
    }

    #[test]
    fn part_names() {
        let p = part_path(Path::new("/tmp"), "My Video (C1hjsVLcGFc)");
        assert_eq!(
            p.file_name().unwrap(),
            "My Video (C1hjsVLcGFc).download.part"
        );
        assert_eq!(existing_len(Path::new("/definitely/not/here.part")), 0);
    }
}
