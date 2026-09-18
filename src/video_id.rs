//! Extract a YouTube video ID (and optional Wayback snapshot date) from a
//! messy set of user-provided inputs.

use anyhow::{anyhow, bail, Result};

/// A parsed video reference: the 11-char video ID plus the optional Wayback
/// snapshot timestamp (`YYYYMMDDhhmmss`) lifted from an archived URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoInput {
    pub id: String,
    pub date: Option<String>,
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

fn extract_id(s: &str) -> Result<String> {
    let patterns = [
        // Fake url form used by the Wayback internal media index.
        r"wayback-fakeurl\.archive\.org/yt/([0-9A-Za-z_\-]{11})",
        // Classic query form (also handles watch.php and extra params).
        r"[?&]v=([0-9A-Za-z_\-]{11})",
        // youtu.be short links.
        r"youtu\.be/([0-9A-Za-z_\-]{11})",
        // Embed / playlist-item paths.
        r"/(?:embed|v|shorts)/([0-9A-Za-z_\-]{11})",
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

/// Parse a user-supplied input into a `VideoInput`.
///
/// Accepted forms:
///   * a bare 11-char ID:             `C1hjsVLcGFc`
///   * a YouTube URL:                 `https://www.youtube.com/watch?v=C1hjsVLcGFc`
///   * a short link:                  `https://youtu.be/C1hjsVLcGFc`
///   * an archived playback URL:      `https://web.archive.org/web/20111027231107oe_/http://.../videoplayback?...`
///   * an archived watch page:        `https://web.archive.org/web/20241125121251/https://www.youtube.com/watch?v=C1hjsVLcGFc`
///   * a Wayback fake-url media link: `https://web.archive.org/web/2oe_/http://wayback-fakeurl.archive.org/yt/C1hjsVLcGFc`
pub fn parse(input: &str) -> Result<VideoInput> {
    let raw = input.trim();
    if raw.is_empty() {
        bail!("empty input");
    }
    if is_bare_id(raw) {
        return Ok(VideoInput {
            id: raw.to_string(),
            date: None,
        });
    }
    let decoded = percent_decode(raw);
    let date = extract_date(&decoded);
    let id = extract_id(&decoded).map_err(|_| {
        anyhow!(
            "could not find a YouTube video ID in input: {input:?}\n\
             expected a video ID, a youtube.com/youtu.be URL, or a web.archive.org URL"
        )
    })?;
    Ok(VideoInput { id, date })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(p: &str) -> String {
        parse(p).unwrap().id
    }

    fn date(p: &str) -> Option<String> {
        parse(p).unwrap().date
    }

    #[test]
    fn bare_id() {
        assert_eq!(id("C1hjsVLcGFc"), "C1hjsVLcGFc");
        assert_eq!(id("  97t7Xj_iBv0  "), "97t7Xj_iBv0");
        assert_eq!(date("C1hjsVLcGFc"), None);
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
    }

    #[test]
    fn archived_watch_page() {
        let input = "https://web.archive.org/web/20241125121251/https://www.youtube.com/watch?v=C1hjsVLcGFc";
        assert_eq!(id(input), "C1hjsVLcGFc");
        assert_eq!(date(input), Some("20241125121251".into()));
    }

    #[test]
    fn archived_media_link_is_not_resolvable_to_a_video_id() {
        // A raw archived `videoplayback` URL only carries a random stream id,
        // not the YouTube video ID, so it cannot resolve on its own.
        let input =
            "https://web.archive.org/web/20111027231107oe_/http://o-o.preferred.example.com/videoplayback?itag=45&id=0b5863b152dc1857&expire=1319781600";
        assert!(parse(input).is_err());
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
