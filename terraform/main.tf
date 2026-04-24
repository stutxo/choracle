provider "aws" {
  region = var.region
}

data "aws_availability_zones" "available" {
  state = "available"
}

data "aws_caller_identity" "current" {}

data "aws_ssm_parameter" "parent_ami" {
  name = var.ami_ssm_parameter
}

locals {
  availability_zone = var.availability_zone != "" ? var.availability_zone : data.aws_availability_zones.available.names[0]
  artifact_bucket   = var.artifact_bucket_name != "" ? var.artifact_bucket_name : lower("${var.name}-${var.environment}-${data.aws_caller_identity.current.account_id}-${var.region}-artifacts")
  release_id        = substr(filesha256(var.eif_path), 0, 16)

  artifact_files = {
    eif = {
      source = var.eif_path
      key    = "releases/${local.release_id}/choracle.eif"
      sha256 = filesha256(var.eif_path)
    }
    release_manifest = {
      source = var.release_manifest_path
      key    = "releases/${local.release_id}/release-manifest.json"
      sha256 = filesha256(var.release_manifest_path)
    }
    gvproxy = {
      source = var.gvproxy_path
      key    = "releases/${local.release_id}/gvproxy"
      sha256 = filesha256(var.gvproxy_path)
    }
    runtime_config = {
      source = var.runtime_config_path
      key    = "releases/${local.release_id}/choracle-runtime-config"
      sha256 = filesha256(var.runtime_config_path)
    }
  }

  common_tags = {
    Name        = var.name
    Terraform   = "true"
    Environment = var.environment
    Project     = "choracle"
  }
}

resource "aws_vpc" "this" {
  cidr_block           = var.vpc_cidr
  enable_dns_hostnames = true
  enable_dns_support   = true

  tags = merge(local.common_tags, {
    Name = "${var.name}-vpc"
  })
}

resource "aws_internet_gateway" "this" {
  vpc_id = aws_vpc.this.id

  tags = merge(local.common_tags, {
    Name = "${var.name}-igw"
  })
}

resource "aws_subnet" "public" {
  vpc_id                  = aws_vpc.this.id
  cidr_block              = var.public_subnet_cidr
  availability_zone       = local.availability_zone
  map_public_ip_on_launch = false

  tags = merge(local.common_tags, {
    Name = "${var.name}-public"
  })
}

resource "aws_route_table" "public" {
  vpc_id = aws_vpc.this.id

  tags = merge(local.common_tags, {
    Name = "${var.name}-public"
  })
}

resource "aws_route" "public_internet" {
  route_table_id         = aws_route_table.public.id
  destination_cidr_block = "0.0.0.0/0"
  gateway_id             = aws_internet_gateway.this.id
}

resource "aws_route_table_association" "public" {
  subnet_id      = aws_subnet.public.id
  route_table_id = aws_route_table.public.id
}

resource "aws_security_group" "parent" {
  name_prefix = "${var.name}-parent-"
  description = "Choracle Nitro parent host"
  vpc_id      = aws_vpc.this.id

  tags = merge(local.common_tags, {
    Name = "${var.name}-parent"
  })
}

resource "aws_vpc_security_group_ingress_rule" "https" {
  security_group_id = aws_security_group.parent.id
  cidr_ipv4         = var.allowed_https_cidr
  from_port         = 443
  ip_protocol       = "tcp"
  to_port           = 443
  description       = "Public HTTPS to nitriding"
}

resource "aws_vpc_security_group_egress_rule" "all_ipv4" {
  security_group_id = aws_security_group.parent.id
  cidr_ipv4         = "0.0.0.0/0"
  ip_protocol       = "-1"
  description       = "Outbound for package installs, S3 artifacts, ACME, and Coinbase"
}

resource "aws_s3_bucket" "artifacts" {
  bucket        = local.artifact_bucket
  force_destroy = var.artifact_bucket_force_destroy

  tags = merge(local.common_tags, {
    Name = "${var.name}-artifacts"
  })
}

