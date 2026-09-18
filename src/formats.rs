//! Physical format ("itag") metadata and extension detection for the
//! `videoplayback` streams served by the Wayback Machine.

use std::collections::HashMap;

/// Human/media facts about a YouTube format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatInfo {
    pub ext: &'static str,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub vcodec: &'static str,
    pub acodec: &'static str,
}

impl FormatInfo {
    /// Default, caller's `format_note` style label, e.g. `720p webm`.
    pub fn note(self) -> String {
        match (self.width, self.height) {
            (_, Some(h)) => format!("{h}p {}", self.ext),
            _ => self.ext.to_string(),
        }
    }
}

fn f(ext: &'static str, w: u32, h: u32, vc: &'static str, ac: &'static str) -> FormatInfo {
    FormatInfo {
        ext,
        width: Some(w),
        height: Some(h),
        vcodec: vc,
        acodec: ac,
    }
}

fn na(ext: &'static str, vc: &'static str, ac: &'static str) -> FormatInfo {
    FormatInfo {
        ext,
        width: None,
        height: None,
        vcodec: vc,
        acodec: ac,
    }
}

/// A curated subset of the well-known YouTube itag table. Enough to name and
/// describe the files the archive most commonly holds.
fn table() -> HashMap<&'static str, FormatInfo> {
    let mut m = HashMap::new();
    let rows: &[(&str, FormatInfo)] = &[
        // Legacy progressive formats.
        ("5", f("flv", 400, 240, "h263", "mp3")),
        ("6", f("flv", 450, 270, "h263", "mp3")),
        ("13", na("3gp", "mp4v", "aac")),
        ("17", f("3gp", 176, 144, "mp4v", "aac")),
        ("18", f("mp4", 640, 360, "h264", "aac")),
        ("22", f("mp4", 1280, 720, "h264", "aac")),
        ("34", f("flv", 640, 360, "h264", "aac")),
        ("35", f("flv", 854, 480, "h264", "aac")),
        ("36", f("3gp", 320, 240, "mp4v", "aac")),
        ("37", f("mp4", 1920, 1080, "h264", "aac")),
        ("43", f("webm", 640, 360, "vp8", "vorbis")),
        ("44", f("webm", 854, 480, "vp8", "vorbis")),
        ("45", f("webm", 1280, 720, "vp8", "vorbis")),
        ("46", f("webm", 1920, 1080, "vp8", "vorbis")),
        ("59", f("mp4", 854, 480, "h264", "aac")),
        ("78", f("mp4", 854, 480, "h264", "aac")),
        // DASH audio.
        ("139", na("m4a", "none", "aac")),
        ("140", na("m4a", "none", "aac")),
        ("141", na("m4a", "none", "aac")),
        ("171", na("webm", "none", "vorbis")),
        ("172", na("webm", "none", "vorbis")),
        ("249", na("webm", "none", "opus")),
        ("250", na("webm", "none", "opus")),
        ("251", na("webm", "none", "opus")),
        // DASH video.
        ("133", f("mp4", 0, 240, "h264", "none")),
        ("134", f("mp4", 0, 360, "h264", "none")),
        ("135", f("mp4", 0, 480, "h264", "none")),
        ("136", f("mp4", 0, 720, "h264", "none")),
        ("137", f("mp4", 0, 1080, "h264", "none")),
        ("160", f("mp4", 0, 144, "h264", "none")),
        ("242", f("webm", 0, 240, "vp9", "none")),
        ("243", f("webm", 0, 360, "vp9", "none")),
        ("244", f("webm", 0, 480, "vp9", "none")),
        ("247", f("webm", 0, 720, "vp9", "none")),
        ("248", f("webm", 0, 1080, "vp9", "none")),
        ("278", f("webm", 0, 144, "vp9", "none")),
    ];
    for &(itag, info) in rows {
        m.insert(itag, info);
    }
    m
}

/// Look up `FormatInfo` for an itag string.
pub fn from_itag(itag: &str) -> Option<FormatInfo> {
    table().get(itag).copied()
}

/// Extract the `itag` value from a stream URL, as a string.
pub fn itag_from_url(url: &str) -> Option<String> {
    url.split(['&', '?']).find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == "itag").then(|| v.to_string())
    })
}

/// Map an HTTP `Content-Type` string to a file extension.
pub fn ext_from_content_type(content_type: &str) -> Option<&'static str> {
    let ct = content_type.split(';').next().unwrap_or("").trim();
    match ct {
        "video/mp4" => Some("mp4"),
        "video/webm" => Some("webm"),
        "video/x-flv" | "application/x-flv" => Some("flv"),
        "video/quicktime" => Some("mov"),
        "video/3gpp" => Some("3gp"),
        "audio/mp4" | "audio/m4a" => Some("m4a"),
        "audio/webm" => Some("webm"),
        "audio/ogg" => Some("ogg"),
        "video/mediasource" | "application/octet-stream" | "" => None,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn itag_lookup() {
        let f = from_itag("45").unwrap();
        assert_eq!(f.ext, "webm");
        assert_eq!(f.height, Some(720));
        assert_eq!(f.note(), "720p webm");
        assert_eq!(
            itag_from_url("http://x/videoplayback?itag=45&ratebypass=yes").as_deref(),
            Some("45")
        );
        let f = from_itag("22").unwrap();
        assert_eq!(f.ext, "mp4");
        assert_eq!(f.note(), "720p mp4");
    }

    #[test]
    fn unknown_itag() {
        assert!(from_itag("999").is_none());
        assert!(itag_from_url("http://x/videoplayback").is_none());
    }

    #[test]
    fn content_types() {
        assert_eq!(ext_from_content_type("video/mp4"), Some("mp4"));
        assert_eq!(
            ext_from_content_type("video/webm; charset=binary"),
            Some("webm")
        );
        assert_eq!(ext_from_content_type("text/html"), None);
        assert_eq!(ext_from_content_type("application/octet-stream"), None);
    }
}
