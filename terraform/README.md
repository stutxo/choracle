# AWS Deployment

This Terraform deploys Choracle as a public HTTPS service on a single Nitro
Enclave-enabled EC2 parent instance. The deployed proof is enclave-observed
HTTPS evidence, not a standalone TLS transcript proof.

## Resources

- VPC, public subnet, internet gateway, and route table
- Nitro Enclave-enabled EC2 parent instance
- Elastic IP
- security group allowing inbound TCP 443
- IAM role/profile with SSM Session Manager access
- private encrypted S3 bucket for release artifacts
- optional Route53 `A` record for `proof_fqdn`

The parent instance bootstraps itself with cloud-init:

1. Installs Nitro CLI, AWS CLI, jq, and minimal runtime tools.
2. Downloads prebuilt release artifacts from the private S3 artifact bucket.
3. Verifies each artifact's SHA-256 digest.
4. Writes PCR measurements and the release manifest under `/opt/choracle/build`.
5. Starts `gvproxy`, serves the runtime FQDN over vsock, exposes parent port
   443 to nitriding, and runs the enclave.

## Requirements

- AWS credentials for Terraform.
- A public DNS name for the proof service, for example `proof.example.com`.
- Prebuilt release artifacts from `deploy/build-reproducible-eif.sh`.
- Route53 hosted zone ID if Terraform should manage the DNS record.

If `route53_zone_id` is omitted, create the DNS record manually after apply:

```text
proof.example.com A <terraform output parent_public_ip>
```

## Configure

Build release artifacts first:

```sh
BUILD_DIR="$PWD/build" \
  deploy/build-reproducible-eif.sh
```

```sh
cd terraform
cp terraform.tfvars.example terraform.tfvars
```

Example:

```hcl
proof_fqdn            = "proof.example.com"
eif_path              = "../build/choracle.eif"
release_manifest_path = "../build/release-manifest.json"
gvproxy_path          = "../build/gvproxy"
runtime_config_path   = "../build/choracle-runtime-config"
route53_zone_id       = "Z0123456789ABCDEFG"
```

## Deploy

```sh
terraform init
terraform apply
```

Terraform uploads the local artifacts to a private encrypted S3 bucket. First
boot downloads and verifies those artifacts, then starts the parent services.

## Outputs

```sh
terraform output -raw proof_url
terraform output -raw artifact_bucket
terraform output -raw artifact_release_id
terraform output -raw parent_public_ip
terraform output -raw parent_instance_id
terraform output -raw ssm_session_command
terraform output -raw pcrs_command
terraform output -raw release_manifest_command
```

The PCR command prints an AWS SSM command. Run that command; it returns a
`CommandId`. Then retrieve the result:

```sh
aws ssm get-command-invocation \
  --region <REGION> \
  --command-id <COMMAND_ID> \
  --instance-id <INSTANCE_ID>
```

Use the `StandardOutputContent` PCR0, PCR1, and PCR2 values as the verifier
policy for that deployed release.

The parent also writes PCRs to:

```text
/opt/choracle/build/auth-price.pcrs.txt
```

The reproducible build manifest is written to:

```text
/opt/choracle/build/release-manifest.json
```

## Test

Root path, expected to return any HTTP response once HTTPS is listening:

```sh
curl -v "$(terraform output -raw proof_url)/"
```

Proof endpoint:

```sh
curl -sS --fail \
  "$(terraform output -raw proof_url)/proof/v1/products/BTC-USD/candles?start=1713718800&end=1713718800&granularity=FIVE_MINUTE&limit=1" \
  -o proof.json
```

Inspect response shape:

```sh
jq 'keys' proof.json
```

## Verify

```sh
cargo run --bin verify-proof -- proof.json \
  --pcr 0=<PCR0_HEX> \
  --pcr 1=<PCR1_HEX> \
  --pcr 2=<PCR2_HEX>
```

## Operations

SSM access:

```sh
terraform output -raw ssm_session_command
```

Services on the parent:

```sh
systemctl status gvproxy.service
systemctl status choracle-expose-nitriding.service
systemctl status choracle-fqdn-config.service
systemctl status choracle-enclave.service
```

Bootstrap log:

```sh
tail -n 200 /var/log/choracle-bootstrap.log
```
