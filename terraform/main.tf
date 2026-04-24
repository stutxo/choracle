provider "aws" {
  region = var.region
}

data "aws_availability_zones" "available" {
  state = "available"
}

data "aws_ssm_parameter" "parent_ami" {
  name = var.ami_ssm_parameter
}

locals {
  availability_zone = var.availability_zone != "" ? var.availability_zone : data.aws_availability_zones.available.names[0]

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

resource "aws_vpc_security_group_ingress_rule" "http_acme" {
  security_group_id = aws_security_group.parent.id
  cidr_ipv4         = "0.0.0.0/0"
  from_port         = 80
  ip_protocol       = "tcp"
  to_port           = 80
  description       = "Public HTTP for ACME HTTP-01 validation"
}

resource "aws_vpc_security_group_egress_rule" "all_ipv4" {
  security_group_id = aws_security_group.parent.id
  cidr_ipv4         = "0.0.0.0/0"
  ip_protocol       = "-1"
  description       = "Outbound for package installs, git, ACME, and Coinbase"
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
    http_put_response_hop_limit = 2
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
    enclave_cpu_count  = var.enclave_cpu_count
    enclave_memory_mib = var.enclave_memory_mib
    enclave_name       = var.enclave_name
    proof_fqdn         = var.proof_fqdn
    region             = var.region
    source_ref         = var.source_ref
    source_repo_url    = var.source_repo_url
  })

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
