use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProofBundle {
    pub schema: String,
    pub payload_json_b64: String,
    pub attestation_doc_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProofPayload {
    pub schema: String,
    pub proof_policy: String,
    pub source: String,
    pub host: String,
    pub product_id: String,
    pub granularity: String,
    pub granularity_seconds: i64,
    pub request_start: i64,
    pub request_end: i64,
    pub request_path: String,
    pub request_query: String,
    pub http_status: u16,
    pub http_date: String,
    pub content_type: String,
    pub body_sha256: String,
    pub body_b64: String,
    pub selected_candle: SelectedCandle,
    pub tls: TlsInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SelectedCandle {
    pub time: i64,
    pub low: String,
    pub high: String,
    pub open: String,
    pub close: String,
    pub volume: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TlsInfo {
    pub sni: String,
    pub cert_chain_der_b64: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProofRequest {
    pub start: i64,
    pub end: i64,
}
