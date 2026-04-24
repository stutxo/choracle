output "parent_instance_id" {
  description = "EC2 parent instance ID."
  value       = aws_instance.parent.id
}

output "parent_public_ip" {
  description = "Elastic IP assigned to the parent instance."
  value       = aws_eip.parent.public_ip
}

output "proof_url" {
  description = "Public proof API base URL."
  value       = "https://${var.proof_fqdn}"
}

output "artifact_bucket" {
  description = "Private S3 bucket containing uploaded release artifacts."
  value       = aws_s3_bucket.artifacts.id
}

output "artifact_release_id" {
  description = "Release artifact key prefix derived from the EIF SHA-256."
  value       = local.release_id
}

output "dns_record" {
  description = "DNS record managed by Terraform, when route53_zone_id is set."
  value       = var.route53_zone_id == "" ? "not managed by this Terraform config" : one(aws_route53_record.proof[*].fqdn)
}

output "ssm_session_command" {
  description = "Optional command for debugging the parent host over SSM."
  value       = "aws ssm start-session --region ${var.region} --target ${aws_instance.parent.id}"
}

output "pcrs_command" {
  description = "SSM command to print the EIF build output containing PCR0/PCR1/PCR2."
  value       = "aws ssm send-command --region ${var.region} --instance-ids ${aws_instance.parent.id} --document-name AWS-RunShellScript --parameters commands='[\"cat /opt/choracle/build/auth-price.pcrs.txt\"]'"
}

output "release_manifest_command" {
  description = "SSM command to print the Choracle reproducible build release manifest."
  value       = "aws ssm send-command --region ${var.region} --instance-ids ${aws_instance.parent.id} --document-name AWS-RunShellScript --parameters commands='[\"cat /opt/choracle/build/release-manifest.json\"]'"
}
