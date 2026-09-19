//! Extract a YouTube video ID (and optional Wayback snapshot date) from a
//! messy set of user-provided inputs.

use anyhow::{bail, Result};

/// What the user handed us, classified by the shape of the link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoInput {
    /// A video identified by its 11-char ID (watch / live / shorts / embed
    /// page), plus the optional Wayback snapshot timestamp and — when the
    /// input was an archived page — the exact inner URL the user linked.
    Video {
        id: String,
        date: Option<String>,
        inner: Option<String>,
    },
    /// A raw archived media link (the archive player's `oe_` `videoplayback`
    /// URL, or any other archived file URL with no resolvable video ID). The
    /// exact URL is downloaded as-is.
    Media { url: String, date: Option<String> },
}

#[cfg_attr(not(test), allow(dead_code))]
impl VideoInput {
    /// The Wayback snapshot timestamp lifted from an archived URL, if any.
    pub fn date(&self) -> Option<&str> {
        match self {
            VideoInput::Video { date, .. } | VideoInput::Media { date, .. } => date.as_deref(),
        }
    }

    /// The exact inner (non-archive) URL the user linked, when available.
    pub fn inner(&self) -> Option<&str> {
        match self {
            VideoInput::Video { inner, .. } => inner.as_deref(),
            VideoInput::Media { .. } => None,
        }
    }
}

