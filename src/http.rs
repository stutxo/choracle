use anyhow::{anyhow, Context, Result};
use httparse::{Header, Response, EMPTY_HEADER};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::Arc;

use crate::crypto::b64_encode;
use crate::proof::TlsInfo;
use crate::{GRANULARITY_LABEL, GRANULARITY_SECONDS, HOST, REQUEST_PATH};

#[derive(Debug, Clone)]
pub struct HttpFetchResult {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body_bytes: Vec<u8>,
    pub body_json: Value,
    pub tls: TlsInfo,
}

pub fn coinbase_candle_query(start: i64, end: i64) -> Result<String> {
    if start < 0 {
        return Err(anyhow!("start must be non-negative"));
    }
    if end < 0 {
        return Err(anyhow!("end must be non-negative"));
    }
    if start > end {
        return Err(anyhow!("start must be less than or equal to end"));
    }
    if start != end {
        return Err(anyhow!("start and end must both equal the candle start"));
    }
    if start % GRANULARITY_SECONDS != 0 {
        return Err(anyhow!("start must be 5-minute aligned"));
    }
    Ok(format!(
        "start={start}&end={end}&granularity={GRANULARITY_LABEL}&limit=1"
    ))
}

pub fn build_coinbase_request(query: &str) -> String {
    format!(
        "GET {REQUEST_PATH}?{query} HTTP/1.1\r\n\
         Host: {HOST}\r\n\
         User-Agent: coinbase-candle-prover/0.1\r\n\
         Accept: application/json\r\n\
         Cache-Control: no-cache\r\n\
         Connection: close\r\n\
         \r\n"
    )
}

pub fn fetch_coinbase_over_tls<S>(stream: S, query: &str) -> Result<HttpFetchResult>
where
    S: Read + Write,
{
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let server_name = rustls::pki_types::ServerName::try_from(HOST.to_string())
        .with_context(|| "invalid TLS server name")?;
    let connection = rustls::ClientConnection::new(Arc::new(config), server_name)
        .with_context(|| "failed to create TLS client connection")?;
    let mut tls_stream = rustls::StreamOwned::new(connection, stream);

    let request = build_coinbase_request(query);
    tls_stream
        .write_all(request.as_bytes())
        .with_context(|| "failed to write HTTPS request")?;
    tls_stream.flush().with_context(|| "failed to flush TLS")?;

    let mut response_bytes = Vec::new();
    tls_stream
        .read_to_end(&mut response_bytes)
        .with_context(|| "failed to read HTTPS response")?;

    let tls = tls_info(&tls_stream)?;
    parse_http_response(&response_bytes, tls)
}

pub fn parse_http_response(response_bytes: &[u8], tls: TlsInfo) -> Result<HttpFetchResult> {
    let header_end = find_header_end(response_bytes)
        .ok_or_else(|| anyhow!("HTTP response did not contain a header/body separator"))?;
    let header_bytes = &response_bytes[..header_end.header_len];
    let raw_body = &response_bytes[header_end.body_start..];

    let mut headers = [EMPTY_HEADER; 64];
    let mut response = Response::new(&mut headers);
    response
        .parse(header_bytes)
        .with_context(|| "failed to parse HTTP response headers")?;

    let status = response
        .code
        .ok_or_else(|| anyhow!("HTTP response did not include a status code"))?;
    let headers = normalize_headers(response.headers);
    let body_bytes = if header_value(&headers, "transfer-encoding")
        .map(|value| value.to_ascii_lowercase().contains("chunked"))
        .unwrap_or(false)
    {
        decode_chunked(raw_body)?
    } else {
        raw_body.to_vec()
    };

    if let Some(content_length) = header_value(&headers, "content-length") {
        let expected = content_length
            .parse::<usize>()
            .with_context(|| format!("invalid Content-Length: {content_length}"))?;
        if body_bytes.len() != expected {
            return Err(anyhow!(
                "body length was {} bytes, expected Content-Length {expected}",
                body_bytes.len()
            ));
        }
    }

    let body_json =
        serde_json::from_slice(&body_bytes).with_context(|| "HTTP body was not valid JSON")?;

    Ok(HttpFetchResult {
        status,
        headers,
        body_bytes,
        body_json,
        tls,
    })
}

