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

variable "source_repo_url" {
  description = "Git repository URL cloned by the parent instance to build the Choracle release artifacts."
  type        = string
  default     = "https://github.com/stutxo/choracle.git"

  validation {
    condition     = length(trimspace(var.source_repo_url)) == length(var.source_repo_url) && length(var.source_repo_url) > 0
    error_message = "source_repo_url must be a non-empty Git repository URL without leading or trailing whitespace."
  }
}

variable "source_ref" {
  description = "Git branch, tag, or commit checked out by the parent instance before building release artifacts. Prefer an immutable commit SHA for production."
  type        = string
  default     = "main"

  validation {
    condition     = length(trimspace(var.source_ref)) == length(var.source_ref) && length(var.source_ref) > 0
    error_message = "source_ref must be a non-empty Git ref without leading or trailing whitespace."
  }
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
  description = "Root EBS volume size for the parent host Nix store, release build, and runtime artifacts."
  type        = number
  default     = 100
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
