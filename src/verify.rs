use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use time::OffsetDateTime;

use crate::attestation::{AttestationVerifier, VerifiedAttestation};
use crate::candle::contains_selected_candle;
use crate::crypto::{b64_decode, sha256_hex, sha384};
use crate::http::coinbase_candle_query;
use crate::proof::{ProofBundle, ProofPayload, TlsInfo};
use crate::timeutil::parse_http_date_epoch;
use crate::{
    BUNDLE_SCHEMA, DEFAULT_MAX_SKEW_SECONDS, GRANULARITY_LABEL, GRANULARITY_SECONDS, HOST,
    PAYLOAD_SCHEMA, PRODUCT_ID, PROOF_POLICY, REQUEST_PATH, SOURCE,
};
use rustls::client::danger::ServerCertVerifier;

#[derive(Debug, Clone)]
pub struct VerificationConfig {
    pub expected_pcrs: BTreeMap<u16, Vec<u8>>,
    pub required_pcrs: Vec<u16>,
    pub max_skew_seconds: i64,
    pub max_future_skew_seconds: i64,
}

impl VerificationConfig {
    pub fn new(expected_pcrs: BTreeMap<u16, Vec<u8>>) -> Self {
        Self {
            expected_pcrs,
            required_pcrs: vec![0, 1, 2],
            max_skew_seconds: DEFAULT_MAX_SKEW_SECONDS,
            max_future_skew_seconds: DEFAULT_MAX_SKEW_SECONDS,
        }
    }
}

#[derive(Debug, Clone)]
pub struct VerifiedProof {
    pub payload: ProofPayload,
    pub attestation: VerifiedAttestation,
}

pub fn verify_bundle_json<V>(
    bundle_json: &[u8],
    verifier: &V,
    config: &VerificationConfig,
    now: OffsetDateTime,
) -> Result<VerifiedProof>
where
    V: AttestationVerifier,
{
    let bundle: ProofBundle =
        serde_json::from_slice(bundle_json).with_context(|| "failed to parse proof bundle JSON")?;
    verify_bundle(&bundle, verifier, config, now)
}

pub fn verify_bundle<V>(
    bundle: &ProofBundle,
    verifier: &V,
    config: &VerificationConfig,
    now: OffsetDateTime,
) -> Result<VerifiedProof>
where
    V: AttestationVerifier,
{
    if bundle.schema != BUNDLE_SCHEMA {
        bail!("unexpected bundle schema: {}", bundle.schema);
    }

    let payload_bytes = b64_decode(&bundle.payload_json_b64)?;

    let payload: ProofPayload = serde_json::from_slice(&payload_bytes)
        .with_context(|| "failed to parse proof payload JSON")?;
    let body_json = verify_body_integrity(&payload)?;
    verify_payload_policy(&payload, &body_json)?;

    let attestation_doc = b64_decode(&bundle.attestation_doc_b64)?;
    let attestation = verifier.verify(&attestation_doc, now)?;
    verify_attestation_time_bounds(&attestation, config, now)?;
    verify_attestation_binding(&payload_bytes, &attestation, config)?;
    verify_tls_certificate_chain(&payload.tls, attestation.timestamp_unix, HOST)?;
    verify_time_policy(&payload, &attestation, config)?;

    Ok(VerifiedProof {
        payload,
        attestation,
    })
}

