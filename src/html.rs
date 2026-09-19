//! Best-effort extraction of metadata (title, uploader, upload date, ...) from
//! archived YouTube watch pages. Modern pages ship their metadata inside
//! `ytInitialPlayerResponse`.

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Metadata gleaned from an archived watch page.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct VideoMetadata {
    pub title: Option<String>,
    pub uploader: Option<String>,
    pub channel_id: Option<String>,
    pub upload_date: Option<String>,
    pub duration: Option<u64>,
    pub thumbnail: Option<String>,
    pub is_live: Option<bool>,
    pub live_start: Option<String>,
    pub live_end: Option<String>,
}

/// JSON may be assigned as `var ytInitialPlayerResponse = {...};`, as
/// `window["ytInitialPlayerResponse"] = {...}`, or appear inline as a key.
/// Find the marker and brace-scan to its matching close to extract the object.
fn extract_json_assignment(html: &str, marker: &str) -> Option<serde_json::Value> {
    let start = html.find(marker)? + marker.len();
    let rest = &html[start..];

    // Skip over an assignment operator / quotes / whitespace, then anchor on
    // the start of the JSON value (`{` for objects, `[` for arrays).
    let candidate = &rest[rest.find(['=', '{', '[', ':'])?..];
    let anchor = candidate.find(['{', '['])?;
    let value = &candidate[anchor..];

    let first = value.as_bytes()[0];
    let (open, close): (u8, u8) = if first == b'{' {
        (b'{', b'}')
    } else {
        (b'[', b']')
    };

    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (i, &b) in value.as_bytes().iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b if b == open => depth += 1,
            b if b == close => {
                depth -= 1;
                if depth == 0 {
                    return serde_json::from_str(&value[..=i]).ok();
                }
            }
            _ => {}
        }
    }
    None
}

fn player_response(html: &str) -> Option<serde_json::Value> {
    extract_json_assignment(html, "ytInitialPlayerResponse")
}

fn html_entity_decode(s: &str) -> String {
    let mut out = s
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ");
    // Cheap numeric-entity pass (decimal only, enough for titles).
    let num_re = Regex::new(r"&#(\d+);").unwrap();
    out = num_re
        .replace_all(&out, |caps: &regex::Captures| {
            caps[1]
                .parse::<u32>()
                .ok()
                .and_then(char::from_u32)
                .map(|c| c.to_string())
                .unwrap_or_default()
        })
        .into_owned();
    out.trim().to_string()
}

fn clean_yt_title(title: &str) -> String {
    let t = title.trim();
    // Branding-only titles (e.g. `<title> - YouTube</title>`) carry no name.
    if t.is_empty() || t == "- YouTube" || t == "YouTube" || t == "YouTube -" {
        return String::new();
    }
    let t = t
        .strip_suffix(" - YouTube")
        .or_else(|| t.strip_prefix("YouTube - "))
        .unwrap_or(t);
    html_entity_decode(t)
}

fn meta_content(html: &str, re: &Regex) -> Option<String> {
    re.captures(html).map(|c| html_entity_decode(&c[1]))
}

fn simple_meta(html: &str, property: &str) -> Option<String> {
    let re = Regex::new(&format!(
        r#"<meta\s+[^>]*property=[\""']?{property}[\""']?[^>]*content=[\""']([^\""']+)"#
    ))
    .ok()?;
    meta_content(html, &re)
}

fn str_field(v: &serde_json::Value, path: &[&str]) -> Option<String> {
    let mut cur = v;
    for k in path {
        cur = cur.get(*k)?;
    }
    cur.as_str().map(str::to_string)
}

