use anyhow::{anyhow, Context, Result};
use httpdate::parse_http_date;
use std::time::UNIX_EPOCH;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::GRANULARITY_SECONDS;

pub fn now_utc() -> OffsetDateTime {
    OffsetDateTime::now_utc()
}

pub fn format_rfc3339_epoch(epoch_seconds: i64) -> Result<String> {
    OffsetDateTime::from_unix_timestamp(epoch_seconds)
        .with_context(|| format!("invalid epoch seconds: {epoch_seconds}"))?
        .format(&Rfc3339)
        .with_context(|| "failed to format RFC3339 timestamp")
}

pub fn parse_http_date_epoch(value: &str) -> Result<i64> {
    let parsed = parse_http_date(value).with_context(|| format!("invalid HTTP Date: {value}"))?;
    parsed
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|err| anyhow!("HTTP Date predates Unix epoch: {err}"))
}

pub fn last_completed_bucket(now: OffsetDateTime) -> (i64, i64) {
    let end = (now.unix_timestamp() / GRANULARITY_SECONDS) * GRANULARITY_SECONDS;
    (end - GRANULARITY_SECONDS, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_completed_bucket_uses_exact_boundary_as_end() {
        let now = OffsetDateTime::from_unix_timestamp(1776772800).unwrap();
        assert_eq!(last_completed_bucket(now), (1776772500, 1776772800));
    }

    #[test]
    fn last_completed_bucket_inside_interval() {
        let now = OffsetDateTime::from_unix_timestamp(1776772921).unwrap();
        assert_eq!(last_completed_bucket(now), (1776772500, 1776772800));
    }
}