fn verify_payload_policy(payload: &ProofPayload, body_json: &Value) -> Result<()> {
    if payload.schema != PAYLOAD_SCHEMA {
        bail!("unexpected payload schema: {}", payload.schema);
    }
    if payload.proof_policy != PROOF_POLICY {
        bail!("unexpected proof_policy: {}", payload.proof_policy);
    }
    if payload.source != SOURCE {
        bail!("unexpected source: {}", payload.source);
    }
    if payload.host != HOST {
        bail!("unexpected host: {}", payload.host);
    }
    if payload.product_id != PRODUCT_ID {
        bail!("unexpected product_id: {}", payload.product_id);
    }
    if payload.granularity != GRANULARITY_LABEL {
        bail!("unexpected granularity: {}", payload.granularity);
    }
    if payload.granularity_seconds != GRANULARITY_SECONDS {
        bail!(
            "unexpected granularity_seconds: {}",
            payload.granularity_seconds
        );
    }
    if payload.request_path != REQUEST_PATH {
        bail!("unexpected request_path: {}", payload.request_path);
    }
    if payload.http_status != 200 {
        bail!("unexpected HTTP status: {}", payload.http_status);
    }
    if !payload
        .content_type
        .to_ascii_lowercase()
        .starts_with("application/json")
    {
        bail!("unexpected Content-Type: {}", payload.content_type);
    }
    if payload.request_start < 0 || payload.request_end < 0 {
        bail!("request range must be non-negative");
    }
    if payload.request_start > payload.request_end {
        bail!("request_start must be less than or equal to request_end");
    }
    if payload.request_start != payload.request_end {
        bail!("request_start and request_end must both equal the candle start");
    }
    if payload.request_start % GRANULARITY_SECONDS != 0 {
        bail!("request_start must be 5-minute aligned");
    }
    let expected_query = coinbase_candle_query(payload.request_start, payload.request_end)?;
    if payload.request_query != expected_query {
        bail!("request_query did not match attested Coinbase v3 query");
    }
    if payload.selected_candle.time % GRANULARITY_SECONDS != 0 {
        bail!("selected candle time was not 5-minute aligned");
    }
    if payload.selected_candle.time != payload.request_start {
        bail!("selected candle time did not equal request_start");
    }
    if !contains_selected_candle(body_json, &payload.selected_candle)? {
        bail!("selected_candle did not match body_b64");
    }
    if payload.tls.sni != HOST {
        bail!("unexpected TLS SNI: {}", payload.tls.sni);
    }
    Ok(())
}

fn verify_tls_certificate_chain(tls: &TlsInfo, timestamp_unix: i64, host: &str) -> Result<()> {
    if tls.cert_chain_der_b64.is_empty() {
        bail!("TLS certificate chain was empty");
    }

    let certs = tls
        .cert_chain_der_b64
        .iter()
        .enumerate()
        .map(|(idx, value)| {
            b64_decode(value)
                .with_context(|| format!("TLS certificate {idx} base64 decode failed"))
                .map(rustls::pki_types::CertificateDer::from)
        })
        .collect::<Result<Vec<_>>>()?;
    let (end_entity, intermediates) = certs
        .split_first()
        .ok_or_else(|| anyhow!("TLS certificate chain was empty"))?;

    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let verifier = rustls::client::WebPkiServerVerifier::builder(Arc::new(roots))
        .build()
        .with_context(|| "failed to build WebPKI server certificate verifier")?;
    let server_name = rustls::pki_types::ServerName::try_from(host.to_string())
        .with_context(|| format!("invalid TLS server name: {host}"))?;
    let timestamp = u64::try_from(timestamp_unix)
        .with_context(|| format!("TLS verification timestamp was negative: {timestamp_unix}"))?;
    let now = rustls::pki_types::UnixTime::since_unix_epoch(Duration::from_secs(timestamp));

    verifier
        .verify_server_cert(end_entity, intermediates, &server_name, &[], now)
        .map(|_| ())
        .map_err(|err| anyhow!("TLS certificate chain validation failed: {err}"))
}

fn verify_body_integrity(payload: &ProofPayload) -> Result<Value> {
    require_hex_digest("body_sha256", &payload.body_sha256, 32)?;
    let body_bytes = b64_decode(&payload.body_b64).with_context(|| "body_b64 decode failed")?;
    if sha256_hex(&body_bytes) != payload.body_sha256 {
        bail!("body_b64 SHA-256 did not match body_sha256");
    }
    serde_json::from_slice(&body_bytes).with_context(|| "body_b64 did not decode to JSON")
}

