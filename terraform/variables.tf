variable "region" {
  description = "AWS region to deploy into."
  type        = string
  default     = "us-east-1"
}

variable "name" {
  description = "Name prefix for AWS resources."
  type        = string
  default     = "choracle"
}

variable "environment" {
  description = "Environment tag value."
  type        = string
  default     = "prod"
}

variable "proof_fqdn" {
  description = "Public DNS name served by nitriding inside the enclave, for example proof.example.com."
  type        = string

  validation {
    condition = (
      length(trimspace(var.proof_fqdn)) == length(var.proof_fqdn) &&
      length(var.proof_fqdn) <= 253 &&
      length(split(".", var.proof_fqdn)) >= 2 &&
      !can(regex("^https?://", var.proof_fqdn)) &&
      !strcontains(var.proof_fqdn, "/") &&
      !strcontains(var.proof_fqdn, ":") &&
      !strcontains(var.proof_fqdn, "_") &&
      !strcontains(var.proof_fqdn, "..") &&
      can(regex("^[A-Za-z0-9][A-Za-z0-9.-]*[A-Za-z0-9]$", var.proof_fqdn))
    )
    error_message = "proof_fqdn must be a bare DNS name, not a URL."
  }
}

variable "eif_path" {
  description = "Local path to the prebuilt Choracle EIF artifact."
  type        = string

  validation {
    condition     = fileexists(var.eif_path)
    error_message = "eif_path must point to an existing local file."
  }
}

variable "release_manifest_path" {
  description = "Local path to the release-manifest.json produced with the EIF."
  type        = string

  validation {
    condition     = fileexists(var.release_manifest_path)
    error_message = "release_manifest_path must point to an existing local file."
  }
}

variable "gvproxy_path" {
  description = "Local path to the prebuilt gvproxy binary for the parent instance architecture."
  type        = string

  validation {
    condition     = fileexists(var.gvproxy_path)
    error_message = "gvproxy_path must point to an existing local file."
  }
}

variable "runtime_config_path" {
  description = "Local path to the prebuilt choracle-runtime-config binary for the parent instance architecture."
  type        = string

  validation {
    condition     = fileexists(var.runtime_config_path)
    error_message = "runtime_config_path must point to an existing local file."
  }
}

variable "artifact_bucket_name" {
  description = "Optional S3 bucket name for release artifacts. Defaults to an account/region-scoped name."
  type        = string
  default     = ""
}

variable "artifact_bucket_force_destroy" {
  description = "Whether Terraform may delete the release artifact bucket while it contains objects."
  type        = bool
  default     = false
}

variable "route53_zone_id" {
  description = "Optional Route53 hosted zone ID. When set, Terraform creates an A record for proof_fqdn."
  type        = string
  default     = ""
}

variable "allowed_https_cidr" {
  description = "CIDR allowed to reach the public HTTPS endpoint."
  type        = string
  default     = "0.0.0.0/0"
}

variable "instance_type" {
  description = "Nitro Enclave-capable parent instance type. Default is ARM64 Graviton."
  type        = string
  default     = "m6g.xlarge"
}

variable "ami_ssm_parameter" {
  description = "SSM public parameter for the parent instance AMI."
  type        = string
  default     = "/aws/service/ami-amazon-linux-latest/al2023-ami-kernel-default-arm64"
}

variable "vpc_cidr" {
  description = "CIDR block for the deployment VPC."
  type        = string
  default     = "10.42.0.0/16"
}

variable "public_subnet_cidr" {
  description = "CIDR block for the public subnet containing the parent instance."
  type        = string
  default     = "10.42.1.0/24"
}

variable "availability_zone" {
  description = "Optional availability zone. Defaults to the first available AZ in the region."
  type        = string
  default     = ""
}

variable "root_volume_size_gb" {
  description = "Root EBS volume size for the parent host runtime and downloaded release artifacts."
  type        = number
  default     = 30
}

variable "enclave_cpu_count" {
  description = "vCPUs allocated to the enclave."
  type        = number
  default     = 2
}

variable "enclave_memory_mib" {
  description = "Memory allocated to the enclave."
  type        = number
  default     = 1024
}

variable "enclave_name" {
  description = "Nitro Enclave name."
  type        = string
  default     = "choracle-proof"
}
