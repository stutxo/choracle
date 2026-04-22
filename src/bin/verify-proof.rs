use anyhow::{anyhow, Context, Result};
use clap::Parser;
use coinbase_candle_prover::attestation::{MockAttestationVerifier, RealNitroAttestationVerifier};
use coinbase_candle_prover::crypto::decode_hex_48;
use coinbase_candle_prover::timeutil::now_utc;
use coinbase_candle_prover::verify::{verify_bundle_json, VerificationConfig};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(about = "Verify a Coinbase candle proof artifact")]
struct Args {
    artifact: PathBuf,

    #[arg(
        long = "pcr",
        value_parser = parse_pcr,
        help = "Expected PCR in the form INDEX=SHA384_HEX. Pass PCR0, PCR1, and PCR2."
    )]
    pcrs: Vec<(u16, Vec<u8>)>,

    #[arg(long, help = "Accept the local JSON mock attestation format")]
    mock_attestation: bool,

    #[arg(long, default_value_t = 300)]
    max_skew_seconds: i64,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let bytes = fs::read(&args.artifact)
        .with_context(|| format!("failed to read {}", args.artifact.display()))?;
    let expected_pcrs: BTreeMap<u16, Vec<u8>> = args.pcrs.into_iter().collect();
    let mut config = VerificationConfig::new(expected_pcrs);
    config.max_skew_seconds = args.max_skew_seconds;

    let verified = if args.mock_attestation {
        verify_bundle_json(&bytes, &MockAttestationVerifier, &config, now_utc())?
    } else {
        verify_bundle_json(&bytes, &RealNitroAttestationVerifier, &config, now_utc())?
    };

    println!(
        "OK: verified {} {}s candle starting at {}",
        verified.payload.product_id,
        verified.payload.granularity_seconds,
        verified.payload.selected_candle.time
    );
    println!(
        "close={} volume={} attestation_time={}",
        verified.payload.selected_candle.close,
        verified.payload.selected_candle.volume,
        verified.attestation.timestamp_unix
    );
    Ok(())
}

fn parse_pcr(value: &str) -> Result<(u16, Vec<u8>)> {
    let (idx, hex_value) = value
        .split_once('=')
        .ok_or_else(|| anyhow!("PCR must use INDEX=HEX format"))?;
    let idx = idx
        .parse::<u16>()
        .with_context(|| format!("invalid PCR index: {idx}"))?;
    Ok((idx, decode_hex_48(hex_value)?))
}
