//! Wayback Machine HTTP plumbing: retries, the fake-url media endpoint, the
//! archived watch-page fetch, and the CDX snapshot index.

use anyhow::{bail, Context, Result};
use std::io::Read;
use std::time::Duration;
use ureq::config::Config;
use ureq::http::header;
use ureq::{Agent, ResponseExt};

pub const USER_AGENT: &str = concat!(
    "ytb-wayback/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/annexare/ytb-wayback; archive retrieval utility)"
);

const BASE: &str = "https://web.archive.org";

/// One capture row from the CDX API.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub timestamp: String,
    pub statuscode: String,
    pub mimetype: String,
    pub length: String,
}

/// One stored copy of a video's archived media (the CDX index of the internal
/// fake-url media path). The archive can hold several stores of the same
/// video at different times/sizes; the largest is usually highest quality.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaCapture {
    pub timestamp: String,
    pub mimetype: String,
    pub length: u64,
}

/// HTTP wrapper around archive.org with politeness baked in.
pub struct Wayback {
    agent: Agent,
    /// Max retries for transient failures (transport errors / 5xx / throttling).
    retries: u32,
    /// Initial backoff before the first retry.
    base_delay: Duration,
    pub verbose: bool,
}

impl Wayback {
    pub fn new(retries: u32, base_delay_ms: u64) -> Self {
        let agent: Agent = Config::builder()
            .user_agent(USER_AGENT)
            .timeout_global(Some(Duration::from_secs(120)))
            .timeout_per_call(Some(Duration::from_secs(60)))
            .max_redirects(5)
            .build()
            .into();
        Self {
            agent,
            retries,
            base_delay: Duration::from_millis(base_delay_ms),
            verbose: false,
        }
    }

    pub fn agent(&self) -> &Agent {
        &self.agent
    }

    pub fn log(&self, msg: impl AsRef<str>) {
        if self.verbose {
            eprintln!("[ytb] {}", msg.as_ref());
        }
    }