fn verify_attestation_binding(
    payload_bytes: &[u8],
    attestation: &VerifiedAttestation,
    config: &VerificationConfig,
) -> Result<()> {
    let expected_user_data = sha384(payload_bytes);
    if attestation.user_data != expected_user_data {
        bail!("attestation user_data did not equal SHA384(payload_json_bytes)");
    }
    if attestation.nonce.len() < 16 {
        bail!("attestation nonce was too short");
    }

    for required in &config.required_pcrs {
        let expected = config
            .expected_pcrs
            .get(required)
            .ok_or_else(|| anyhow!("expected PCR{required} was not configured"))?;
        let actual = attestation
            .pcrs
            .get(required)
            .ok_or_else(|| anyhow!("attestation did not include PCR{required}"))?;
        if actual.iter().all(|byte| *byte == 0) {
            bail!("PCR{required} was all zeroes; debug-mode attestations are rejected");
        }
        if actual != expected {
            bail!("PCR{required} did not match expected value");
        }
    }

    Ok(())
}

fn verify_attestation_time_bounds(
    attestation: &VerifiedAttestation,
    config: &VerificationConfig,
    now: OffsetDateTime,
) -> Result<()> {
    let future_skew = attestation.timestamp_unix - now.unix_timestamp();
    if future_skew > config.max_future_skew_seconds {
        bail!(
            "attestation timestamp was too far in the future: {}s > {}s",
            future_skew,
            config.max_future_skew_seconds
        );
    }
    Ok(())
}

fn verify_time_policy(
    payload: &ProofPayload,
    attestation: &VerifiedAttestation,
    config: &VerificationConfig,
) -> Result<()> {
    let http_date_epoch = parse_http_date_epoch(&payload.http_date)?;
    let skew = (attestation.timestamp_unix - http_date_epoch).abs();
    if skew > config.max_skew_seconds {
        bail!(
            "attestation timestamp and Coinbase Date skew exceeded limit: {}s > {}s",
            skew,
            config.max_skew_seconds
        );
    }
    Ok(())
}

