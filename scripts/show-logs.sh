#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STATE_FILE="$ROOT_DIR/infra/terraform.tfstate"
FUNCTION_NAME="${1:-${LAMBDA_FUNCTION_NAME:-}}"
MINUTES="${LOG_MINUTES:-30}"
STATE_REGION=""
STATE_ACCOUNT=""

if [[ -f "$STATE_FILE" ]] && command -v jq >/dev/null 2>&1; then
	STATE_LAMBDA_ARN="$(
		jq -r '
			.resources[]
			| select(.type == "aws_lambda_function" and .name == "bot")
			| .instances[0].attributes.arn // empty
		' "$STATE_FILE"
	)"

	if [[ -n "$STATE_LAMBDA_ARN" ]]; then
		STATE_REGION="$(printf '%s' "$STATE_LAMBDA_ARN" | cut -d: -f4)"
		STATE_ACCOUNT="$(printf '%s' "$STATE_LAMBDA_ARN" | cut -d: -f5)"
	fi
fi

REGION="${AWS_REGION:-${STATE_REGION:-${AWS_DEFAULT_REGION:-us-east-1}}}"

if [[ -z "$FUNCTION_NAME" ]]; then
	FUNCTION_NAME="$(terraform -chdir="$ROOT_DIR/infra" output -raw function_name 2>/dev/null || true)"
fi

if [[ -z "$FUNCTION_NAME" && -f "$STATE_FILE" ]] && command -v jq >/dev/null 2>&1; then
	FUNCTION_NAME="$(
		jq -r '
			.resources[]
			| select(.type == "aws_lambda_function" and .name == "bot")
			| .instances[0].attributes.function_name // empty
		' "$STATE_FILE"
	)"
fi

if [[ -z "$FUNCTION_NAME" ]]; then
	FUNCTION_NAME="telegram-photo-geolocator-bot"
fi

LOG_GROUP="${LOG_GROUP_NAME:-}"
if [[ -z "$LOG_GROUP" ]]; then
	LOG_GROUP="/aws/lambda/$FUNCTION_NAME"
fi

START_TIME="$(( ($(date +%s) - MINUTES * 60) * 1000 ))"

echo "Reading $LOG_GROUP in $REGION${STATE_ACCOUNT:+, Terraform account $STATE_ACCOUNT}" >&2

if ! aws logs filter-log-events \
	--region "$REGION" \
	--log-group-name "$LOG_GROUP" \
	--start-time "$START_TIME" \
	--interleaved \
	--query 'events[*].message' \
	--output text; then
	echo >&2
	echo "Log read failed. Diagnostics:" >&2
	aws sts get-caller-identity --output table >&2 || true
	echo >&2
	echo "Lambda log groups visible in $REGION:" >&2
	aws logs describe-log-groups \
		--region "$REGION" \
		--log-group-name-prefix "/aws/lambda/" \
		--query 'logGroups[*].logGroupName' \
		--output table >&2 || true
	exit 1
fi
