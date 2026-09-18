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

    /// Fetch the raw (`if_`) archived watch page. Returns `None` when the
    /// snapshot holds no watch page we can parse (metadata is optional).
    pub fn fetch_archived_page(&self, id: &str, date: Option<&str>) -> Result<Option<String>> {
        let watch_url = format!("https://www.youtube.com/watch?v={id}");
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
        // The archive crawled both the bare and www hosts over the years.
        for host in ["www.youtube.com", "youtube.com"] {
            let url = format!("{host}/watch?v={id}");
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
                .with_context(|| format!("CDX query failed for {host}"))?;
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
}
