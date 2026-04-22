# Manual Nitriding Deployment

The supported automated deployment is in `terraform/`. This directory contains
the lower-level commands used by that deployment.

## Build Enclave Image

The canonical release build is Nix-based. The proof FQDN is runtime config from
the parent instance and is not baked into the EIF.

```sh
BUILD_DIR=/tmp/choracle-build \
  deploy/build-reproducible-eif.sh
```

Record PCR0, PCR1, and PCR2 from `release-manifest.json` or
`measurements.json`.

The older `Dockerfile.enclave` path is retained for development only. It uses
mutable distro/package inputs and should not be used for published PCRs.

## Start Parent Networking

Run `gvproxy`:

```sh
bash deploy/start-gvproxy.sh
```

Expose parent port 443 to nitriding's static TAP address:

```sh
bash deploy/expose-nitriding.sh
```

## Run Enclave

The enclave expects a parent-side FQDN config server on vsock port `11001`:

```sh
choracle-runtime-config serve-fqdn --fqdn proof.example.com --port 11001
```

```sh
nitro-cli run-enclave \
  --enclave-name choracle-proof \
  --cpu-count 2 \
  --memory 1024 \
  --eif-path choracle.eif
```

## Test

```sh
curl "https://proof.example.com/proof/v1/products/BTC-USD/candles?start=1713718800&end=1713719100&granularity=FIVE_MINUTE&limit=1"
```

Nitriding's attestation endpoint remains available:

```text
GET /enclave/attestation?nonce=<40 hex chars>
```