pub fn header_value<'a>(headers: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    headers.get(&name.to_ascii_lowercase()).map(String::as_str)
}

fn tls_info<S>(tls_stream: &rustls::StreamOwned<rustls::ClientConnection, S>) -> Result<TlsInfo>
where
    S: Read + Write,
{
    let certs = tls_stream
        .conn
        .peer_certificates()
        .ok_or_else(|| anyhow!("TLS peer did not present certificates"))?;
    if certs.is_empty() {
        return Err(anyhow!("TLS peer certificate chain was empty"));
    }

    Ok(TlsInfo {
        sni: HOST.to_string(),
        cert_chain_der_b64: certs.iter().map(|cert| b64_encode(cert.as_ref())).collect(),
    })
}

#[derive(Debug, Clone, Copy)]
struct HeaderEnd {
    header_len: usize,
    body_start: usize,
}

fn find_header_end(bytes: &[u8]) -> Option<HeaderEnd> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|pos| HeaderEnd {
            header_len: pos + 4,
            body_start: pos + 4,
        })
}

fn normalize_headers(headers: &[Header<'_>]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for header in headers {
        let name = header.name.to_ascii_lowercase();
        let value = String::from_utf8_lossy(header.value).trim().to_string();
        out.insert(name, value);
    }
    out
}

fn decode_chunked(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut pos = 0;
    let mut out = Vec::new();

    loop {
        let line_end = find_crlf(bytes, pos).ok_or_else(|| anyhow!("malformed chunk size"))?;
        let line = std::str::from_utf8(&bytes[pos..line_end])
            .with_context(|| "chunk size was not UTF-8")?;
        let size_hex = line
            .split(';')
            .next()
            .ok_or_else(|| anyhow!("empty chunk size"))?
            .trim();
        let size = usize::from_str_radix(size_hex, 16)
            .with_context(|| format!("invalid chunk size: {size_hex}"))?;
        pos = line_end + 2;

        if size == 0 {
            break;
        }

        if bytes.len() < pos + size + 2 {
            return Err(anyhow!("chunk body exceeded response length"));
        }
        out.extend_from_slice(&bytes[pos..pos + size]);
        pos += size;
        if bytes.get(pos..pos + 2) != Some(b"\r\n") {
            return Err(anyhow!("chunk was not followed by CRLF"));
        }
        pos += 2;
    }

    Ok(out)
}

fn find_crlf(bytes: &[u8], start: usize) -> Option<usize> {
    bytes[start..]
        .windows(2)
        .position(|window| window == b"\r\n")
        .map(|offset| start + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tls() -> TlsInfo {
        TlsInfo {
            sni: HOST.to_string(),
            cert_chain_der_b64: vec!["leaf".to_string()],
        }
    }

    #[test]
    fn builds_expected_query_shape() {
        let query = coinbase_candle_query(1776717600, 1776717600).unwrap();
        assert_eq!(
            query,
            "start=1776717600&end=1776717600&granularity=FIVE_MINUTE&limit=1"
        );
    }

    #[test]
    fn parses_content_length_response() {
        let raw = b"HTTP/1.1 200 OK\r\nDate: Tue, 21 Apr 2026 12:00:00 GMT\r\nContent-Type: application/json\r\nContent-Length: 87\r\n\r\n{\"candles\":[{\"start\":\"1000\",\"low\":\"1\",\"high\":\"2\",\"open\":\"3\",\"close\":\"4\",\"volume\":\"5\"}]}";
        let parsed = parse_http_response(raw, tls()).unwrap();
        assert_eq!(parsed.status, 200);
        assert_eq!(
            header_value(&parsed.headers, "content-type"),
            Some("application/json")
        );
        assert_eq!(
            parsed.body_json,
            serde_json::json!({
                "candles": [
                    {
                        "start": "1000",
                        "low": "1",
                        "high": "2",
                        "open": "3",
                        "close": "4",
                        "volume": "5"
                    }
                ]
            })
        );
    }

    #[test]
    fn decodes_chunked_response() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: application/json\r\n\r\n5\r\n{\"can\r\n8\r\ndles\":[]\r\n1\r\n}\r\n0\r\n\r\n";
        let parsed = parse_http_response(raw, tls()).unwrap();
        assert_eq!(parsed.body_bytes, b"{\"candles\":[]}");
    }
}
