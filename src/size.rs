//! Human-friendly byte-size parsing for `--max-size` (e.g. `150M`, `1.5G`,
//! `500MiB`). `K`/`KiB`/`M`/`MiB`/... are powers of 1024; `KB`/`MB`/`GB` are
//! powers of 1000. A bare number is bytes.

use anyhow::{anyhow, bail, Result};

/// Parse a human-readable byte size into a byte count.
pub fn parse_size(input: &str) -> Result<u64> {
    let re = regex::Regex::new(
        r"(?i)^\s*(\d+(?:\.\d+)?)\s*(b|kb|mb|gb|tb|k|kib|m|mib|g|gib|t|tib)?\s*$",
    )
    .unwrap();
    let caps = re.captures(input.trim()).ok_or_else(|| {
        anyhow!("invalid size {input:?}; expected e.g. 150M, 1.5G, 500MiB, or 104857600")
    })?;
    let num: f64 = caps[1].parse()?;
    let unit = caps.get(2).map(|m| m.as_str().to_ascii_lowercase());
    let mult: f64 = match unit.as_deref() {
        None | Some("b") => 1.0,
        Some("k") | Some("kib") => 1024.0,
        Some("m") | Some("mib") => 1024.0 * 1024.0,
        Some("g") | Some("gib") => 1024.0 * 1024.0 * 1024.0,
        Some("t") | Some("tib") => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        Some("kb") => 1000.0,
        Some("mb") => 1000.0 * 1000.0,
        Some("gb") => 1000.0 * 1000.0 * 1000.0,
        Some("tb") => 1000.0 * 1000.0 * 1000.0 * 1000.0,
        _ => unreachable!(),
    };
    let bytes = num * mult;
    if bytes > u64::MAX as f64 {
        bail!("size {input:?} too large");
    }
    Ok(bytes as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_bytes() {
        assert_eq!(parse_size("0").unwrap(), 0);
        assert_eq!(parse_size("1024").unwrap(), 1024);
        assert_eq!(parse_size("104857600").unwrap(), 104857600);
    }

    #[test]
    fn powers_of_1024() {
        assert_eq!(parse_size("1K").unwrap(), 1024);
        assert_eq!(parse_size("1KiB").unwrap(), 1024);
        assert_eq!(parse_size("1M").unwrap(), 1024 * 1024);
        assert_eq!(parse_size("150M").unwrap(), 150 * 1024 * 1024);
        assert_eq!(parse_size("1.5G").unwrap(), 1536 * 1024 * 1024);
        assert_eq!(parse_size("500MiB").unwrap(), 500 * 1024 * 1024);
    }

    #[test]
    fn powers_of_1000() {
        assert_eq!(parse_size("1KB").unwrap(), 1000);
        assert_eq!(parse_size("2MB").unwrap(), 2_000_000);
        assert_eq!(parse_size("1GB").unwrap(), 1_000_000_000);
    }

    #[test]
    fn whitespace_and_case() {
        assert_eq!(parse_size("  1 M  ").unwrap(), 1024 * 1024);
        assert_eq!(parse_size("2MiB").unwrap(), parse_size("2mib").unwrap());
    }

    #[test]
    fn invalid() {
        assert!(parse_size("").is_err());
        assert!(parse_size("abc").is_err());
        assert!(parse_size("10X").is_err());
        assert!(parse_size("-1").is_err());
    }
}
