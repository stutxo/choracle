#!/bin/sh
set -eu

if [ -z "${PROOF_FQDN:-}" ]; then
  echo "PROOF_FQDN must be set at image build time" >&2
  exit 1
fi

exec /usr/local/bin/nitriding \
  -fqdn "$PROOF_FQDN" \
  -acme \
  -appwebsrv "http://${PROOF_HTTP_LISTEN:-127.0.0.1:8081}" \
  -wait-for-app \
  -appcmd "/usr/local/bin/enclave-prover --serve-http --http-listen ${PROOF_HTTP_LISTEN:-127.0.0.1:8081}"
