# BTC/USD Oracle Nitro Prover

Choracle is a Nitro Enclave proof service for Coinbase `BTC-USD` 5-minute
candles. The production service runs behind `nitriding-daemon`, which terminates
public HTTPS inside the enclave and exposes Nitro attestation.

## Components

- `enclave-prover`: enclave HTTP service that fetches Coinbase v3 public-market
  candles, builds a proof bundle, and binds the bundle to a Nitro
  attestation document.
- `verify-proof`: offline verifier for proof bundles.
- `Dockerfile.enclave`: enclave image with `nitriding`, `enclave-prover`, and
  `verify-proof`.
- `terraform/`: one-instance AWS deployment for a public HTTPS proof endpoint.
- `deploy/`: lower-level manual deployment scripts.

## Proof API

```text
GET /proof/v1/products/BTC-USD/candles?start=<epoch>&end=<epoch>&granularity=FIVE_MINUTE&limit=1
```

The proof bundle schema is `coinbase-candle-proof-bundle/v1`. Its attested
payload schema is `coinbase-candle-proof-payload/v1`. The proof bundle contains:

- base64-encoded payload JSON
- the Coinbase TLS server certificate chain, DER-encoded and base64-wrapped in
  leaf-first order, inside the attested payload
- Nitro attestation document whose `user_data` binds
  `SHA384(payload_json_bytes)` directly to the enclave measurements

The proof policy is historical observed-response semantics: the enclave proves
that it fetched the Coinbase response at attestation time. The request time does
not need to be near the candle close.

## Local Development

```sh
cargo test
```

Local one-shot smoke test with mock attestation:

```sh
cargo run --bin enclave-prover -- \
  --once \
  --mock-attestation \
  --start 1713718800 \
  --end 1713719100 \
  --output /tmp/proof.json
```

Mock verification:

```sh
cargo run --bin verify-proof -- \
  --mock-attestation \
  --pcr 0=010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101 \
  --pcr 1=020202020202020202020202020202020202020202020202020202020202020202020202020202020202020202020202 \
  --pcr 2=030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303 \
  /tmp/proof.json
```

## AWS Deployment

Use [terraform/README.md](terraform/README.md) for the standard public AWS
deployment. The deployment requires:

- a public DNS name such as `proof.example.com`
- a public GitHub repository URL/ref that the EC2 parent can clone
- a Nitro Enclave-capable EC2 parent instance

Real Nitro attestation requires a non-debug Nitro Enclave. Debug-mode
attestations produce all-zero PCRs and are rejected by the verifier.

## Verification

Get PCR0, PCR1, and PCR2 from `nitro-cli build-enclave` or from the Terraform
deployment output, then verify a production proof:

```sh
cargo run --bin verify-proof -- proof.json \
  --pcr 0=<PCR0_HEX> \
  --pcr 1=<PCR1_HEX> \
  --pcr 2=<PCR2_HEX>
```

Verification checks the attestation first, then validates the recorded Coinbase
TLS certificate chain for `api.coinbase.com` at the attestation timestamp using
the WebPKI root store. Certificate pinning and revocation checks are not used.