resource "aws_s3_bucket_public_access_block" "artifacts" {
  bucket = aws_s3_bucket.artifacts.id

  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

resource "aws_s3_bucket_server_side_encryption_configuration" "artifacts" {
  bucket = aws_s3_bucket.artifacts.id

  rule {
    apply_server_side_encryption_by_default {
      sse_algorithm = "AES256"
    }
  }
}

resource "aws_s3_bucket_versioning" "artifacts" {
  bucket = aws_s3_bucket.artifacts.id

  versioning_configuration {
    status = "Enabled"
  }
}

resource "aws_s3_object" "artifacts" {
  for_each = local.artifact_files

  bucket                 = aws_s3_bucket.artifacts.id
  key                    = each.value.key
  source                 = each.value.source
  source_hash            = filebase64sha256(each.value.source)
  server_side_encryption = "AES256"

  tags = local.common_tags
}

data "aws_iam_policy_document" "ec2_assume_role" {
  statement {
    actions = ["sts:AssumeRole"]

    principals {
      type        = "Service"
      identifiers = ["ec2.amazonaws.com"]
    }
  }
}

resource "aws_iam_role" "parent" {
  name               = "${var.name}-parent-role"
  assume_role_policy = data.aws_iam_policy_document.ec2_assume_role.json
  tags               = local.common_tags
}

resource "aws_iam_role_policy_attachment" "ssm_core" {
  role       = aws_iam_role.parent.name
  policy_arn = "arn:aws:iam::aws:policy/AmazonSSMManagedInstanceCore"
}

data "aws_iam_policy_document" "parent_artifacts" {
  statement {
    actions   = ["s3:GetObject"]
    resources = [for object in aws_s3_object.artifacts : object.arn]
  }
}

resource "aws_iam_role_policy" "parent_artifacts" {
  name   = "${var.name}-artifact-read"
  role   = aws_iam_role.parent.id
  policy = data.aws_iam_policy_document.parent_artifacts.json
}

resource "aws_iam_instance_profile" "parent" {
  name = "${var.name}-parent-profile"
  role = aws_iam_role.parent.name
}

resource "aws_instance" "parent" {
  ami                         = data.aws_ssm_parameter.parent_ami.value
  instance_type               = var.instance_type
  subnet_id                   = aws_subnet.public.id
  vpc_security_group_ids      = [aws_security_group.parent.id]
  iam_instance_profile        = aws_iam_instance_profile.parent.name
  associate_public_ip_address = true
  monitoring                  = true
  user_data_replace_on_change = true

  metadata_options {
    http_endpoint               = "enabled"
    http_put_response_hop_limit = 1
    http_tokens                 = "required"
  }

  enclave_options {
    enabled = true
  }

  root_block_device {
    volume_type           = "gp3"
    volume_size           = var.root_volume_size_gb
    encrypted             = true
    delete_on_termination = true
  }

  user_data = templatefile("${path.module}/scripts/choracle-bootstrap.sh.tftpl", {
    artifact_bucket         = aws_s3_bucket.artifacts.id
    enclave_cpu_count       = var.enclave_cpu_count
    enclave_memory_mib      = var.enclave_memory_mib
    enclave_name            = var.enclave_name
    eif_key                 = aws_s3_object.artifacts["eif"].key
    eif_sha256              = local.artifact_files.eif.sha256
    gvproxy_key             = aws_s3_object.artifacts["gvproxy"].key
    gvproxy_sha256          = local.artifact_files.gvproxy.sha256
    proof_fqdn              = var.proof_fqdn
    region                  = var.region
    release_manifest_key    = aws_s3_object.artifacts["release_manifest"].key
    release_manifest_sha256 = local.artifact_files.release_manifest.sha256
    runtime_config_key      = aws_s3_object.artifacts["runtime_config"].key
    runtime_config_sha256   = local.artifact_files.runtime_config.sha256
  })

  depends_on = [
    aws_iam_role_policy.parent_artifacts,
    aws_s3_object.artifacts,
  ]

  tags = merge(local.common_tags, {
    Name = "${var.name}-parent"
  })
}

resource "aws_eip" "parent" {
  domain = "vpc"

  tags = merge(local.common_tags, {
    Name = "${var.name}-parent"
  })
}

resource "aws_eip_association" "parent" {
  allocation_id = aws_eip.parent.id
  instance_id   = aws_instance.parent.id
}

resource "aws_route53_record" "proof" {
  count = var.route53_zone_id == "" ? 0 : 1

  zone_id = var.route53_zone_id
  name    = var.proof_fqdn
  type    = "A"
  ttl     = 60
  records = [aws_eip.parent.public_ip]
}
