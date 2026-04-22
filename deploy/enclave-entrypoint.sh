#!/bin/sh
set -eu

PROOF_FQDN="$(
  /usr/local/bin/choracle-runtime-config fetch-fqdn \
    --cid "${CHORACLE_PARENT_CID:-3}" \
    --port "${CHORACLE_FQDN_CONFIG_PORT:-11001}" \
    --retries "${CHORACLE_FQDN_CONFIG_RETRIES:-60}" \
    --retry-delay-millis "${CHORACLE_FQDN_CONFIG_RETRY_DELAY_MILLIS:-1000}"
)"
export PROOF_FQDN

exec /usr/local/bin/nitriding \
  -fqdn "$PROOF_FQDN" \
  -acme \
  -appwebsrv "http://${PROOF_HTTP_LISTEN:-127.0.0.1:8081}" \
  -wait-for-app \
  -appcmd "/usr/local/bin/enclave-prover --serve-http --http-listen ${PROOF_HTTP_LISTEN:-127.0.0.1:8081}"
