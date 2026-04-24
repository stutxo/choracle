# Release Artifact Build

The supported deployment path is in `terraform/`. This directory contains the
release artifact builder consumed by that Terraform deployment.

## Build Artifacts

Run this on a Nitro-capable ARM64 Linux builder with Nix, Docker, Nitro CLI,
Git, and Go installed:

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

`choracle.eif`, `release-manifest.json`, `gvproxy`, and
`choracle-runtime-config` are passed to Terraform as local artifact paths.

## Notes

The public proof FQDN is runtime config from the parent instance and is not
baked into the EIF. PCR0, PCR1, and PCR2 must come from `release-manifest.json`,
`measurements.json`, or `choracle.pcrs.txt`; do not derive them by inspection.
