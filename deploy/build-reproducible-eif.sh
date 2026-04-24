#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR=${PROJECT_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}
BUILD_DIR=${BUILD_DIR:-"$PROJECT_DIR/build"}
IMAGE_TAG=${IMAGE_TAG:-"choracle-enclave:reproducible"}
EIF_PATH=${EIF_PATH:-"$BUILD_DIR/choracle.eif"}
GVPROXY_REF=${GVPROXY_REF:-"v0.7.4"}
GVPROXY_SRC_DIR=${GVPROXY_SRC_DIR:-"$BUILD_DIR/gvisor-tap-vsock"}
GVPROXY_PATH=${GVPROXY_PATH:-"$BUILD_DIR/gvproxy"}
RUNTIME_CONFIG_PATH=${RUNTIME_CONFIG_PATH:-"$BUILD_DIR/choracle-runtime-config"}
NIX_FLAGS=${NIX_FLAGS:-"--extra-experimental-features nix-command --extra-experimental-features flakes"}
NIX=${NIX:-nix}

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

preflight() {
  case "$(uname -s)" in
    Linux) ;;
    *) fail "this builder must run on ARM64 Linux with Nitro CLI; use a Nitro-capable Graviton EC2 builder, not macOS" ;;
  esac

  case "$(uname -m)" in
    aarch64|arm64) ;;
    *) fail "this builder targets aarch64-linux; run it on an ARM64 Linux builder" ;;
  esac

  require_command "$NIX"
  require_command docker
  require_command nitro-cli
  require_command git
  require_command go
  require_command jq
  require_command sha256sum
}

preflight

mkdir -p "$BUILD_DIR"

oci_store_path=$($NIX $NIX_FLAGS build "$PROJECT_DIR#choracle-enclave-oci-aarch64" --no-link --print-out-paths)
tools_store_path=$($NIX $NIX_FLAGS build "$PROJECT_DIR#choracle-tools-aarch64" --no-link --print-out-paths)

oci_tar="$BUILD_DIR/choracle-enclave-image.tar"
cp -f "$oci_store_path" "$oci_tar"

if [ -w /usr/local/bin ]; then
  install -m 0755 "$tools_store_path/bin/choracle-runtime-config" /usr/local/bin/choracle-runtime-config
fi
install -m 0755 "$tools_store_path/bin/choracle-runtime-config" "$RUNTIME_CONFIG_PATH"

if [ ! -x "$GVPROXY_PATH" ]; then
  if [ ! -d "$GVPROXY_SRC_DIR/.git" ]; then
    rm -rf "$GVPROXY_SRC_DIR"
    git clone --depth 1 --branch "$GVPROXY_REF" https://github.com/containers/gvisor-tap-vsock.git "$GVPROXY_SRC_DIR"
  fi
  (cd "$GVPROXY_SRC_DIR" && git fetch --tags --depth 1 origin "$GVPROXY_REF" && git checkout "$GVPROXY_REF")
  (cd "$GVPROXY_SRC_DIR" && go build -o "$GVPROXY_PATH" ./cmd/gvproxy)
fi

docker load -i "$oci_tar"
if [ "$IMAGE_TAG" != "choracle-enclave:reproducible" ]; then
  docker tag choracle-enclave:reproducible "$IMAGE_TAG"
fi

nitro-cli build-enclave \
  --docker-uri "$IMAGE_TAG" \
  --output-file "$EIF_PATH" \
  | tee "$BUILD_DIR/measurements.log"

sed -n '/^{/,$p' "$BUILD_DIR/measurements.log" > "$BUILD_DIR/measurements.json"
jq -r '.Measurements | to_entries[] | "\(.key)=\(.value)"' \
  "$BUILD_DIR/measurements.json" \
  > "$BUILD_DIR/choracle.pcrs.txt"

git_commit=$(git -C "$PROJECT_DIR" rev-parse HEAD)
oci_sha256=$(sha256sum "$oci_tar" | awk '{ print $1 }')
eif_sha256=$(sha256sum "$EIF_PATH" | awk '{ print $1 }')
gvproxy_sha256=$(sha256sum "$GVPROXY_PATH" | awk '{ print $1 }')
runtime_config_sha256=$(sha256sum "$RUNTIME_CONFIG_PATH" | awk '{ print $1 }')
image_id=$(docker image inspect "$IMAGE_TAG" --format '{{.Id}}')
flake_lock_sha256=""
if [ -f "$PROJECT_DIR/flake.lock" ]; then
  flake_lock_sha256=$(sha256sum "$PROJECT_DIR/flake.lock" | awk '{ print $1 }')
fi

jq -n \
  --slurpfile measurements "$BUILD_DIR/measurements.json" \
  --arg git_commit "$git_commit" \
  --arg flake_lock_sha256 "$flake_lock_sha256" \
  --arg oci_image_tar "$oci_tar" \
  --arg oci_image_sha256 "$oci_sha256" \
  --arg docker_image "$IMAGE_TAG" \
  --arg docker_image_id "$image_id" \
  --arg eif_path "$EIF_PATH" \
  --arg eif_sha256 "$eif_sha256" \
  --arg gvproxy_path "$GVPROXY_PATH" \
  --arg gvproxy_sha256 "$gvproxy_sha256" \
  --arg runtime_config_path "$RUNTIME_CONFIG_PATH" \
  --arg runtime_config_sha256 "$runtime_config_sha256" \
  --arg target_arch "aarch64-linux" \
  --arg proof_bundle_schema "coinbase-candle-proof-bundle/v1" \
  '{
    git_commit: $git_commit,
    flake_lock_sha256: (if $flake_lock_sha256 == "" then null else $flake_lock_sha256 end),
    target_arch: $target_arch,
    proof_bundle_schema: $proof_bundle_schema,
    oci_image: {
      path: $oci_image_tar,
      sha256: $oci_image_sha256,
      docker_tag: $docker_image,
      docker_image_id: $docker_image_id
    },
    eif: {
      path: $eif_path,
      sha256: $eif_sha256
    },
    parent_artifacts: {
      gvproxy: {
        path: $gvproxy_path,
        sha256: $gvproxy_sha256
      },
      choracle_runtime_config: {
        path: $runtime_config_path,
        sha256: $runtime_config_sha256
      }
    },
    measurements: $measurements[0].Measurements
  }' > "$BUILD_DIR/release-manifest.json"

test -s "$oci_tar"
test -s "$EIF_PATH"
test -s "$GVPROXY_PATH"
test -s "$RUNTIME_CONFIG_PATH"
test -s "$BUILD_DIR/measurements.json"
test -s "$BUILD_DIR/release-manifest.json"

echo "Wrote $BUILD_DIR/release-manifest.json"