/// Extract all available metadata from an archived watch page.
pub fn extract_metadata(html: &str) -> VideoMetadata {
    let mut meta = VideoMetadata::default();
    let json = Regex::new(r#"(?is)<title[^>]*>(.*?)</title>"#).unwrap();

    if let Some(pr) = player_response(html) {
        meta.title = str_field(&pr, &["videoDetails", "title"]);
        meta.uploader = str_field(&pr, &["videoDetails", "author"]);
        meta.channel_id = str_field(&pr, &["videoDetails", "channelId"]);
        if let Some(v) = pr.pointer("/videoDetails/lengthSeconds") {
            meta.duration = v
                .as_str()
                .and_then(|s| s.parse().ok())
                .or_else(|| v.as_u64());
        }
        meta.thumbnail = pr
            .pointer("/videoDetails/thumbnail/thumbnails/0/url")
            .and_then(|v| v.as_str().map(str::to_string));
        meta.upload_date = str_field(
            &pr,
            &["microformat", "playerMicroformatRenderer", "publishDate"],
        )
        .map(|d| d.chars().take(10).collect());
        meta.is_live = pr
            .pointer("/videoDetails/isLive")
            .and_then(|v| v.as_bool())
            .or_else(|| {
                pr.pointer("/videoDetails/isLiveContent")
                    .and_then(|v| v.as_bool())
            });
        let live = pr.pointer("/microformat/playerMicroformatRenderer/liveBroadcastDetails");
        meta.live_start = live
            .and_then(|v| v.get("startTimestamp"))
            .and_then(|v| v.as_str().map(str::to_string));
        meta.live_end = live
            .and_then(|v| v.get("endTimestamp"))
            .and_then(|v| v.as_str().map(str::to_string));
    }

    if meta.title.is_none() {
        if let Some(m) = simple_meta(html, "og:title") {
            let t = clean_yt_title(&m);
            if !t.is_empty() {
                meta.title = Some(t);
            }
        }
    }
    if meta.title.is_none() {
        if let Some(c) = json.captures(html) {
            let t = clean_yt_title(&c[1]);
            if !t.is_empty() {
                meta.title = Some(t);
            }
        }
    }
    // Some archived pages (e.g. the player-overlay layout) carry their real
    // facts in `videoDescriptionHeaderRenderer`, which also has human dates
    // ("Mar 5, 2011") instead of ISO strings.
    if meta.title.is_none() || meta.uploader.is_none() || meta.upload_date.is_none() {
        if let Some(desc) = extract_json_assignment(html, "videoDescriptionHeaderRenderer") {
            if meta.title.is_none() {
                meta.title = desc
                    .pointer("/title/runs/0/text")
                    .and_then(|v| v.as_str().map(str::to_string));
            }
            if meta.uploader.is_none() {
                if let Some(ch) = desc.get("channel") {
                    meta.uploader = ch.as_str().map(str::to_string).or_else(|| {
                        ch.get("simpleText")
                            .and_then(|v| v.as_str())
                            .map(str::to_string)
                    });
                }
            }
            if meta.upload_date.is_none() {
                meta.upload_date = desc
                    .get("publishDate")
                    .and_then(|v| v.get("simpleText"))
                    .and_then(|v| v.as_str().map(str::to_string));
            }
        }
    }
    // Live broadcasts have liveBroadcastDetails.startTimestamp instead of a
    // publishDate; fall back to it so a sensible upload date is still found.
    if meta.upload_date.is_none() {
        if let Some(start) = &meta.live_start {
            meta.upload_date = Some(start.chars().take(10).collect());
        }
    }
    meta
}

/// A nice, filesystem-safe title (used for naming output).
pub fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    let max = 140;
    if cleaned.chars().count() > max {
        cleaned.chars().take(max).collect()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODERN_PAGE: &str = r#"<!doctype html><html><head><title>Me at the zoo - YouTube</title>
<meta property="og:title" content="Me at the zoo" /></head><body>
<script>var ytInitialPlayerResponse = {"videoDetails":{"title":"Me at the zoo","author":"jawed","channelId":"UC4QobU6STFB0P71PMvOGN5A","lengthSeconds":"19","thumbnail":{"thumbnails":[{"url":"https://i.ytimg.com/vi/jNQXAC9IVRw/hqdefault.jpg"}]}},"microformat":{"playerMicroformatRenderer":{"publishDate":"2005-04-23T00:00:00Z"}}};</script>
</body></html>"#;

    #[test]
    fn metadata_from_player_response() {
        let m = extract_metadata(MODERN_PAGE);
        assert_eq!(m.title.as_deref(), Some("Me at the zoo"));
        assert_eq!(m.uploader.as_deref(), Some("jawed"));
        assert_eq!(m.upload_date.as_deref(), Some("2005-04-23"));
        assert_eq!(m.duration, Some(19));
        assert_eq!(m.channel_id.as_deref(), Some("UC4QobU6STFB0P71PMvOGN5A"));
    }

    #[test]
    fn metadata_from_live_broadcast() {
        let page = r#"<!doctype html><html><head><title>Minecraft stream - YouTube</title></head><body>
<script>var ytInitialPlayerResponse = {"videoDetails":{"title":"САМЫЙ ЛУЧШИЙ СТРИМ","author":"SomeChannel","channelId":"UCxxxxxxxxxxxxxxxxxxxxxx","isLive":true,"isLiveContent":true,"thumbnail":{"thumbnails":[{"url":"https://i.ytimg.com/vi/S2dvG697FQo/hqdefault.jpg"}]}},"microformat":{"playerMicroformatRenderer":{"liveBroadcastDetails":{"isLiveNow":true,"startTimestamp":"2023-05-19T15:01:11+00:00"}}}};</script>
</body></html>"#;
        let m = extract_metadata(page);
        assert_eq!(m.is_live, Some(true));
        assert_eq!(m.live_start.as_deref(), Some("2023-05-19T15:01:11+00:00"));
        assert_eq!(m.live_end, None);
        // No publishDate -> upload_date falls back to the broadcast start.
        assert_eq!(m.upload_date.as_deref(), Some("2023-05-19"));
        let m2 = extract_metadata(
            r#"<script>var ytInitialPlayerResponse={"videoDetails":{"isLiveContent":false},"microformat":{"playerMicroformatRenderer":{}}};</script>"#,
        );
        assert_eq!(m2.is_live, Some(false));
    }

    #[test]
    fn title_fallbacks() {
        let m = extract_metadata("<html><head><title>My Video - YouTube</title></head></html>");
        assert_eq!(m.title.as_deref(), Some("My Video"));
        let m = extract_metadata("<html><head><title>YouTube - Old Clip</title></head></html>");
        assert_eq!(m.title.as_deref(), Some("Old Clip"));
        let m = extract_metadata(
            r#"<html><head><meta property="og:title" content="&quot;Quoted&quot; &amp; more" /></head></html>"#,
        );
        assert_eq!(m.title.as_deref(), Some("\"Quoted\" & more"));
        // Empty titles (e.g. a JS-blank `<title> - YouTube</title>`) never win.
        let m = extract_metadata("<html><head><title> - YouTube</title></head></html>");
        assert_eq!(m.title, None);
    }

    #[test]
    fn metadata_from_description_header() {
        let page = r#"<script>var ytInitialData={"contents":{"twoColumnWatchNextResults":{"results":{"results":{"contents":[{"videoSecondaryInfoRenderer":{"metadataRowContainer":{"metadataRowContainerRenderer":{}}},"items":[{"videoDescriptionHeaderRenderer":{"title":{"runs":[{"text":"Skrillex - With, You Friends"}]},"channel":{"simpleText":"MSK989"},"views":{"simpleText":"2,281,627 views"},"publishDate":{"simpleText":"Mar 5, 2011"}}}]}}]}}}};</script>"#;
        let m = extract_metadata(page);
        assert_eq!(m.title.as_deref(), Some("Skrillex - With, You Friends"));
        assert_eq!(m.uploader.as_deref(), Some("MSK989"));
        assert_eq!(m.upload_date.as_deref(), Some("Mar 5, 2011"));
    }

    #[test]
    fn no_metadata_returns_defaults() {
        let m = extract_metadata("<html><body>junk</body></html>");
        assert_eq!(m, VideoMetadata::default());
    }

    #[test]
    fn sanitize() {
        assert_eq!(
            sanitize_filename("a/b\\c:d*e?f\"g<h>i|j"),
            "a_b_c_d_e_f_g_h_i_j"
        );
        assert_eq!(sanitize_filename("...hidden..."), "hidden");
        assert_eq!(sanitize_filename("  spaced  "), "spaced");
        assert_eq!(sanitize_filename("nul\x00byte"), "nul_byte");
        assert_eq!(sanitize_filename(&"x".repeat(300)).chars().count(), 140);
    }
}
