use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use coinbase_candle_prover::attestation::{MockAttester, RealNsmAttester};
use coinbase_candle_prover::proof::{ProofBundle, ProofRequest};
use coinbase_candle_prover::prover::{validate_request, Prover};
use coinbase_candle_prover::timeutil::{last_completed_bucket, now_utc};
use coinbase_candle_prover::{
    DEFAULT_HTTP_LISTEN, DEFAULT_NITRIDING_INTERNAL_URL, GRANULARITY_LABEL, PRODUCT_ID,
};
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

const PROOF_PATH_PREFIX: &str = "/proof/v1/products/";
const PROOF_PATH_SUFFIX: &str = "/candles";
const MAX_REQUEST_BYTES: usize = 16 * 1024;
const NITRIDING_READY_RETRIES: usize = 60;
const NITRIDING_READY_RETRY_DELAY: Duration = Duration::from_millis(250);

#[derive(Debug, Parser)]
#[command(about = "Run the Nitro Enclave Coinbase candle prover")]
struct Args {
    #[arg(
        long,
        help = "Produce one proof and exit instead of serving HTTP requests"
    )]
    once: bool,

    #[arg(long, alias = "bucket-start")]
    start: Option<i64>,

    #[arg(long, alias = "bucket-end")]
    end: Option<i64>,

    #[arg(long)]
    output: Option<PathBuf>,

    #[arg(long, help = "Use the explicit local mock attestation format")]
    mock_attestation: bool,

    #[arg(long, help = "Serve the public proof HTTP API")]
    serve_http: bool,

    #[arg(long, default_value = DEFAULT_HTTP_LISTEN)]
    http_listen: String,

    #[arg(long, default_value = DEFAULT_NITRIDING_INTERNAL_URL)]
    nitriding_internal_url: String,

    #[arg(long, help = "Skip /enclave/ready call; useful for local HTTP tests")]
    skip_nitriding_ready: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.once {
        return run_once(args);
    }
    if args.serve_http {
        return run_http_server(args);
    }

    Err(anyhow!(
        "use --serve-http for nitriding mode or --once for one-shot proofs"
    ))
}

fn run_once(args: Args) -> Result<()> {
    let request = request_from_args(args.start, args.end)?;
    let bundle = prove(request, args.mock_attestation)?;
    write_bundle(args.output, &bundle)
}

fn run_http_server(args: Args) -> Result<()> {
    let listener = TcpListener::bind(&args.http_listen)
        .with_context(|| format!("failed to bind HTTP listener {}", args.http_listen))?;

    if !args.skip_nitriding_ready {
        signal_nitriding_ready(&args.nitriding_internal_url)
            .with_context(|| "failed to signal readiness to nitriding")?;
    }
    eprintln!("enclave-prover listening on {}", args.http_listen);

    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let mock_attestation = args.mock_attestation;
                thread::spawn(move || {
                    if let Err(err) = handle_connection(stream, mock_attestation) {
                        eprintln!("HTTP connection failed: {err:#}");
                    }
                });
            }
            Err(err) => eprintln!("HTTP accept failed: {err}"),
        }
    }

    Ok(())
}

fn handle_connection(mut stream: TcpStream, mock_attestation: bool) -> Result<()> {
    let request = read_http_request(&mut stream)?;
    let proof_request = match parse_public_proof_request(&request.method, &request.target) {
        Ok(request) => request,
        Err(response) => {
            write_json_response(&mut stream, response.status, &response.body)?;
            return Ok(());
        }
    };

    let result = prove(proof_request, mock_attestation);
    match result {
        Ok(bundle) => write_json_response(&mut stream, 200, &bundle)?,
        Err(err) => {
            let status = error_status(&err);
            let body = serde_json::json!({ "error": err.to_string() });
            write_json_response(&mut stream, status, &body)?;
        }
    }
    Ok(())
}

fn prove(request: ProofRequest, mock_attestation: bool) -> Result<ProofBundle> {
    validate_request(&request)?;

    if mock_attestation {
        let attester = MockAttester::new(now_utc().unix_timestamp(), mock_pcrs());
        let mut prover = Prover::new(attester);
        return prover.prove(request);
    }

    let attester = RealNsmAttester::open()?;
    let mut prover = Prover::new(attester);
    prover.prove(request)
}

fn error_status(err: &anyhow::Error) -> u16 {
    let text = err.to_string();
    if text.contains("Coinbase")
        || text.contains("HTTPS")
        || text.contains("TLS")
        || text.contains("api.coinbase.com")
        || text.contains("HTTP body")
    {
        502
    } else {
        500
    }
}

#[derive(Debug)]
struct HttpRequest {
    method: String,
    target: String,
}

#[derive(Debug)]
struct ErrorResponse {
    status: u16,
    body: serde_json::Value,
}

fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest> {
    let mut bytes = Vec::new();
    let mut buf = [0u8; 1024];

    loop {
        let read = stream
            .read(&mut buf)
            .with_context(|| "failed to read HTTP request")?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..read]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if bytes.len() > MAX_REQUEST_BYTES {
            bail!("HTTP request exceeded {MAX_REQUEST_BYTES} bytes");
        }
    }

    let text = std::str::from_utf8(&bytes).with_context(|| "HTTP request was not UTF-8")?;
    let request_line = text
        .lines()
        .next()
        .ok_or_else(|| anyhow!("HTTP request omitted request line"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| anyhow!("HTTP request omitted method"))?;
    let target = parts
        .next()
        .ok_or_else(|| anyhow!("HTTP request omitted target"))?;
    let version = parts
        .next()
        .ok_or_else(|| anyhow!("HTTP request omitted version"))?;
    if !version.starts_with("HTTP/") {
        bail!("HTTP request had invalid version: {version}");
    }

    Ok(HttpRequest {
        method: method.to_string(),
        target: target.to_string(),
    })
}

fn parse_public_proof_request(method: &str, target: &str) -> Result<ProofRequest, ErrorResponse> {
    if method != "GET" {
        return Err(json_error(405, "method not allowed"));
    }

    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let product = match path
        .strip_prefix(PROOF_PATH_PREFIX)
        .and_then(|rest| rest.strip_suffix(PROOF_PATH_SUFFIX))
    {
        Some(product) => product,
        None => return Err(json_error(404, "not found")),
    };
    if product != PRODUCT_ID {
        return Err(json_error(400, "unsupported product_id"));
    }

    let params = match parse_query(query) {
        Ok(params) => params,
        Err(err) => return Err(json_error(400, &err.to_string())),
    };
    for key in params.keys() {
        if !matches!(key.as_str(), "start" | "end" | "granularity" | "limit") {
            return Err(json_error(400, "unsupported query parameter"));
        }
    }
    let start = match required_i64(&params, "start") {
        Ok(value) => value,
        Err(err) => return Err(json_error(400, &err.to_string())),
    };
    let end = match required_i64(&params, "end") {
        Ok(value) => value,
        Err(err) => return Err(json_error(400, &err.to_string())),
    };
    if params.get("granularity").map(String::as_str) != Some(GRANULARITY_LABEL) {
        return Err(json_error(400, "granularity must be FIVE_MINUTE"));
    }
    if params.get("limit").map(String::as_str) != Some("1") {
        return Err(json_error(400, "limit must be 1"));
    }

    let request = ProofRequest { start, end };
    if let Err(err) = validate_request(&request) {
        return Err(json_error(400, &err.to_string()));
    }
    Ok(request)
}

fn parse_query(query: &str) -> Result<HashMap<String, String>> {
    let mut out = HashMap::new();
    if query.is_empty() {
        return Ok(out);
    }

    for pair in query.split('&') {
        let (key, value) = pair
            .split_once('=')
            .ok_or_else(|| anyhow!("query parameter omitted '='"))?;
        let key = urlencoding::decode(key)
            .with_context(|| "query parameter key was not valid percent-encoding")?
            .into_owned();
        let value = urlencoding::decode(value)
            .with_context(|| "query parameter value was not valid percent-encoding")?
            .into_owned();
        if out.insert(key.clone(), value).is_some() {
            bail!("duplicate query parameter: {key}");
        }
    }
    Ok(out)
}

fn required_i64(params: &HashMap<String, String>, name: &str) -> Result<i64> {
    params
        .get(name)
        .ok_or_else(|| anyhow!("missing {name}"))?
        .parse::<i64>()
        .with_context(|| format!("{name} must be an integer"))
}

fn json_error(status: u16, message: &str) -> ErrorResponse {
    ErrorResponse {
        status,
        body: serde_json::json!({ "error": message }),
    }
}

fn write_json_response<T: serde::Serialize>(
    stream: &mut TcpStream,
    status: u16,
    body: &T,
) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(body).with_context(|| "failed to serialize response")?;
    let header = format!(
        "HTTP/1.1 {status} {}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n",
        reason_phrase(status),
        bytes.len()
    );
    stream
        .write_all(header.as_bytes())
        .with_context(|| "failed to write HTTP response header")?;
    stream
        .write_all(&bytes)
        .with_context(|| "failed to write HTTP response body")?;
    let _ = stream.shutdown(Shutdown::Both);
    Ok(())
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        _ => "OK",
    }
}

