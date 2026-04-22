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
    condition     = length(trimspace(var.proof_fqdn)) > 0 && !can(regex("^https?://", var.proof_fqdn))
    error_message = "proof_fqdn must be a bare DNS name, not a URL."
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

variable "repo_url" {
  description = "Public Git repository URL cloned by the EC2 bootstrap."
  type        = string
  default     = "https://github.com/stutxo/choracle.git"
}

variable "repo_ref" {
  description = "Git branch, tag, or commit checked out by the EC2 bootstrap."
  type        = string
  default     = "main"
}

variable "nitriding_commit" {
  description = "nitriding-daemon commit baked into the enclave image."
  type        = string
  default     = "2b7dfefaee56819681b7f5a4ee8d66a417ad457d"
}

variable "gvproxy_ref" {
  description = "gvisor-tap-vsock ref used to build gvproxy on the parent instance."
  type        = string
  default     = "v0.7.4"
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
  description = "Root EBS volume size. Docker builds need room for Rust, Go, and EIF layers."
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
