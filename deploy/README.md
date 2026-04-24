# Release Artifact Build

The supported deployment path is in `terraform/`. This directory contains the
release artifact builder consumed by that Terraform deployment.

## Build Artifacts

Terraform runs this script on the Nitro parent instance during first boot. You
can also run it manually on a Nitro-capable ARM64 Linux builder with Nix,
Docker, Nitro CLI, Git, Go, jq, and sha256sum installed. A macOS workstation is
not enough for this step, even on Apple Silicon, because the EIF build depends
on the Linux Nitro CLI toolchain and the flake targets `aarch64-linux`.

```sh
BUILD_DIR=/tmp/choracle-build \
  deploy/build-reproducible-eif.sh
```

The script writes:

- `choracle-enclave-image.tar`
- `choracle.eif`
- `measurements.json`
- `choracle.pcrs.txt`
- `release-manifest.json`
- `gvproxy`
- `choracle-runtime-config`

The Terraform bootstrap consumes these files from the parent instance's
`/opt/choracle/build` directory.

## Notes

The public proof FQDN is runtime config from the parent instance and is not
baked into the EIF. PCR0, PCR1, and PCR2 must come from `release-manifest.json`,
`measurements.json`, or `choracle.pcrs.txt`; do not derive them by inspection.
