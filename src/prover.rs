use crate::attestation::Attester;
use crate::candle::select_candle;
use crate::crypto::{b64_encode, random_nonce, sha256_hex, sha384};
use crate::http::{coinbase_candle_query, fetch_coinbase_over_tls, header_value};
use crate::proof::{ProofBundle, ProofPayload, ProofRequest};
use crate::{
    BUNDLE_SCHEMA, GRANULARITY_LABEL, GRANULARITY_SECONDS, HOST, PAYLOAD_SCHEMA, PRODUCT_ID,
    PROOF_POLICY, REQUEST_PATH, SOURCE,
};
use anyhow::{anyhow, Context, Result};

pub struct Prover<A> {
    attester: A,
}

impl<A> Prover<A>
where
    A: Attester,
{
    pub fn new(attester: A) -> Self {
        Self { attester }
    }

    pub fn prove(&mut self, request: ProofRequest) -> Result<ProofBundle> {
        validate_request(&request)?;

        let query = coinbase_candle_query(request.start, request.end)?;
        let stream = connect_coinbase()?;
        let fetch = fetch_coinbase_over_tls(stream, &query)?;
        if fetch.status != 200 {
            return Err(anyhow!("Coinbase returned HTTP status {}", fetch.status));
        }

        let content_type = header_value(&fetch.headers, "content-type")
            .ok_or_else(|| anyhow!("Coinbase response omitted Content-Type"))?
            .to_string();
        if !content_type
            .to_ascii_lowercase()
            .starts_with("application/json")
        {
            return Err(anyhow!(
                "Coinbase returned unexpected Content-Type: {content_type}"
            ));
        }
        let http_date = header_value(&fetch.headers, "date")
            .ok_or_else(|| anyhow!("Coinbase response omitted Date header"))?
            .to_string();
        let selected_candle = select_candle(&fetch.body_json)?;

        let payload = ProofPayload {
            schema: PAYLOAD_SCHEMA.to_string(),
            proof_policy: PROOF_POLICY.to_string(),
            source: SOURCE.to_string(),
            host: HOST.to_string(),
            product_id: PRODUCT_ID.to_string(),
            granularity: GRANULARITY_LABEL.to_string(),
            granularity_seconds: GRANULARITY_SECONDS,
            request_start: request.start,
            request_end: request.end,
            request_path: REQUEST_PATH.to_string(),
            request_query: query,
            http_status: fetch.status,
            http_date,
            content_type,
            body_sha256: sha256_hex(&fetch.body_bytes),
            body_b64: b64_encode(&fetch.body_bytes),
            selected_candle,
            tls: fetch.tls,
        };

        let payload_bytes =
            serde_json::to_vec(&payload).with_context(|| "failed to serialize proof payload")?;
        let nonce = random_nonce();
        let user_data = sha384(&payload_bytes);
        let attestation_doc = self
            .attester
            .attest(&user_data, &nonce)
            .with_context(|| "failed to obtain attestation document")?;

        Ok(ProofBundle {
            schema: BUNDLE_SCHEMA.to_string(),
            payload_json_b64: b64_encode(&payload_bytes),
            attestation_doc_b64: b64_encode(&attestation_doc),
        })
    }
}

pub fn validate_request(request: &ProofRequest) -> Result<()> {
    if request.start < 0 {
        return Err(anyhow!("start must be non-negative"));
    }
    if request.end < 0 {
        return Err(anyhow!("end must be non-negative"));
    }
    if request.start > request.end {
        return Err(anyhow!("start must be less than or equal to end"));
    }
    Ok(())
}

fn connect_coinbase() -> Result<std::net::TcpStream> {
    let stream = std::net::TcpStream::connect((HOST, 443))
        .with_context(|| format!("failed to connect to {HOST}:443"))?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(30)))
        .ok();
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(30)))
        .ok();
    Ok(stream)
}
