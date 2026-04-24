use ::time::OffsetDateTime;
use anyhow::{anyhow, bail, Context, Result};
use coset::{CborSerializable, CoseSign1};
use ring::{
    digest,
    signature::{UnparsedPublicKey, ECDSA_P384_SHA384_FIXED},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, iter::once};
use x509_parser::{
    certificate::X509Certificate,
    oid_registry::OID_SIG_ECDSA_WITH_SHA384,
    prelude::{FromDer, Validator},
    validate::X509StructureValidator,
    x509::SubjectPublicKeyInfo,
};

use crate::crypto::{b64_decode, b64_encode};
use crate::MOCK_ATTESTATION_SCHEMA;

const NITRO_ROOT_G1_SHA256: [u8; 32] = [
    0x64, 0x1a, 0x03, 0x21, 0xa3, 0xe2, 0x44, 0xef, 0xe4, 0x56, 0x46, 0x31, 0x95, 0xd6, 0x06, 0x31,
    0x7e, 0xd7, 0xcd, 0xcc, 0x3c, 0x17, 0x56, 0xe0, 0x98, 0x93, 0xf3, 0xc6, 0x8f, 0x79, 0xbb, 0x5b,
];

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
        parse_and_verify_nitro_attestation(document, now)
            .with_context(|| "Nitro attestation verification failed")
    }
}

#[derive(Debug)]
struct NitroCert<'a> {
    cert: &'a [u8],
    idx: usize,
    x509: X509Certificate<'a>,
}

impl<'a> NitroCert<'a> {
    fn parse(cert: &'a [u8], idx: usize) -> Result<Self> {
        let (_, x509) = X509Certificate::from_der(cert)
            .map_err(|err| anyhow!("certificate {idx} failed to parse: {err}"))?;

        let mut logger = x509_parser::validate::VecLogger::default();
        if !X509StructureValidator.validate(&x509, &mut logger) {
            bail!("certificate {idx} structure malformed: {logger:?}");
        }

        Ok(Self { cert, idx, x509 })
    }

    fn common_name(&self) -> Result<&str> {
        self.x509
            .subject()
            .iter_common_name()
            .next()
            .ok_or_else(|| anyhow!("certificate {} common name missing", self.idx))?
            .as_str()
            .with_context(|| format!("certificate {} common name is not a string", self.idx))
    }

    fn fingerprint(&self) -> ring::digest::Digest {
        digest::digest(&digest::SHA256, self.cert)
    }

    fn public_key(&'a self) -> &'a SubjectPublicKeyInfo<'a> {
        self.x509.public_key()
    }

    fn validate_at(self, timestamp: OffsetDateTime) -> Result<Self> {
        let not_before = self.x509.validity.not_before.to_datetime();
        if timestamp < not_before {
            bail!(
                "certificate {} ({}) was not valid at attestation time",
                self.idx,
                self.common_name()?
            );
        }

        let not_after = self.x509.validity.not_after.to_datetime();
        if timestamp > not_after {
            bail!(
                "certificate {} ({}) expired before attestation time",
                self.idx,
                self.common_name()?
            );
        }

        Ok(self)
    }

    fn verify_signature(self, parent: Option<&NitroCert<'_>>) -> Result<Self> {
        let algorithm = self.x509.signature_algorithm.oid();
        if algorithm != &OID_SIG_ECDSA_WITH_SHA384 {
            bail!(
                "certificate {} ({}) used unexpected signature algorithm {algorithm}",
                self.idx,
                self.common_name()?
            );
        }

        if let Some(parent) = parent {
            self.x509
                .verify_signature(Some(parent.public_key()))
                .map_err(|err| {
                    anyhow!(
                        "certificate {} ({}) signature verification failed: {err}",
                        self.idx,
                        self.common_name().unwrap_or("<unknown>")
                    )
                })?;
        } else if self.fingerprint().as_ref() != NITRO_ROOT_G1_SHA256 {
            bail!(
                "root certificate fingerprint mismatch: have {}, want {}",
                hex::encode(self.fingerprint().as_ref()),
                hex::encode(NITRO_ROOT_G1_SHA256)
            );
        }

        Ok(self)
    }
}

