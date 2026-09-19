//! Human-friendly listing of the Wayback Machine's CDX snapshots for a video.

use crate::wayback::Wayback;
use anyhow::Result;

/// Query the CDX index and print every archived capture of a watch page.
pub fn print_snapshots(wb: &Wayback, id: &str) -> Result<()> {
    let snaps = wb.list_snapshots(id)?;
    if snaps.is_empty() {
        println!("no archived captures found for video {id}");
        return Ok(());
    }

    println!(
        "{} archived capture(s) for video {id} (watch & live pages)",
        snaps.len()
    );
    let (htmls, media) = snaps.iter().fold((0, 0), |(h, m), s| {
        (
            h + if s.mimetype.contains("html") { 1 } else { 0 },
            m + if s.mimetype.contains("video") { 1 } else { 0 },
        )
    });
    println!("  -> {} watch page(s), {} media capture(s)\n", htmls, media);
    println!(
        "{:<19} {:<6} {:<24} length",
        "timestamp", "status", "mimetype"
    );
    println!("{}", "-".repeat(63));
    for s in &snaps {
        let bytes = s.length.parse::<u64>().unwrap_or(0);
        let len = if bytes >= 1024 * 1024 {
            format!("{:.1} MiB", bytes as f64 / 1024.0 / 1024.0)
        } else {
            format!("{:.0} KiB", bytes as f64 / 1024.0)
        };
        println!(
            "{:<19} {:<6} {:<24} {len}",
            pretty_ts(&s.timestamp),
            s.statuscode,
            s.mimetype,
        );
    }

    // Stored copies of the actual media (the internal fake-url index).
    match wb.media_captures(id) {
        Ok(caps) if !caps.is_empty() => {
            println!(
                "\n{} archived media copy(ies) for video {id} (stored video files)",
                caps.len()
            );
            println!("{:<19} {:<16} size", "timestamp", "mimetype");
            println!("{}", "-".repeat(63));
            for c in &caps {
                println!(
                    "{:<19} {:<16} {}",
                    pretty_ts(&c.timestamp),
                    c.mimetype,
                    crate::download::size_human(c.length)
                );
            }
            println!(
                "\nhint: by default the largest copy (highest quality) is downloaded; \
                 use --max-size (e.g. 150M) to pick the largest copy that still fits"
            );
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn pretty_ts(ts: &str) -> String {
    if ts.len() == 14 {
        format!(
            "{}-{}-{} {}:{}:{}",
            &ts[0..4],
            &ts[4..6],
            &ts[6..8],
            &ts[8..10],
            &ts[10..12],
            &ts[12..14]
        )
    } else {
        ts.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps() {
        assert_eq!(pretty_ts("20241125121251"), "2024-11-25 12:12:51");
        assert_eq!(pretty_ts("junk"), "junk");
    }
}