    /// Run a closure that produces a `Result<Response>`; retry transient
    /// failures (network errors and HTTP 5xx/429) with exponential backoff.
    /// Terminal HTTP errors (4xx) surface immediately as `HTTP <code>`.
    fn with_retries<F>(&self, mut f: F) -> Result<ureq::http::Response<ureq::Body>>
    where
        F: FnMut() -> std::result::Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    {
        let mut delay = self.base_delay;
        let mut last_err: Option<anyhow::Error> = None;
        for attempt in 0..=self.retries {
            match f() {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if (500..=599).contains(&status) || status == 429 {
                        last_err = Some(anyhow::anyhow!("HTTP {status}"));
                        self.log(format!("received HTTP {status}, retrying ({attempt})"));
                    } else {
                        return Ok(resp);
                    }
                }
                Err(ureq::Error::StatusCode(status))
                    if (500..=599).contains(&status) || status == 429 =>
                {
                    last_err = Some(anyhow::anyhow!("HTTP {status}"));
                    self.log(format!("received HTTP {status}, retrying ({attempt})"));
                }
                Err(ureq::Error::StatusCode(status)) => {
                    return Err(anyhow::anyhow!("HTTP {status}"))
                }
                Err(e) => {
                    let msg = format!("network error: {e}");
                    last_err = Some(anyhow::anyhow!("{msg}"));
                    self.log(format!("{msg}, retrying ({attempt})"));
                }
            }
            if attempt < self.retries {
                std::thread::sleep(delay);
                delay *= 2;
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("request failed")))
    }

    /// `?`-compatible builder for an archive replay URL. The replay flag
    /// (`oe_` raw media, `if_` raw page) must follow the snapshot token
    /// directly; a slash between them yields HTTP 404.
    fn replay_url(date: Option<&str>, flag: &str, rest: &str) -> String {
        // Magic timestamp `2` = the archive's default (closest) capture.
        format!("{BASE}/web/{}{flag}/{rest}", date.unwrap_or("2"))
    }

    fn content_type(resp: &ureq::http::Response<ureq::Body>) -> String {
        resp.headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase()
    }

    /// Resolve the archived media stream URL for a video.
    ///
    /// The Wayback Machine keeps an internal index of captured media under the
    /// fake host `wayback-fakeurl.archive.org/yt/<id>`. Asking web.archive.org
    /// for it (`oe_` + original encoding) redirects to the stored
    /// `videoplayback` stream, which supports HTTP Range requests.
    pub fn locate_stream(&self, id: &str, date: Option<&str>) -> Result<String> {
        match self.resolve_stream(id, date) {
            Ok(url) => Ok(url),
            // A pinned snapshot may hold only the page, not the media. Fall
            // back to the closest capture instead of failing the whole run.
            Err(first) if date.is_some() => {
                self.log(format!(
                    "snapshot {} holds no media for {id}; falling back to the closest capture",
                    date.unwrap_or_default()
                ));
                self.resolve_stream(id, None).map_err(|_| first)
            }
            Err(e) => Err(e),
        }
    }

    fn resolve_stream(&self, id: &str, date: Option<&str>) -> Result<String> {
        let url = Self::replay_url(
            date,
            "oe_",
            &format!("http://wayback-fakeurl.archive.org/yt/{id}"),
        );
        self.log(format!("locating archived stream: {url}"));

        // Lease the connection with a 0-byte range; all we need is the
        // redirect chain's final URL, which `get_uri()` reports.
        let resp = match self.with_retries(|| {
            self.agent
                .get(&url)
                .header("Accept", "video/*")
                .header("Range", "bytes=0-0")
                .call()
        }) {
            Ok(r) => r,
            Err(e) => {
                let msg = e.to_string();
                if msg.starts_with("HTTP 4") {
                    bail!(
                        "no archive media for video {id} at snapshot `{}` \
                         (the page may be captured but the media lost), try \
                         `--list-snapshots` or leave `--date` unset to use the closest capture",
                        date.unwrap_or("2")
                    );
                }
                return Err(e)
                    .with_context(|| format!("could not reach the archive for video {id}"));
            }
        };

        let stream_url = resp.get_uri().to_string();
        let ctype = Self::content_type(&resp);
        if ctype.contains("html") || ctype.starts_with("text/") || ctype.is_empty() {
            bail!("no archived media stream found for video {id} (archive has the page, not the media)");
        }
        self.log(format!("archived stream: {stream_url} [{ctype}]"));
        Ok(stream_url)
    }

    /// List every stored copy of a video's archived media together with its
    /// size, via the CDX index of the internal fake-url path. The archive also
    /// keeps a tiny `application/json` index stub under the same key — that is
    /// filtered out so only real media remains.
    ///
    /// The index occasionally answers with an empty `[]` while the backend is
    /// under load, so an empty result is retried with backoff before giving up.
    pub fn media_captures(&self, id: &str) -> Result<Vec<MediaCapture>> {
        let url = format!("wayback-fakeurl.archive.org/yt/{id}");
        let mut delay = self.base_delay;
        let mut attempts = 0;
        loop {
            let resp = self
                .with_retries(|| {
                    self.agent
                        .get(&format!("{BASE}/cdx/search/cdx"))
                        .query("url", &url)
                        .query("output", "json")
                        .query("fl", "timestamp,original,statuscode,mimetype,digest,length")
                        .query("filter", "statuscode:200")
                        .query("collapse", "digest")
                        .query("limit", "500")
                        .call()
                })
                .with_context(|| format!("CDX query failed for media of {id}"))?;
            let mut cdx = String::new();
            resp.into_body()
                .into_reader()
                .read_to_string(&mut cdx)
                .context("reading CDX response")?;
            let mut out: Vec<MediaCapture> = Vec::new();
            for (timestamp, _status, mimetype, length) in parse_cdx(&cdx) {
                if !mimetype.contains("video") {
                    continue;
                }
                if let Ok(length) = length.parse::<u64>() {
                    out.push(MediaCapture {
                        timestamp,
                        mimetype,
                        length,
                    });
                }
            }
            out.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
            if !out.is_empty() || attempts >= self.retries {
                return Ok(out);
            }
            attempts += 1;
            self.log(format!(
                "media index returned no rows for {id}, retrying ({attempts}); body: {:?}",
                &cdx[..cdx.len().min(140)]
            ));
            std::thread::sleep(delay);
            delay *= 2;
        }
    }

    /// Choose which stored media copy to download.
    ///
    /// The default is the **largest** store (highest quality). With a size cap
    /// the largest store still under the cap wins; if none fits, the smallest
    /// is returned so the caller can warn and still download *something*.
    pub fn select_capture(caps: &[MediaCapture], max_size: Option<u64>) -> Option<&MediaCapture> {
        if caps.is_empty() {
            return None;
        }
        match max_size {
            None => caps.iter().max_by_key(|c| c.length),
            Some(limit) => caps
                .iter()
                .filter(|c| c.length <= limit)
                .max_by_key(|c| c.length)
                .or_else(|| caps.iter().min_by_key(|c| c.length)),
        }
    }

    /// The snapshot timestamp embedded in a resolved stream URL, if any.
    pub fn stream_timestamp(url: &str) -> Option<String> {
        let re = regex::Regex::new(r"/web/(\d{14})").ok()?;
        re.captures(url).map(|c| c[1].to_string())
    }

    /// Verify a raw archived media URL is reachable and really serves media
    /// (not an HTML error page). Returns the final URL after redirects plus
    /// the served content type.
    pub fn probe(&self, url: &str) -> Result<(String, String)> {
        self.log(format!("probing archived media: {url}"));
        let resp = self
            .with_retries(|| {
                self.agent
                    .get(url)
                    .header("Accept", "video/*")
                    .header("Range", "bytes=0-0")
                    .call()
            })
            .context("could not reach archived media")?;
        let final_url = resp.get_uri().to_string();
        let ctype = Self::content_type(&resp);
        if ctype.contains("html") || ctype.starts_with("text/") || ctype.is_empty() {
            bail!(
                "no archived media stream at {url} (the archive holds only a page for this link)"
            );
        }
        Ok((final_url, ctype))
    }

    /// Force a snapshot timestamp into an archived URL, keeping any replay
    /// flag (`oe_`, `id_`, `if_`) that followed the old timestamp.
    pub fn pin_date(url: &str, date: &str) -> Result<String> {
        let re =
            regex::Regex::new(r"(web\.archive\.org|archive\.org)/web/([0-9]+)([0-9A-Za-z_*]*)")
                .unwrap();
        if re.is_match(url) {
            Ok(re
                .replace(url, |caps: &regex::Captures<'_>| {
                    format!("{}/web/{}{}", &caps[1], date, &caps[3])
                })
                .into_owned())
        } else {
            bail!("cannot pin a snapshot date on {url:?}: not a web.archive.org URL")
        }
    }

    /// Fetch the raw (`if_`) archived watch page. Returns `None` when the
    /// snapshot holds no watch page we can parse (metadata is optional).
    /// When the user linked a specific inner URL, it is replayed as-is so live
    /// broadcasts (`/live/<id>`) keep their `ytInitialPlayerResponse`.
    pub fn fetch_archived_page(
        &self,
        id: &str,
        date: Option<&str>,
        prefer_inner: Option<&str>,
    ) -> Result<Option<String>> {
        let watch_url = match prefer_inner {
            Some(inner) => inner.to_string(),
            None => format!("https://www.youtube.com/watch?v={id}"),
        };
        let url = Self::replay_url(date, "if_", &watch_url);
        self.log(format!("fetching archived watch page: {url}"));

        let resp = match self.with_retries(|| {
            self.agent
                .get(&url)
                .header("Accept", "text/html,*/*")
                .call()
        }) {
            Ok(r) => r,
            Err(e) => {
                self.log(format!(
                    "watch page unavailable, continuing without metadata: {e:#}"
                ));
                return Ok(None);
            }
        };
        if Self::content_type(&resp).contains("html") {
            let mut body = String::new();
            let mut reader = resp.into_body().into_reader().take(4 * 1024 * 1024);
            reader
                .read_to_string(&mut body)
                .context("truncated archive response")?;
            Ok(Some(body))
        } else {
            Ok(None)
        }
    }

    /// Query the CDX index for every archived capture of a video's watch page.
    pub fn list_snapshots(&self, id: &str) -> Result<Vec<Snapshot>> {
        let mut seen: std::collections::HashSet<String> = Default::default();
        let mut out = Vec::new();
        // The archive crawled both the bare and www hosts over the years, and
        // live broadcasts are caught under `/live/<id>` rather than `/watch?v=`.
        for url in [
            format!("www.youtube.com/watch?v={id}"),
            format!("youtube.com/watch?v={id}"),
            format!("www.youtube.com/live/{id}"),
            format!("youtube.com/live/{id}"),
        ] {
            let resp = self
                .with_retries(|| {
                    self.agent
                        .get(&format!("{BASE}/cdx/search/cdx"))
                        .query("url", &url)
                        .query("output", "json")
                        .query("fl", "timestamp,original,statuscode,mimetype,digest,length")
                        .query("filter", "statuscode:200")
                        .query("collapse", "digest")
                        .query("limit", "500")
                        .call()
                })
                .with_context(|| format!("CDX query failed for {url}"))?;
            let mut cdx = String::new();
            resp.into_body()
                .into_reader()
                .read_to_string(&mut cdx)
                .context("reading CDX response")?;
            for (timestamp, statuscode, mimetype, length) in parse_cdx(&cdx) {
                if seen.insert(timestamp.clone()) {
                    out.push(Snapshot {
                        timestamp,
                        statuscode,
                        mimetype,
                        length,
                    });
                }
            }
        }
        out.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        Ok(out)
    }
}

