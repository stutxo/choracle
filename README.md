# BTC/USD Oracle Nitro Prover

Choracle is a Nitro Enclave proof service for Coinbase `BTC-USD` 5-minute
candles. The production service runs behind `nitriding-daemon`, which terminates
public HTTPS inside the enclave and exposes Nitro attestation.

## Components

- `enclave-prover`: enclave HTTP service that fetches Coinbase v3 public-market
  candles, builds a proof bundle, and binds the bundle to a Nitro
  attestation document.
- `verify-proof`: offline verifier for proof bundles.
- `flake.nix`: canonical reproducible enclave OCI image build.
- `Dockerfile.enclave`: legacy development image path; it is not the
  reproducible release build.
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

## Reproducible Builds

The release build is the Nix-built OCI image:

```sh
nix build .#choracle-enclave-oci-aarch64
```

On a Nitro-capable Linux builder, create the EIF and release manifest with:

```sh
BUILD_DIR=/tmp/choracle-build \
  deploy/build-reproducible-eif.sh
```

This writes:

- `choracle-enclave-image.tar`
- `choracle.eif`
- `measurements.json`
- `release-manifest.json`

The public proof FQDN is not baked into the measured image. At runtime the
enclave entrypoint fetches the FQDN from the parent instance over Nitro vsock,
then starts `nitriding`.

## Verification

PCR0, PCR1, and PCR2 are measured outputs of the release build. Do not derive
them by inspection. Get them from `nitro-cli build-enclave`, from
`release-manifest.json`, or from the Terraform deployment output.

After a Terraform deploy:

```sh
cd terraform
terraform output -raw pcrs_command
terraform output -raw release_manifest_command
```

Run the printed SSM command, then fetch its result:

```sh
aws ssm get-command-invocation \
  --region <REGION> \
  --command-id <COMMAND_ID> \
  --instance-id <INSTANCE_ID>
```

The parent writes the same values to:

```text
/opt/choracle/build/auth-price.pcrs.txt
/opt/choracle/build/release-manifest.json
```

Then verify a production proof:

```sh
cargo run --bin verify-proof -- proof.json \
  --pcr 0=<PCR0_HEX> \
  --pcr 1=<PCR1_HEX> \
  --pcr 2=<PCR2_HEX>
```

Verification checks the attestation first, then validates the recorded Coinbase
TLS certificate chain for `api.coinbase.com` at the attestation timestamp using
the WebPKI root store. Certificate pinning and revocation checks are not used.


## Example

```json
{
  "schema": "coinbase-candle-proof-bundle/v1",
  "payload": {
    "schema": "coinbase-candle-proof-payload/v1",
    "proof_policy": "coinbase-v3-observed-response/v1",
    "source": "coinbase_public_market",
    "host": "api.coinbase.com",
    "product_id": "BTC-USD",
    "granularity": "FIVE_MINUTE",
    "granularity_seconds": 300,
    "request_start": 1776890400,
    "request_end": 1776890400,
    "request_path": "/api/v3/brokerage/market/products/BTC-USD/candles",
    "request_query": "start=1776890400&end=1776890400&granularity=FIVE_MINUTE&limit=1",
    "http_status": 200,
    "http_date": "Wed, 22 Apr 2026 20:47:56 GMT",
    "content_type": "application/json; charset=utf-8",
    "body_sha256": "43c231c1347ea9050357677f60ff7782c91ffa261a27da32642af595b9cfd10b",
    "body_b64": "eyJjYW5kbGVzIjpbeyJzdGFydCI6IjE3NzY4OTA0MDAiLCAibG93IjoiNzg1MzkuMDciLCAiaGlnaCI6Ijc4NTk3LjA2IiwgIm9wZW4iOiI3ODU0OS42MSIsICJjbG9zZSI6Ijc4NTM5LjUiLCAidm9sdW1lIjoiMTcuNzcxMjUxMDMifV19",
    "selected_candle": {
      "time": 1776890400,
      "low": "78539.07",
      "high": "78597.06",
      "open": "78549.61",
      "close": "78539.5",
      "volume": "17.77125103"
    },
    "tls": {
      "sni": "api.coinbase.com",
      "cert_chain_der_b64": [
        "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"
      ]
    },
    "coinbase_body": {
      "candles": [
        {
          "start": "1776890400",
          "low": "78539.07",
          "high": "78597.06",
          "open": "78549.61",
          "close": "78539.5",
          "volume": "17.77125103"
        }
      ]
    }
  },
  "attestation_doc_b64": "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"
}
