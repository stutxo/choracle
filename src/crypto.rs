use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256, Sha384};

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn sha384(bytes: &[u8]) -> Vec<u8> {
    Sha384::digest(bytes).to_vec()
}

pub fn random_nonce() -> Vec<u8> {
    let mut nonce = vec![0u8; 32];
    OsRng.fill_bytes(&mut nonce);
    nonce
}

pub fn b64_encode(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

pub fn b64_decode(value: &str) -> Result<Vec<u8>> {
    STANDARD
        .decode(value)
        .with_context(|| "base64 decode failed")
}

pub fn decode_hex_48(value: &str) -> Result<Vec<u8>> {
    let bytes = hex::decode(value).with_context(|| "PCR hex decode failed")?;
    if bytes.len() != 48 {
        return Err(anyhow!(
            "PCR must be a SHA384 digest, got {} bytes",
            bytes.len()
        ));
    }
    Ok(bytes)
}
