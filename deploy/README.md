# Manual Nitriding Deployment

The supported automated deployment is in `terraform/`. This directory contains
the lower-level commands used by that deployment.

## Build Enclave Image

Set DNS for the proof FQDN before building. `nitriding` uses ACME, and the FQDN
is baked into the EIF, so changing it changes PCRs.

```sh
docker build \
  -f Dockerfile.enclave \
  -t choracle-enclave:latest \
  --build-arg PROOF_FQDN=proof.example.com \
  --build-arg NITRIDING_COMMIT=2b7dfefaee56819681b7f5a4ee8d66a417ad457d \
  .

nitro-cli build-enclave \
  --docker-uri choracle-enclave:latest \
  --output-file choracle.eif
```

Record PCR0, PCR1, and PCR2 from the `nitro-cli build-enclave` output.

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