fn require_hex_digest(name: &str, value: &str, bytes_len: usize) -> Result<()> {
    let decoded = hex::decode(value).with_context(|| format!("{name} was not hex"))?;
    if decoded.len() != bytes_len {
        bail!("{name} must be {bytes_len} bytes");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attestation::{Attester, MockAttestationVerifier, MockAttester};
    use crate::crypto::{b64_encode, random_nonce};
    use crate::proof::{SelectedCandle, TlsInfo};
    use crate::timeutil::now_utc;
    use crate::{BUNDLE_SCHEMA, MOCK_ATTESTATION_SCHEMA};
    use std::collections::BTreeMap;

    fn pcrs() -> BTreeMap<u16, Vec<u8>> {
        [(0, vec![1u8; 48]), (1, vec![2u8; 48]), (2, vec![3u8; 48])]
            .into_iter()
            .collect()
    }

    const COINBASE_LEAF_DER_B64: &str = "MIIDqDCCA02gAwIBAgIQNKGqmFlTBj4RwJTKXlZS3jAKBggqhkjOPQQDAjA7MQswCQYDVQQGEwJVUzEeMBwGA1UEChMVR29vZ2xlIFRydXN0IFNlcnZpY2VzMQwwCgYDVQQDEwNXRTEwHhcNMjYwMzE5MDM0MzU0WhcNMjYwNjE3MDQ0MzUxWjAXMRUwEwYDVQQDEwxjb2luYmFzZS5jb20wWTATBgcqhkjOPQIBBggqhkjOPQMBBwNCAATUhC24Y2m3HtnNNN9fpyBxHDIw0Sog0Lz/pycdRMp96iLTOtyQzRzXxI7yLIzW3E14C0ZYCgT1syE3TJW6brEBo4ICVTCCAlEwDgYDVR0PAQH/BAQDAgeAMBMGA1UdJQQMMAoGCCsGAQUFBwMBMAwGA1UdEwEB/wQCMAAwHQYDVR0OBBYEFGv7o7xgsfl2VjbFVr1KL3eFpFYDMB8GA1UdIwQYMBaAFJB3kjVnxP+ozKnme9mAeXvMk/k4MF4GCCsGAQUFBwEBBFIwUDAnBggrBgEFBQcwAYYbaHR0cDovL28ucGtpLmdvb2cvcy93ZTEvTktFMCUGCCsGAQUFBzAChhlodHRwOi8vaS5wa2kuZ29vZy93ZTEuY3J0MCcGA1UdEQQgMB6CDGNvaW5iYXNlLmNvbYIOKi5jb2luYmFzZS5jb20wEwYDVR0gBAwwCjAIBgZngQwBAgEwNgYDVR0fBC8wLTAroCmgJ4YlaHR0cDovL2MucGtpLmdvb2cvd2UxL2JkMlY2QUpZVVhnLmNybDCCAQQGCisGAQQB1nkCBAIEgfUEgfIA8AB1AA5XlLzzrqk+MxssmQez95Dfm8I9cTIl3SGpJaxhxU4hAAABnQRoRScAAAQDAEYwRAIgXqfi7tETTQ81+Wf8Bf7Bm96a01F3ZhtEFQZMxjOb/0wCIEVFsWRHZusqCPGoaEfbBXEcamZwir/klXTYmurtXRKoAHcAZBHEbKQS7KeJHKICLgC8q08oB9QeNSer6v7VA8l9zfAAAAGdBGhFDAAABAMASDBGAiEAzDJbjkSRiFH/mhwQaJ/opTTgGlpvLYzcTpyoC3yfK9UCIQCOfvDUGr4svMtYyad+67zkmsJX0yFaR1Vncat9wx95yjAKBggqhkjOPQQDAgNJADBGAiEAjRfNX//s+AR39XNgW0SXXfra6z+To5tgRIloujWlxEYCIQDVoFBLzin5gUp+WTVEcwgq6snwhIKP5sR1Cf7dxmH9qw==";
    const COINBASE_WE1_DER_B64: &str = "MIICnzCCAiWgAwIBAgIQf/MZd5csIkp2FV0TttaF4zAKBggqhkjOPQQDAzBHMQswCQYDVQQGEwJVUzEiMCAGA1UEChMZR29vZ2xlIFRydXN0IFNlcnZpY2VzIExMQzEUMBIGA1UEAxMLR1RTIFJvb3QgUjQwHhcNMjMxMjEzMDkwMDAwWhcNMjkwMjIwMTQwMDAwWjA7MQswCQYDVQQGEwJVUzEeMBwGA1UEChMVR29vZ2xlIFRydXN0IFNlcnZpY2VzMQwwCgYDVQQDEwNXRTEwWTATBgcqhkjOPQIBBggqhkjOPQMBBwNCAARvzTr+Z1dHTCEDhUDCR127WEcPQMFcF4XGGTfn1XzthkubgdnXGhOlCgP4mMTG6J7/EFmPLCaY9eYmJbsPAvpWo4H+MIH7MA4GA1UdDwEB/wQEAwIBhjAdBgNVHSUEFjAUBggrBgEFBQcDAQYIKwYBBQUHAwIwEgYDVR0TAQH/BAgwBgEB/wIBADAdBgNVHQ4EFgQUkHeSNWfE/6jMqeZ72YB5e8yT+TgwHwYDVR0jBBgwFoAUgEzW63T/STaj1dj8tT7FavCUHYwwNAYIKwYBBQUHAQEEKDAmMCQGCCsGAQUFBzAChhhodHRwOi8vaS5wa2kuZ29vZy9yNC5jcnQwKwYDVR0fBCQwIjAgoB6gHIYaaHR0cDovL2MucGtpLmdvb2cvci9yNC5jcmwwEwYDVR0gBAwwCjAIBgZngQwBAgEwCgYIKoZIzj0EAwMDaAAwZQIxAOcCq1HW90OVznX+0RGU1cxAQXomvtgM8zItPZCuFQ8jSBJSjz5keROv9aYsAm5VsQIwJonMaAFi54mrfhfoFNZEfuNMSQ6/bIBiNLiyoX46FohQvKeIoJ99cx7sUkFN7uJW";
    const GTS_ROOT_R4_CROSS_DER_B64: &str = "MIIDejCCAmKgAwIBAgIQf+UwvzMTQ77dghYQST2KGzANBgkqhkiG9w0BAQsFADBXMQswCQYDVQQGEwJCRTEZMBcGA1UEChMQR2xvYmFsU2lnbiBudi1zYTEQMA4GA1UECxMHUm9vdCBDQTEbMBkGA1UEAxMSR2xvYmFsU2lnbiBSb290IENBMB4XDTIzMTExNTAzNDMyMVoXDTI4MDEyODAwMDA0MlowRzELMAkGA1UEBhMCVVMxIjAgBgNVBAoTGUdvb2dsZSBUcnVzdCBTZXJ2aWNlcyBMTEMxFDASBgNVBAMTC0dUUyBSb290IFI0MHYwEAYHKoZIzj0CAQYFK4EEACIDYgAE83Rzp2iLYK5DuDXFgTB7S0md+8FhzubeRr1r1WEYNa5A3XP3iZEwWus87oV8okB2O6nGuEfYKueSkWpz6bFyOZ8pn6KY019eWIZlD6GEZQbR3IvJx3PIjGov5cSr0R2Ko4H/MIH8MA4GA1UdDwEB/wQEAwIBhjAdBgNVHSUEFjAUBggrBgEFBQcDAQYIKwYBBQUHAwIwDwYDVR0TAQH/BAUwAwEB/zAdBgNVHQ4EFgQUgEzW63T/STaj1dj8tT7FavCUHYwwHwYDVR0jBBgwFoAUYHtmGkUNl8qJUC99BM00qP/8/UswNgYIKwYBBQUHAQEEKjAoMCYGCCsGAQUFBzAChhpodHRwOi8vaS5wa2kuZ29vZy9nc3IxLmNydDAtBgNVHR8EJjAkMCKgIKAehhxodHRwOi8vYy5wa2kuZ29vZy9yL2dzcjEuY3JsMBMGA1UdIAQMMAowCAYGZ4EMAQIBMA0GCSqGSIb3DQEBCwUAA4IBAQAYQrsPBtYDh5bjP2OBDwmkoWhIDDkic574y04tfzHpn+cJodI2D4SseesQ6bDrarZ7C30ddLibZatoKiws3UL9xnELz4ct92vID24FfVbiI1hY+SW6FoVHkNeWIP0GCbaM4C6uVdF5dTUsMVs/ZbzNnIdCp5Gxmx5ejvEau8otR/CskGN+hr/W5GvT1tMBjgWKZ1i4//emhA1JG1BbPzoLJQvyEotc03lXjTaCzv8mEbep8RqZ7a2CPsgRbuvTPBwcOMBBmuFeU88+FSBX6+7iP0il8b4Z0QFqIwwMHfs/L6K1vepuoxtGzi4CZ68zJpiq1UvSqTbFJjtbD4seiMHl";

    fn coinbase_tls_chain() -> Vec<String> {
        vec![
            COINBASE_LEAF_DER_B64.to_string(),
            COINBASE_WE1_DER_B64.to_string(),
            GTS_ROOT_R4_CROSS_DER_B64.to_string(),
        ]
    }

    fn tls() -> TlsInfo {
        TlsInfo {
            sni: HOST.to_string(),
            cert_chain_der_b64: coinbase_tls_chain(),
        }
    }

    fn payload() -> ProofPayload {
        let body_bytes = br#"{"candles":[{"start":"1713718800","low":"76000.0","high":"76200.0","open":"76100.0","close":"76150.0","volume":"12.34"}]}"#;
        ProofPayload {
            schema: PAYLOAD_SCHEMA.to_string(),
            proof_policy: PROOF_POLICY.to_string(),
            source: SOURCE.to_string(),
            host: HOST.to_string(),
            product_id: PRODUCT_ID.to_string(),
            granularity: GRANULARITY_LABEL.to_string(),
            granularity_seconds: GRANULARITY_SECONDS,
            request_start: 1713718800,
            request_end: 1713718800,
            request_path: REQUEST_PATH.to_string(),
            request_query: "start=1713718800&end=1713718800&granularity=FIVE_MINUTE&limit=1"
                .to_string(),
            http_status: 200,
            http_date: "Mon, 20 Apr 2026 20:45:30 GMT".to_string(),
            content_type: "application/json".to_string(),
            body_sha256: sha256_hex(body_bytes),
            body_b64: b64_encode(body_bytes),
            selected_candle: SelectedCandle {
                time: 1713718800,
                low: "76000.0".to_string(),
                high: "76200.0".to_string(),
                open: "76100.0".to_string(),
                close: "76150.0".to_string(),
                volume: "12.34".to_string(),
            },
            tls: tls(),
        }
    }

    fn bundle_for(payload: &ProofPayload) -> ProofBundle {
        let payload_bytes = serde_json::to_vec(payload).unwrap();
        let mut attester = MockAttester::new(1776717930, pcrs());
        let user_data = sha384(&payload_bytes);
        let nonce = random_nonce();
        let attestation = attester.attest(&user_data, &nonce).unwrap();
        ProofBundle {
            schema: BUNDLE_SCHEMA.to_string(),
            payload_json_b64: b64_encode(&payload_bytes),
            attestation_doc_b64: b64_encode(&attestation),
        }
    }

    fn config() -> VerificationConfig {
        VerificationConfig::new(pcrs())
    }

    #[test]
    fn verifies_mock_bundle() {
        let bundle = bundle_for(&payload());
        let verified =
            verify_bundle(&bundle, &MockAttestationVerifier, &config(), now_utc()).unwrap();
        assert_eq!(verified.payload.selected_candle.time, 1713718800);
    }

    #[test]
    fn accepts_valid_tls_chain() {
        verify_tls_certificate_chain(&tls(), 1776717930, HOST).unwrap();
    }

    #[test]
    fn rejects_empty_tls_chain() {
        let tls = TlsInfo {
            sni: HOST.to_string(),
            cert_chain_der_b64: Vec::new(),
        };
        assert!(verify_tls_certificate_chain(&tls, 1776717930, HOST).is_err());
    }

    #[test]
    fn rejects_malformed_tls_certificate_base64() {
        let tls = TlsInfo {
            sni: HOST.to_string(),
            cert_chain_der_b64: vec!["not base64".to_string()],
        };
        assert!(verify_tls_certificate_chain(&tls, 1776717930, HOST).is_err());
    }

    #[test]
    fn rejects_tls_chain_for_wrong_dns_name() {
        assert!(verify_tls_certificate_chain(&tls(), 1776717930, "example.com").is_err());
    }

    #[test]
    fn rejects_tls_chain_before_certificate_validity() {
        assert!(verify_tls_certificate_chain(&tls(), 1700000000, HOST).is_err());
    }

    #[test]
    fn rejects_tls_chain_after_certificate_validity() {
        assert!(verify_tls_certificate_chain(&tls(), 1800000000, HOST).is_err());
    }

    #[test]
    fn rejects_payload_tampering() {
        let mut payload = payload();
        let bundle = bundle_for(&payload);
        payload.product_id = "ETH-USD".to_string();
        let tampered_bytes = serde_json::to_vec(&payload).unwrap();
        let mut tampered = bundle;
        tampered.payload_json_b64 = b64_encode(&tampered_bytes);
        assert!(verify_bundle(&tampered, &MockAttestationVerifier, &config(), now_utc()).is_err());
    }

    #[test]
    fn rejects_wrong_pcr() {
        let bundle = bundle_for(&payload());
        let mut config = config();
        config.expected_pcrs.insert(1, vec![9u8; 48]);
        assert!(verify_bundle(&bundle, &MockAttestationVerifier, &config, now_utc()).is_err());
    }

    #[test]
    fn rejects_zero_pcr() {
        let payload = payload();
        let payload_bytes = serde_json::to_vec(&payload).unwrap();
        let mut pcrs = pcrs();
        pcrs.insert(1, vec![0u8; 48]);
        let mut attester = MockAttester::new(1776717930, pcrs);
        let attestation = attester
            .attest(&sha384(&payload_bytes), &random_nonce())
            .unwrap();
        let bundle = ProofBundle {
            schema: BUNDLE_SCHEMA.to_string(),
            payload_json_b64: b64_encode(&payload_bytes),
            attestation_doc_b64: b64_encode(&attestation),
        };
        assert!(verify_bundle(&bundle, &MockAttestationVerifier, &config(), now_utc()).is_err());
    }

    #[test]
    fn rejects_mismatched_attestation_user_data() {
        let payload = payload();
        let payload_bytes = serde_json::to_vec(&payload).unwrap();
        let mut attester = MockAttester::new(1776717930, pcrs());
        let attestation = attester.attest(&[9u8; 48], &random_nonce()).unwrap();
        let bundle = ProofBundle {
            schema: BUNDLE_SCHEMA.to_string(),
            payload_json_b64: b64_encode(&payload_bytes),
            attestation_doc_b64: b64_encode(&attestation),
        };
        assert!(verify_bundle(&bundle, &MockAttestationVerifier, &config(), now_utc()).is_err());
    }

    #[test]
    fn accepts_historical_observed_response() {
        let payload = payload();
        let bundle = bundle_for(&payload);
        let verified =
            verify_bundle(&bundle, &MockAttestationVerifier, &config(), now_utc()).unwrap();
        assert_eq!(verified.payload.request_start, 1713718800);
        assert_eq!(verified.payload.http_date, "Mon, 20 Apr 2026 20:45:30 GMT");
    }

    #[test]
    fn rejects_attestation_coinbase_date_skew() {
        let mut payload = payload();
        payload.http_date = "Mon, 20 Apr 2026 21:30:00 GMT".to_string();
        let bundle = bundle_for(&payload);
        assert!(verify_bundle(&bundle, &MockAttestationVerifier, &config(), now_utc()).is_err());
    }

    #[test]
    fn rejects_selected_candle_not_in_body_b64() {
        let mut payload = payload();
        payload.selected_candle.close = "1.0".to_string();
        let bundle = bundle_for(&payload);
        assert!(verify_bundle(&bundle, &MockAttestationVerifier, &config(), now_utc()).is_err());
    }

    #[test]
    fn rejects_unaligned_selected_candle() {
        let mut payload = payload();
        payload.selected_candle.time = 1713718801;
        let bundle = bundle_for(&payload);
        assert!(verify_bundle(&bundle, &MockAttestationVerifier, &config(), now_utc()).is_err());
    }

    #[test]
    fn rejects_selected_candle_outside_request_range() {
        let mut payload = payload();
        payload.request_start = 1713719100;
        payload.request_end = 1713719100;
        payload.request_query =
            "start=1713719100&end=1713719100&granularity=FIVE_MINUTE&limit=1".to_string();
        let bundle = bundle_for(&payload);
        assert!(verify_bundle(&bundle, &MockAttestationVerifier, &config(), now_utc()).is_err());
    }

    #[test]
    fn rejects_wide_request_range() {
        let mut payload = payload();
        payload.request_end = 1713719100;
        payload.request_query =
            "start=1713718800&end=1713719100&granularity=FIVE_MINUTE&limit=1".to_string();
        let bundle = bundle_for(&payload);
        assert!(verify_bundle(&bundle, &MockAttestationVerifier, &config(), now_utc()).is_err());
    }

    #[test]
    fn rejects_unaligned_request_start() {
        let mut payload = payload();
        payload.request_start = 1713718801;
        payload.request_end = 1713718801;
        payload.request_query =
            "start=1713718801&end=1713718801&granularity=FIVE_MINUTE&limit=1".to_string();
        let bundle = bundle_for(&payload);
        assert!(verify_bundle(&bundle, &MockAttestationVerifier, &config(), now_utc()).is_err());
    }

    #[test]
    fn rejects_future_attestation_timestamp() {
        let bundle = bundle_for(&payload());
        let now = OffsetDateTime::from_unix_timestamp(1776717629).unwrap();
        assert!(verify_bundle(&bundle, &MockAttestationVerifier, &config(), now).is_err());
    }

    #[test]
    fn mock_attestation_schema_constant_is_stable() {
        assert_eq!(
            MOCK_ATTESTATION_SCHEMA,
            "coinbase-candle-mock-attestation/v1"
        );
    }
}