fn signal_nitriding_ready(base_url: &str) -> Result<()> {
    let base_url = base_url.trim_end_matches('/');
    let url = format!("{base_url}/enclave/ready");
    let mut last_error = None;

    for attempt in 1..=NITRIDING_READY_RETRIES {
        match send_local_http("GET", &url, None) {
            Ok(()) => return Ok(()),
            Err(err) if attempt < NITRIDING_READY_RETRIES => {
                last_error = Some(err);
                thread::sleep(NITRIDING_READY_RETRY_DELAY);
            }
            Err(err) => last_error = Some(err),
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("nitriding readiness signal failed")))
        .with_context(|| format!("nitriding did not become ready at {url}"))
}

fn send_local_http(method: &str, url: &str, body: Option<&[u8]>) -> Result<()> {
    let stripped = url
        .strip_prefix("http://")
        .ok_or_else(|| anyhow!("only http:// nitriding internal URLs are supported"))?;
    let (host, path) = stripped.split_once('/').unwrap_or((stripped, ""));
    let path = format!("/{path}");
    let mut stream = TcpStream::connect(host)
        .with_context(|| format!("failed to connect to nitriding internal server {host}"))?;
    let body = body.unwrap_or(&[]);
    let request = format!(
        "{method} {path} HTTP/1.1\r\n\
         Host: {host}\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .with_context(|| "failed to write nitriding request header")?;
    stream
        .write_all(body)
        .with_context(|| "failed to write nitriding request body")?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .with_context(|| "failed to read nitriding response")?;
    let status = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| anyhow!("nitriding returned an invalid HTTP response"))?;
    if status != 200 {
        bail!("nitriding returned HTTP status {status}");
    }
    Ok(())
}

fn request_from_args(start: Option<i64>, end: Option<i64>) -> Result<ProofRequest> {
    match (start, end) {
        (Some(start), Some(end)) => Ok(ProofRequest { start, end }),
        (None, None) => {
            let (start, end) = last_completed_bucket(now_utc());
            Ok(ProofRequest { start, end })
        }
        _ => Err(anyhow!("--start and --end must be provided together")),
    }
}

fn write_bundle(output: Option<PathBuf>, bundle: &ProofBundle) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(bundle).with_context(|| "failed to serialize bundle")?;
    if let Some(output) = output {
        std::fs::write(&output, bytes)
            .with_context(|| format!("failed to write {}", output.display()))?;
    } else {
        std::io::stdout()
            .write_all(&bytes)
            .with_context(|| "failed to write bundle to stdout")?;
        println!();
    }
    Ok(())
}

fn mock_pcrs() -> BTreeMap<u16, Vec<u8>> {
    [(0, vec![1u8; 48]), (1, vec![2u8; 48]), (2, vec![3u8; 48])]
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_public_proof_request() {
        let request = parse_public_proof_request(
            "GET",
            "/proof/v1/products/BTC-USD/candles?start=1713718800&end=1713718800&granularity=FIVE_MINUTE&limit=1",
        )
        .unwrap();
        assert_eq!(
            request,
            ProofRequest {
                start: 1713718800,
                end: 1713718800
            }
        );
    }

    #[test]
    fn rejects_bad_product() {
        let err = parse_public_proof_request(
            "GET",
            "/proof/v1/products/ETH-USD/candles?start=1&end=1&granularity=FIVE_MINUTE&limit=1",
        )
        .unwrap_err();
        assert_eq!(err.status, 400);
    }

    #[test]
    fn rejects_bad_granularity() {
        let err = parse_public_proof_request(
            "GET",
            "/proof/v1/products/BTC-USD/candles?start=1&end=1&granularity=ONE_MINUTE&limit=1",
        )
        .unwrap_err();
        assert_eq!(err.status, 400);
    }

    #[test]
    fn rejects_missing_start() {
        let err = parse_public_proof_request(
            "GET",
            "/proof/v1/products/BTC-USD/candles?end=1&granularity=FIVE_MINUTE&limit=1",
        )
        .unwrap_err();
        assert_eq!(err.status, 400);
    }

    #[test]
    fn rejects_wide_request_range() {
        let err = parse_public_proof_request(
            "GET",
            "/proof/v1/products/BTC-USD/candles?start=1713718800&end=1713719100&granularity=FIVE_MINUTE&limit=1",
        )
        .unwrap_err();
        assert_eq!(err.status, 400);
    }

    #[test]
    fn rejects_unsupported_method() {
        let err = parse_public_proof_request(
            "POST",
            "/proof/v1/products/BTC-USD/candles?start=1&end=1&granularity=FIVE_MINUTE&limit=1",
        )
        .unwrap_err();
        assert_eq!(err.status, 405);
    }

    #[test]
    fn rejects_unsupported_query_parameter() {
        let err = parse_public_proof_request(
            "GET",
            "/proof/v1/products/BTC-USD/candles?start=1&end=1&granularity=FIVE_MINUTE&limit=1&foo=bar",
        )
        .unwrap_err();
        assert_eq!(err.status, 400);
    }
}
