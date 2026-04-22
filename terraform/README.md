# AWS Deployment

This Terraform deploys Choracle as a public HTTPS service on a single Nitro
Enclave-enabled EC2 parent instance.

## Resources

- VPC, public subnet, internet gateway, and route table
- Nitro Enclave-enabled EC2 parent instance
- Elastic IP
- security group allowing inbound TCP 443
- IAM role/profile with SSM Session Manager access
- optional Route53 `A` record for `proof_fqdn`

The parent instance bootstraps itself with cloud-init:

1. Installs Docker, Nitro CLI, Go, Git, jq, and build tools.
2. Builds `gvproxy`.
3. Clones the configured public Git repository/ref.
4. Builds the enclave Docker image and EIF.
5. Writes PCR measurements under `/opt/choracle/build`.
6. Starts `gvproxy`, exposes parent port 443 to nitriding, and runs the enclave.

## Requirements

- AWS credentials for Terraform.
- A public DNS name for the proof service, for example `proof.example.com`.
- A public Git repository URL/ref that the EC2 parent can clone.
- Route53 hosted zone ID if Terraform should manage the DNS record.

If `route53_zone_id` is omitted, create the DNS record manually after apply:

```text
proof.example.com A <terraform output parent_public_ip>
```

## Configure

```sh
cd terraform
cp terraform.tfvars.example terraform.tfvars
```

Example:

```hcl
proof_fqdn      = "proof.example.com"
route53_zone_id = "Z0123456789ABCDEFG"
repo_url        = "https://github.com/stutxo/choracle.git"
repo_ref        = "main"
```

## Deploy

```sh
terraform init
terraform apply
```

The first boot can take several minutes because the instance builds `gvproxy`,
the enclave image, and the EIF.

## Outputs

```sh
terraform output -raw proof_url
terraform output -raw parent_public_ip
terraform output -raw parent_instance_id
terraform output -raw ssm_session_command
terraform output -raw pcrs_command
```

The PCR command prints an AWS SSM command. Run that command, then inspect the
SSM command invocation output to retrieve PCR0, PCR1, and PCR2.

The parent also writes PCRs to:

```text
/opt/choracle/build/auth-price.pcrs.txt
```

## Test

Root path, expected to return any HTTP response once HTTPS is listening:

```sh
curl -v "$(terraform output -raw proof_url)/"
```

Proof endpoint:

```sh
curl -sS --fail \
  "$(terraform output -raw proof_url)/proof/v1/products/BTC-USD/candles?start=1713718800&end=1713719100&granularity=FIVE_MINUTE&limit=1" \
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
systemctl status choracle-build-eif.service
systemctl status choracle-expose-nitriding.service
systemctl status choracle-enclave.service
```

Bootstrap log:

```sh
tail -n 200 /var/log/choracle-bootstrap.log
```