fn parse_and_verify_nitro_attestation(
    document: &[u8],
    _now: OffsetDateTime,
) -> Result<VerifiedAttestation> {
    let cose = CoseSign1::from_slice(document)
        .map_err(|err| anyhow!("attestation COSE was malformed: {err:?}"))?;
    let payload = cose
        .payload
        .as_ref()
        .ok_or_else(|| anyhow!("attestation COSE payload missing"))?;
    let doc = aws_nitro_enclaves_nsm_api::api::AttestationDoc::from_binary(payload.as_slice())
        .map_err(|err| anyhow!("attestation CBOR payload was malformed: {err:?}"))?;

    let timestamp_unix = i64::try_from(doc.timestamp / 1000)
        .with_context(|| format!("attestation timestamp was too large: {}", doc.timestamp))?;
    let timestamp = OffsetDateTime::from_unix_timestamp(timestamp_unix)
        .with_context(|| format!("attestation timestamp was invalid: {timestamp_unix}"))?;

    let certs = doc
        .cabundle
        .iter()
        .chain(once(&doc.certificate))
        .enumerate()
        .map(|(idx, cert)| NitroCert::parse(cert.as_ref(), idx))
        .collect::<Result<Vec<_>>>()?;

    let signing_cert = certs
        .into_iter()
        .try_fold(None, |parent, cert| {
            let cert = cert.verify_signature(parent.as_ref())?;
            let cert = cert.validate_at(timestamp)?;
            Ok::<_, anyhow::Error>(Some(cert))
        })?
        .ok_or_else(|| anyhow!("attestation document did not contain certificates"))?;

    let public_key = signing_cert.public_key().subject_public_key.as_ref();
    cose.verify_signature(&[], |sig, data| {
        UnparsedPublicKey::new(&ECDSA_P384_SHA384_FIXED, public_key)
            .verify(data, sig)
            .map_err(|_| {
                coset::CoseError::UnexpectedItem("valid COSE signature", "invalid COSE signature")
            })
    })
    .map_err(|err| anyhow!("attestation COSE signature verification failed: {err:?}"))?;

    let pcrs = doc
        .pcrs
        .into_iter()
        .filter(|(_, digest)| !digest.iter().all(|byte| *byte == 0))
        .map(|(idx, digest)| {
            let idx = u16::try_from(idx).with_context(|| format!("PCR index too large: {idx}"))?;
            Ok((idx, digest.to_vec()))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;

    Ok(VerifiedAttestation {
        timestamp_unix,
        pcrs,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn sample_real_attestation_doc() -> Vec<u8> {
        let bundle: Value = serde_json::from_str(include_str!("../proof.json")).unwrap();
        b64_decode(
            bundle
                .get("attestation_doc_b64")
                .and_then(Value::as_str)
                .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn verifies_real_attestation_after_leaf_cert_expiry() {
        let verifier = RealNitroAttestationVerifier;
        let doc = sample_real_attestation_doc();
        let now_after_leaf_expiry = OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap();
        let verified = verifier.verify(&doc, now_after_leaf_expiry).unwrap();
        assert!(verified.timestamp_unix < now_after_leaf_expiry.unix_timestamp());
        assert!(verified.pcrs.contains_key(&0));
    }

    #[test]
    fn verifies_real_attestation_at_document_time_not_wall_clock() {
        let verifier = RealNitroAttestationVerifier;
        let doc = sample_real_attestation_doc();
        let verified = verifier
            .verify(
                &doc,
                OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap(),
            )
            .unwrap();
        let document_time = OffsetDateTime::from_unix_timestamp(verified.timestamp_unix).unwrap();
        assert!(verifier.verify(&doc, document_time).is_ok());
    }
}
