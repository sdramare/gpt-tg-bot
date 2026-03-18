terraform {
  required_version = ">= 1.5.0"

  backend "s3" {}

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = ">= 5.70.0"
    }
  }
}

provider "aws" {
  region = var.aws_region
}

variable "aws_region" {
  description = "AWS region where gpt-tg-bot exists"
  type        = string
  default     = "eu-north-1"
}

variable "function_name" {
  description = "Lambda function name"
  type        = string
  default     = "gpt-tg-bot"
}

variable "lambda_role_name" {
  description = "IAM role name used by lambda"
  type        = string
  default     = "gpt-tg-bot-role"
}

variable "lambda_zip_path" {
  description = "Path to lambda deployment zip built by cargo-lambda"
  type        = string
  default     = "../target/lambda/gpt-tg-bot/bootstrap.zip"
}

variable "lambda_log_group_name" {
  description = "CloudWatch log group name for lambda"
  type        = string
  default     = ""
}

variable "gpt_rules_file_path" {
  description = "Local path to a GPT rules file uploaded to S3. Leave empty to disable S3 upload."
  type        = string
  default     = ""
}

variable "gpt_rules_object_key" {
  description = "S3 object key for the uploaded GPT rules file."
  type        = string
  default     = "gpt-rules.txt"
}

variable "lambda_architectures_override" {
  description = "Optional architecture override, for example [\"arm64\"]"
  type        = list(string)
  default     = null
}

variable "lambda_environment" {
  description = "Lambda environment variables. Provide values matching the current function."
  type        = map(string)
  sensitive   = true

  validation {
    condition = alltrue([
      for key in [
        "BOT_ALIAS",
        "DUMMY_ANSWERS",
        "TG_TOKEN",
        "GPT_TOKEN",
        "GPT_MODEL",
        "GPT_SMART_MODEL",
        "GPT_RULES",
        "GPT_PREAMBLE",
        "TG_ALLOW_CHATS",
        "NAMES_MAP"
      ] : contains(keys(var.lambda_environment), key)
    ])
    error_message = "lambda_environment is missing one or more required keys from src/config.rs"
  }
}

variable "lambda_tags" {
  description = "Tags applied to lambda and log group"
  type        = map(string)
  default     = {}
}

data "aws_caller_identity" "current" {}

locals {
  lambda_runtime            = "provided.al2023"
  lambda_handler            = "bootstrap"
  lambda_architectures      = var.lambda_architectures_override != null ? var.lambda_architectures_override : ["x86_64"]
  lambda_memory_size        = 128
  lambda_timeout_seconds    = 120
  lambda_ephemeral_storage  = 512
  lambda_log_group_name_in  = trimspace(var.lambda_log_group_name)
  lambda_log_group_name     = local.lambda_log_group_name_in != "" ? local.lambda_log_group_name_in : "/aws/lambda/${var.function_name}"
  lambda_log_retention_days = 30
  gpt_rules_file_path       = trimspace(var.gpt_rules_file_path)
  gpt_rules_object_key      = trimspace(var.gpt_rules_object_key)
  gpt_rules_enabled         = local.gpt_rules_file_path != ""
  gpt_rules_bucket_base     = "${lower(replace(var.function_name, "/[^a-z0-9-]/", "-"))}-rules-${data.aws_caller_identity.current.account_id}-${var.aws_region}"
  gpt_rules_bucket_name     = trim(substr(local.gpt_rules_bucket_base, 0, 63), "-")
  gpt_rules_s3_uri          = "s3://${local.gpt_rules_bucket_name}/${local.gpt_rules_object_key}"
}

data "aws_iam_policy_document" "lambda_assume_role" {
  statement {
    effect = "Allow"

    principals {
      type        = "Service"
      identifiers = ["lambda.amazonaws.com"]
    }

    actions = ["sts:AssumeRole"]
  }
}

resource "aws_s3_bucket" "gpt_rules" {
  count  = local.gpt_rules_enabled ? 1 : 0
  bucket = local.gpt_rules_bucket_name

  tags = merge(
    var.lambda_tags,
    {
      Name = local.gpt_rules_bucket_name
    }
  )
}