/// Parse the CDX JSON array-of-arrays (first row is the field header).
fn parse_cdx(cdx: &str) -> Vec<(String, String, String, String)> {
    let Ok(json) = serde_json::from_str::<Vec<Vec<String>>>(cdx) else {
        return Vec::new();
    };
    let mut rows = json.into_iter();
    let _header = rows.next(); // ["timestamp","original","statuscode",...]
    rows.filter_map(|r| {
        if r.len() < 6 {
            return None;
        }
        Some((r[0].clone(), r[2].clone(), r[3].clone(), r[5].clone()))
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cdx_parse() {
        let raw = r#"[["timestamp","original","statuscode","mimetype","digest","length"],
["20241125121251","http://www.youtube.com/watch?v=C1hjsVLcGFc","200","text/html","ABCDEF","1234"],
["20111027231107","http://www.youtube.com/watch?v=C1hjsVLcGFc","200","text/html","ABCDEF","1234"]]"#;
        let rows = parse_cdx(raw);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "20241125121251");
        assert_eq!(rows[1].1, "200");
        assert_eq!(rows[1].2, "text/html");
        assert_eq!(rows[1].3, "1234");
    }

    #[test]
    fn cdx_parse_garbage() {
        assert!(parse_cdx("not json").is_empty());
        assert!(parse_cdx("[]").is_empty());
    }

    #[test]
    fn replay_url_flags() {
        assert_eq!(
            Wayback::replay_url(None, "oe_", "http://wayback-fakeurl.archive.org/yt/ABC"),
            "https://web.archive.org/web/2oe_/http://wayback-fakeurl.archive.org/yt/ABC"
        );
        assert_eq!(
            Wayback::replay_url(
                Some("20241125121251"),
                "if_",
                "https://www.youtube.com/watch?v=x"
            ),
            "https://web.archive.org/web/20241125121251if_/https://www.youtube.com/watch?v=x"
        );
        assert_eq!(
            Wayback::replay_url(Some("20241125121251"), "oe_", "http://a/yt/x"),
            "https://web.archive.org/web/20241125121251oe_/http://a/yt/x"
        );
    }

    fn cap(ts: &str, len: u64) -> MediaCapture {
        MediaCapture {
            timestamp: ts.to_string(),
            mimetype: "video/webm".into(),
            length: len,
        }
    }

    #[test]
    fn select_capture_default_is_largest() {
        let caps = vec![cap("1", 100), cap("2", 5_000), cap("3", 300)];
        let got = Wayback::select_capture(&caps, None).unwrap();
        assert_eq!(got.timestamp, "2");
        assert_eq!(got.length, 5_000);
    }

    #[test]
    fn select_capture_respects_cap() {
        let caps = vec![cap("1", 100), cap("2", 5_000), cap("3", 300)];
        let got = Wayback::select_capture(&caps, Some(400)).unwrap();
        assert_eq!(got.timestamp, "3");
        assert_eq!(got.length, 300);
    }

    #[test]
    fn select_capture_falls_back_to_smallest_when_none_fits() {
        let caps = vec![cap("1", 100), cap("2", 5_000), cap("3", 300)];
        let got = Wayback::select_capture(&caps, Some(10)).unwrap();
        assert_eq!(got.timestamp, "1");
        assert_eq!(got.length, 100);
    }

    #[test]
    fn select_capture_empty() {
        assert_eq!(Wayback::select_capture(&[], None), None);
    }

    #[test]
    fn stream_timestamp_from_url() {
        assert_eq!(
            Wayback::stream_timestamp(
                "https://web.archive.org/web/20111027231107oe_/http://o-o.preferred..../videoplayback"
            ),
            Some("20111027231107".into())
        );
        assert_eq!(
            Wayback::stream_timestamp("https://web.archive.org/web/2oe_/http://a/yt/x"),
            None
        );
    }

    #[test]
    fn pin_date_replaces_snapshot_token() {
        assert_eq!(
            Wayback::pin_date(
                "https://web.archive.org/web/20111027231107oe_/http://a/videoplayback?itag=5",
                "20250228024155"
            )
            .unwrap(),
            "https://web.archive.org/web/20250228024155oe_/http://a/videoplayback?itag=5"
        );
        assert_eq!(
            Wayback::pin_date(
                "https://web.archive.org/web/2oe_/http://a/videoplayback",
                "20241125121251"
            )
            .unwrap(),
            "https://web.archive.org/web/20241125121251oe_/http://a/videoplayback"
        );
        assert_eq!(
            Wayback::pin_date(
                "https://web.archive.org/web/20230520013354/https://www.youtube.com/live/S2dvG697FQo",
                "20241125121251"
            )
            .unwrap(),
            "https://web.archive.org/web/20241125121251/https://www.youtube.com/live/S2dvG697FQo"
        );
        assert!(Wayback::pin_date("https://youtu.be/abc", "20240101000000").is_err());
    }
}