/// Match exactly an 11-char YouTube video ID.
fn is_bare_id(s: &str) -> bool {
    s.len() == 11
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Extract the snapshot timestamp (`YYYYMMDDhhmmss`) from an archived URL.
fn extract_date(s: &str) -> Option<String> {
    let re = regex::Regex::new(r"(?:web\.archive\.org|archive\.org)/web/(\d{14})").ok()?;
    re.captures(s).map(|c| c[1].to_string())
}

/// Extract the inner URL from an archived link
/// (`.../web/<timestamp>[<flags>]/<inner-url>`), if present.
fn extract_inner(s: &str) -> Option<String> {
    let re = regex::Regex::new(
        r"(?:web\.archive\.org|archive\.org)/web/[0-9A-Za-z_*]+[0-9A-Za-z_*]*/(https?://.*)",
    )
    .ok()?;
    re.captures(s).map(|c| c[1].to_string())
}

fn extract_id(s: &str) -> Result<String> {
    let patterns = [
        // Fake url form used by the Wayback internal media index.
        r"wayback-fakeurl\.archive\.org/yt/([0-9A-Za-z_\-]{11})",
        // Classic query form (also handles watch.php and extra params).
        r"[?&]v=([0-9A-Za-z_\-]{11})",
        // youtu.be short links.
        r"youtu\.be/([0-9A-Za-z_\-]{11})",
        // Embed / playlist-item / live-broadcast / watch paths.
        r"/(?:embed|v|shorts|live|watch)/([0-9A-Za-z_\-]{11})",
        // Percent-encoded v%3D<id> forms.
        r"v%3[dD]([0-9A-Za-z_\-]{11})",
    ];
    for p in patterns {
        let re = regex::Regex::new(p)?;
        if let Some(c) = re.captures(s) {
            return Ok(c[1].to_string());
        }
    }
    if is_bare_id(s) {
        return Ok(s.to_string());
    }
    bail!("could not find a YouTube video ID in: {s:?}")
}

/// Percent-decode a byte string (`%XX`), leaving `+` untouched.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_val(bytes[i + 1]);
            let lo = hex_val(bytes[i + 2]);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Honestly note when an archived URL is a YouTube page that can never hold a
/// single file (channel / user / playlist / search / feed).
fn looks_like_youtube_non_video(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    [
        "youtube.com/channel",
        "youtube.com/user",
        "youtube.com/c/",
        "youtube.com/playlist",
        "youtube.com/@",
        "youtube.com/results",
        "youtube.com/feed",
        "youtube.com/gaming",
        "youtube.com/live_chat",
        "?list=",
    ]
    .iter()
    .any(|frag| lower.contains(frag))
}

/// Whether an archived URL is itself a stored media file (as opposed to a
/// page). The archive player's "Copy video address" links are `oe_` replay
/// URLs of `videoplayback` streams.
fn is_media_like(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    lower.contains("oe_")
        || lower.contains("videoplayback")
        || lower.contains("googlevideo")
        || lower.contains(".c.youtube.com")
        || lower.contains("o-o.preferred")
}

/// Parse a user-supplied input into a `VideoInput`.
///
/// Accepted forms:
///   * a bare 11-char ID:             `C1hjsVLcGFc`
///   * a YouTube URL:                 `https://www.youtube.com/watch?v=C1hjsVLcGFc`
///   * a short link:                  `https://youtu.be/C1hjsVLcGFc`
///   * an archived playback URL:      `https://web.archive.org/web/20111027231107oe_/http://.../videoplayback?...`
///   * an archived watch page:        `https://web.archive.org/web/20241125121251/https://www.youtube.com/watch?v=C1hjsVLcGFc`
///   * an archived live page:         `https://web.archive.org/web/20230520013354/https://www.youtube.com/live/S2dvG697FQo`
///   * a Wayback fake-url media link: `https://web.archive.org/web/2oe_/http://wayback-fakeurl.archive.org/yt/C1hjsVLcGFc`
pub fn parse(input: &str) -> Result<VideoInput> {
    let raw = input.trim();
    if raw.is_empty() {
        bail!("empty input");
    }
    if is_bare_id(raw) {
        return Ok(VideoInput::Video {
            id: raw.to_string(),
            date: None,
            inner: None,
        });
    }
    let decoded = percent_decode(raw);
    let date = extract_date(&decoded);
    let inner = extract_inner(&decoded);
    if let Ok(id) = extract_id(&decoded) {
        return Ok(VideoInput::Video { id, date, inner });
    }
    if inner.is_some() {
        if looks_like_youtube_non_video(&decoded) {
            bail!(
                "that looks like a YouTube channel/playlist/search page, not a single video.\n\
                 hint: pass a watch/live/shorts/embed URL or an 11-char video ID"
            );
        }
        if is_media_like(&decoded) {
            return Ok(VideoInput::Media { url: decoded, date });
        }
    }
    bail!(
        "could not find a YouTube video ID in: {raw:?}\n\
         expected a video ID, a youtube.com/youtu.be URL, an archived media link, \
         or a web.archive.org watch/live URL"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(p: &str) -> String {
        match parse(p).unwrap() {
            VideoInput::Video { id, .. } => id,
            VideoInput::Media { .. } => panic!("expected a video, got a media link"),
        }
    }

    fn date(p: &str) -> Option<String> {
        parse(p).unwrap().date().map(str::to_string)
    }

    fn inner(p: &str) -> Option<String> {
        parse(p).unwrap().inner().map(str::to_string)
    }

    #[test]
    fn bare_id() {
        assert_eq!(id("C1hjsVLcGFc"), "C1hjsVLcGFc");
        assert_eq!(id("  97t7Xj_iBv0  "), "97t7Xj_iBv0");
        assert_eq!(date("C1hjsVLcGFc"), None);
        assert_eq!(inner("C1hjsVLcGFc"), None);
    }

    #[test]
    fn youtube_watch_url() {
        assert_eq!(
            id("https://www.youtube.com/watch?v=C1hjsVLcGFc"),
            "C1hjsVLcGFc"
        );
        assert_eq!(
            id("http://youtube.com/watch?v=C1hjsVLcGFc&list=PL123&index=2"),
            "C1hjsVLcGFc"
        );
        assert_eq!(
            id("https://m.youtube.com/watch?v=BaW_jenozKc"),
            "BaW_jenozKc"
        );
        assert_eq!(
            id("https://www.youtube.com:80/watch.php?v=ELTFsLT73fA&search=soccer"),
            "ELTFsLT73fA"
        );
        assert_eq!(date("https://www.youtube.com/watch?v=C1hjsVLcGFc"), None);
    }

    #[test]
    fn short_and_embed_links() {
        assert_eq!(id("https://youtu.be/C1hjsVLcGFc"), "C1hjsVLcGFc");
        assert_eq!(
            id("https://www.youtube.com/embed/C1hjsVLcGFc"),
            "C1hjsVLcGFc"
        );
        assert_eq!(
            id("https://www.youtube.com/v/C1hjsVLcGFc?version=3"),
            "C1hjsVLcGFc"
        );
        assert_eq!(
            id("https://www.youtube.com/shorts/C1hjsVLcGFc"),
            "C1hjsVLcGFc"
        );
        assert_eq!(
            id("https://www.youtube.com/watch/C1hjsVLcGFc"),
            "C1hjsVLcGFc"
        );
        assert_eq!(
            id("https://youtube-nocookie.com/embed/C1hjsVLcGFc"),
            "C1hjsVLcGFc"
        );
        assert_eq!(
            id("https://music.youtube.com/watch?v=C1hjsVLcGFc"),
            "C1hjsVLcGFc"
        );
    }

    #[test]
    fn live_broadcast_links() {
        assert_eq!(
            id("https://www.youtube.com/live/S2dvG697FQo"),
            "S2dvG697FQo"
        );
        assert_eq!(
            id("https://m.youtube.com/live/S2dvG697FQo?feature=share"),
            "S2dvG697FQo"
        );
        let archived = "https://web.archive.org/web/20230520013354/https://www.youtube.com/live/S2dvG697FQo?feature=share";
        assert_eq!(id(archived), "S2dvG697FQo");
        assert_eq!(date(archived), Some("20230520013354".into()));
    }

    #[test]
    fn archived_watch_page() {
        let input = "https://web.archive.org/web/20241125121251/https://www.youtube.com/watch?v=C1hjsVLcGFc";
        assert_eq!(id(input), "C1hjsVLcGFc");
        assert_eq!(date(input), Some("20241125121251".into()));
        assert_eq!(
            inner(input),
            Some("https://www.youtube.com/watch?v=C1hjsVLcGFc".into())
        );
    }

    #[test]
    fn archived_live_page() {
        let input = "https://web.archive.org/web/20230520013354/https://www.youtube.com/live/S2dvG697FQo?feature=share";
        assert_eq!(id(input), "S2dvG697FQo");
        assert_eq!(date(input), Some("20230520013354".into()));
        assert_eq!(
            inner(input),
            Some("https://www.youtube.com/live/S2dvG697FQo?feature=share".into())
        );
    }

    #[test]
    fn archived_media_link_is_direct_media() {
        // A raw archived `videoplayback` URL only carries a random stream id,
        // not the YouTube video ID — it is downloaded exactly as linked.
        let input =
            "https://web.archive.org/web/20111027231107oe_/http://o-o.preferred.example.com/videoplayback?itag=45&id=0b5863b152dc1857&expire=1319781600";
        match parse(input).unwrap() {
            VideoInput::Media { url, date } => {
                assert_eq!(url, input);
                assert_eq!(date.as_deref(), Some("20111027231107"));
            }
            VideoInput::Video { .. } => panic!("expected a media link"),
        }
    }

    #[test]
    fn non_video_youtube_archive_pages_are_rejected() {
        for url in [
            "https://web.archive.org/web/20240101000000/https://www.youtube.com/channel/UCabcxyz123",
            "https://web.archive.org/web/20240101000000/https://www.youtube.com/playlist?list=PL123",
            "https://web.archive.org/web/20240101000000/https://www.youtube.com/@dankpods",
        ] {
            let err = parse(url).unwrap_err().to_string();
            assert!(
                err.contains("channel/playlist") || err.contains("channel/playlist/search"),
                "unexpected error for {url}: {err}"
            );
        }
    }

    #[test]
    fn fakeurl_link() {
        let input =
            "https://web.archive.org/web/2oe_/http://wayback-fakeurl.archive.org/yt/C1hjsVLcGFc";
        assert_eq!(id(input), "C1hjsVLcGFc");
        assert_eq!(date(input), None); // magic timestamp `2` carries no date
    }

    #[test]
    fn percent_encoded_archived_url() {
        let input = "https://web.archive.org/web/20120712231619/http%3A//www.youtube.com/watch%3Fv%3DAkhihxRKcrs%26gl%3DUS%26hl%3Den";
        assert_eq!(id(input), "AkhihxRKcrs");
        assert_eq!(date(input), Some("20120712231619".into()));
    }

    #[test]
    fn invalid_inputs() {
        assert!(parse("").is_err());
        assert!(parse("   ").is_err());
        assert!(parse("https://example.com/random").is_err());
        assert!(parse("watch?v=tooshort").is_err());
    }
}