resource "aws_s3_bucket_public_access_block" "gpt_rules" {
  count  = local.gpt_rules_enabled ? 1 : 0
  bucket = aws_s3_bucket.gpt_rules[0].id

  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

resource "aws_s3_bucket_ownership_controls" "gpt_rules" {
  count  = local.gpt_rules_enabled ? 1 : 0
  bucket = aws_s3_bucket.gpt_rules[0].id

  rule {
    object_ownership = "BucketOwnerEnforced"
  }
}

resource "aws_s3_object" "gpt_rules" {
  count  = local.gpt_rules_enabled ? 1 : 0
  bucket = aws_s3_bucket.gpt_rules[0].id
  key    = local.gpt_rules_object_key
  source = local.gpt_rules_file_path

  source_hash  = filebase64sha256(local.gpt_rules_file_path)
  content_type = "text/plain"

  tags = var.lambda_tags

  depends_on = [
    aws_s3_bucket_public_access_block.gpt_rules,
    aws_s3_bucket_ownership_controls.gpt_rules
  ]
}

data "aws_iam_policy_document" "lambda_s3_rules_readonly" {
  count = local.gpt_rules_enabled ? 1 : 0

  statement {
    sid    = "ReadGptRulesObject"
    effect = "Allow"
    actions = [
      "s3:GetObject"
    ]
    resources = [
      "${aws_s3_bucket.gpt_rules[0].arn}/${local.gpt_rules_object_key}"
    ]
  }
}

resource "aws_iam_role" "lambda" {
  name                 = var.lambda_role_name
  path                 = "/"
  assume_role_policy   = data.aws_iam_policy_document.lambda_assume_role.json
  max_session_duration = 3600

  tags = var.lambda_tags
}

resource "aws_iam_role_policy_attachment" "lambda_managed" {
  for_each = toset([
    "arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole",
    "arn:aws:iam::aws:policy/service-role/AWSLambdaRole"
  ])

  role       = aws_iam_role.lambda.name
  policy_arn = each.value
}

resource "aws_iam_role_policy" "lambda_s3_rules_readonly" {
  count  = local.gpt_rules_enabled ? 1 : 0
  name   = "${var.lambda_role_name}-s3-rules-readonly"
  role   = aws_iam_role.lambda.id
  policy = data.aws_iam_policy_document.lambda_s3_rules_readonly[0].json
}

resource "aws_cloudwatch_log_group" "lambda" {
  name              = local.lambda_log_group_name
  retention_in_days = local.lambda_log_retention_days

  tags = var.lambda_tags
}

resource "aws_lambda_function" "this" {
  function_name = var.function_name
  description   = ""
  role          = aws_iam_role.lambda.arn

  package_type  = "Zip"
  runtime       = local.lambda_runtime
  handler       = local.lambda_handler
  architectures = local.lambda_architectures

  filename         = var.lambda_zip_path
  source_code_hash = filebase64sha256(var.lambda_zip_path)
  publish          = false

  memory_size = local.lambda_memory_size
  timeout     = local.lambda_timeout_seconds

  environment {
    variables = merge(
      var.lambda_environment,
      local.gpt_rules_enabled ? {
        S3_RULES_URI = local.gpt_rules_s3_uri
      } : {}
    )
  }

  ephemeral_storage {
    size = local.lambda_ephemeral_storage
  }

  tracing_config {
    mode = "PassThrough"
  }

  logging_config {
    log_format            = "JSON"
    application_log_level = "INFO"
    system_log_level      = "INFO"
    log_group             = aws_cloudwatch_log_group.lambda.name
  }

  tags = var.lambda_tags

  depends_on = [
    aws_iam_role_policy_attachment.lambda_managed,
    aws_iam_role_policy.lambda_s3_rules_readonly,
    aws_cloudwatch_log_group.lambda
  ]
}

resource "aws_lambda_function_url" "this" {
  function_name      = aws_lambda_function.this.function_name
  authorization_type = "NONE"
  invoke_mode        = "BUFFERED"
}

resource "aws_lambda_permission" "public_function_url" {
  statement_id = "FunctionURLAllowPublicAccess"
  action                 = "lambda:InvokeFunctionUrl"
  function_name          = aws_lambda_function.this.function_name
  principal              = "*"
  function_url_auth_type = aws_lambda_function_url.this.authorization_type
}

resource "aws_lambda_runtime_management_config" "this" {
  function_name     = aws_lambda_function.this.function_name
  qualifier         = "$LATEST"
  update_runtime_on = "Auto"
}

resource "aws_lambda_function_recursion_config" "this" {
  function_name  = aws_lambda_function.this.function_name
  recursive_loop = "Terminate"
}

output "lambda_function_arn" {
  description = "ARN of the managed gpt-tg-bot lambda"
  value       = aws_lambda_function.this.arn
}

output "lambda_role_arn" {
  description = "IAM role ARN used by lambda"
  value       = aws_iam_role.lambda.arn
}

output "lambda_function_url" {
  description = "Function URL"
  value       = aws_lambda_function_url.this.function_url
}

output "gpt_rules_bucket_name" {
  description = "S3 bucket used for optional GPT rules upload"
  value       = local.gpt_rules_enabled ? aws_s3_bucket.gpt_rules[0].id : null
}

output "s3_rules_uri" {
  description = "S3 URI set in lambda as S3_RULES_URI when enabled"
  value       = local.gpt_rules_enabled ? local.gpt_rules_s3_uri : null
}
