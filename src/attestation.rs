use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use time::OffsetDateTime;

use crate::crypto::{b64_decode, b64_encode};
use crate::MOCK_ATTESTATION_SCHEMA;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedAttestation {
    pub timestamp_unix: i64,
    pub pcrs: BTreeMap<u16, Vec<u8>>,
    pub user_data: Vec<u8>,
    pub nonce: Vec<u8>,
}

pub trait Attester {
    fn attest(&mut self, user_data: &[u8], nonce: &[u8]) -> Result<Vec<u8>>;
}

pub trait AttestationVerifier {
    fn verify(&self, document: &[u8], now: OffsetDateTime) -> Result<VerifiedAttestation>;
}

pub struct RealNsmAttester {
    fd: i32,
}

impl RealNsmAttester {
    pub fn open() -> Result<Self> {
        let fd = aws_nitro_enclaves_nsm_api::driver::nsm_init();
        if fd < 0 {
            return Err(anyhow!(
                "failed to open /dev/nsm; are you inside a Nitro Enclave?"
            ));
        }
        Ok(Self { fd })
    }
}

impl Drop for RealNsmAttester {
    fn drop(&mut self) {
        aws_nitro_enclaves_nsm_api::driver::nsm_exit(self.fd);
    }
}

impl Attester for RealNsmAttester {
    fn attest(&mut self, user_data: &[u8], nonce: &[u8]) -> Result<Vec<u8>> {
        use aws_nitro_enclaves_nsm_api::api::{Request, Response};
        use serde_bytes::ByteBuf;

        let response = aws_nitro_enclaves_nsm_api::driver::nsm_process_request(
            self.fd,
            Request::Attestation {
                public_key: None,
                user_data: Some(ByteBuf::from(user_data.to_vec())),
                nonce: Some(ByteBuf::from(nonce.to_vec())),
            },
        );

        match response {
            Response::Attestation { document } => Ok(document),
            Response::Error(code) => Err(anyhow!("NSM attestation failed: {code:?}")),
            other => Err(anyhow!("unexpected NSM response to attestation: {other:?}")),
        }
    }
}

pub struct RealNitroAttestationVerifier;

impl AttestationVerifier for RealNitroAttestationVerifier {
    fn verify(&self, document: &[u8], now: OffsetDateTime) -> Result<VerifiedAttestation> {
        let doc: nitro_attest::UnparsedAttestationDoc<'_> = document.into();
        let doc = doc
            .parse_and_verify(now)
            .map_err(|err| anyhow!("Nitro attestation verification failed: {err}"))?;

        Ok(VerifiedAttestation {
            timestamp_unix: doc.timestamp.unix_timestamp(),
            pcrs: doc
                .pcrs
                .into_iter()
                .map(|(idx, digest)| (idx, digest.value))
                .collect(),
            user_data: doc
                .user_data
                .map(|value| value.to_vec())
                .ok_or_else(|| anyhow!("attestation document did not contain user_data"))?,
            nonce: doc
                .nonce
                .map(|value| value.to_vec())
                .ok_or_else(|| anyhow!("attestation document did not contain nonce"))?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MockAttestationDocument {
    schema: String,
    timestamp_unix: i64,
    pcrs: BTreeMap<String, String>,
    user_data_b64: String,
    nonce_b64: String,
}

#[derive(Debug, Clone)]
pub struct MockAttester {
    pub timestamp_unix: i64,
    pub pcrs: BTreeMap<u16, Vec<u8>>,
}

impl MockAttester {
    pub fn new(timestamp_unix: i64, pcrs: BTreeMap<u16, Vec<u8>>) -> Self {
        Self {
            timestamp_unix,
            pcrs,
        }
    }
}

impl Attester for MockAttester {
    fn attest(&mut self, user_data: &[u8], nonce: &[u8]) -> Result<Vec<u8>> {
        let doc = MockAttestationDocument {
            schema: MOCK_ATTESTATION_SCHEMA.to_string(),
            timestamp_unix: self.timestamp_unix,
            pcrs: self
                .pcrs
                .iter()
                .map(|(idx, value)| (idx.to_string(), hex::encode(value)))
                .collect(),
            user_data_b64: b64_encode(user_data),
            nonce_b64: b64_encode(nonce),
        };
        serde_json::to_vec(&doc).with_context(|| "failed to serialize mock attestation")
    }
}

pub struct MockAttestationVerifier;

impl AttestationVerifier for MockAttestationVerifier {
    fn verify(&self, document: &[u8], _now: OffsetDateTime) -> Result<VerifiedAttestation> {
        let doc: MockAttestationDocument = serde_json::from_slice(document)
            .with_context(|| "failed to parse mock attestation document")?;
        if doc.schema != MOCK_ATTESTATION_SCHEMA {
            return Err(anyhow!(
                "unexpected mock attestation schema: {}",
                doc.schema
            ));
        }

        let mut pcrs = BTreeMap::new();
        for (idx, value) in doc.pcrs {
            let idx = idx
                .parse::<u16>()
                .with_context(|| format!("invalid mock PCR index: {idx}"))?;
            let value = hex::decode(value).with_context(|| "invalid mock PCR hex")?;
            pcrs.insert(idx, value);
        }

        Ok(VerifiedAttestation {
            timestamp_unix: doc.timestamp_unix,
            pcrs,
            user_data: b64_decode(&doc.user_data_b64)?,
            nonce: b64_decode(&doc.nonce_b64)?,
        })
    }
}
