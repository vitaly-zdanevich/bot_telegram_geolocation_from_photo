data "aws_iam_policy_document" "lambda_assume_role" {
  statement {
    actions = ["sts:AssumeRole"]

    principals {
      type        = "Service"
      identifiers = ["lambda.amazonaws.com"]
    }
  }
}

locals {
  user_agent = "${var.project_name}/0.1 (${var.github_url})"
  wikipedia_language_list_raw = length(var.wikipedia_languages) > 0 ? var.wikipedia_languages : compact([
    for language in split(",", var.wikipedia_language) : trimspace(language)
  ])
  wikipedia_language_list = length(local.wikipedia_language_list_raw) > 0 ? local.wikipedia_language_list_raw : ["en", "ru", "be"]

  raw_environment_variables = {
    PROJECT_NAME              = var.project_name
    TELEGRAM_BOT_TOKEN        = var.telegram_bot_token
    TELEGRAM_WEBHOOK_SECRET   = var.telegram_webhook_secret
    GITHUB_URL                = var.github_url
    MAX_FILE_MB               = tostring(var.max_file_mb)
    ENABLE_REVERSE_GEOCODING  = tostring(var.enable_reverse_geocoding)
    NOMINATIM_BASE_URL        = var.nominatim_base_url
    NOMINATIM_USER_AGENT      = local.user_agent
    NOMINATIM_EMAIL           = var.nominatim_email
    NOMINATIM_ACCEPT_LANGUAGE = var.nominatim_accept_language
    ENABLE_WIKIMEDIA_LOOKUP   = tostring(var.enable_wikimedia_lookup)
    WIKIMEDIA_USER_AGENT      = local.user_agent
    WIKIPEDIA_LANGUAGE        = local.wikipedia_language_list[0]
    WIKIPEDIA_LANGUAGES       = join(",", local.wikipedia_language_list)
    WIKIPEDIA_API_URL         = var.wikipedia_api_url
    WIKIDATA_SPARQL_URL       = var.wikidata_sparql_url
    WIKIMEDIA_RADIUS_METERS   = tostring(var.wikimedia_radius_meters)
    WIKIMEDIA_LIMIT           = tostring(var.wikimedia_limit)
    RUST_LOG                  = "info"
  }

  environment_variables = {
    for key, value in local.raw_environment_variables : key => value
    if value != null && value != ""
  }
}

resource "aws_iam_role" "lambda" {
  name               = var.project_name
  assume_role_policy = data.aws_iam_policy_document.lambda_assume_role.json
}

resource "aws_iam_role_policy_attachment" "lambda_basic" {
  role       = aws_iam_role.lambda.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole"
}

resource "aws_cloudwatch_log_group" "lambda" {
  name              = "/aws/lambda/${var.project_name}"
  retention_in_days = 14
}

resource "aws_lambda_function" "bot" {
  function_name = var.project_name
  role          = aws_iam_role.lambda.arn
  filename      = var.lambda_zip_path

  package_type  = "Zip"
  architectures = ["arm64"]
  runtime       = "provided.al2023"
  handler       = "bootstrap"

  memory_size                    = var.lambda_memory_size
  timeout                        = var.lambda_timeout_seconds
  reserved_concurrent_executions = var.reserved_concurrent_executions
  source_code_hash               = filebase64sha256(var.lambda_zip_path)

  ephemeral_storage {
    size = 512
  }

  environment {
    variables = local.environment_variables
  }

  depends_on = [
    aws_cloudwatch_log_group.lambda,
    aws_iam_role_policy_attachment.lambda_basic
  ]
}

resource "aws_lambda_function_url" "bot" {
  function_name      = aws_lambda_function.bot.function_name
  authorization_type = "NONE"
}

resource "aws_lambda_permission" "function_url" {
  statement_id           = "AllowFunctionUrlInvoke"
  action                 = "lambda:InvokeFunctionUrl"
  function_name          = aws_lambda_function.bot.function_name
  principal              = "*"
  function_url_auth_type = "NONE"

  depends_on = [aws_lambda_function_url.bot]
}
