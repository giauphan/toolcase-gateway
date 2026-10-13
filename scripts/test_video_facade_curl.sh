#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${GW_SMOKE_BASE_URL:-http://127.0.0.1:20129}"

require_json_field() {
  local json="$1"
  local python_expr="$2"
  JSON_BODY="$json" PY_EXPR="$python_expr" python3 - <<'PY'
import json, os, sys
body = json.loads(os.environ["JSON_BODY"])
if not eval(os.environ["PY_EXPR"], {"__builtins__": {}}, {"body": body}):
    print(json.dumps(body, indent=2), file=sys.stderr)
    sys.exit(1)
PY
}

request() {
  local method="$1"
  local path="$2"
  local body="${3:-}"
  local out status payload
  out=$(mktemp)
  if [[ -n "$body" ]]; then
    status=$(curl -sS -o "$out" -w '%{http_code}' \
      -X "$method" \
      -H 'Content-Type: application/json' \
      --data-raw "$body" \
      "$BASE_URL$path")
  else
    status=$(curl -sS -o "$out" -w '%{http_code}' \
      -X "$method" \
      "$BASE_URL$path")
  fi
  payload=$(cat "$out")
  rm -f "$out"
  printf '%s\n%s\n' "$status" "$payload"
}

mapfile -t response < <(request GET '/video/v1/providers')
[[ "${response[0]}" == "200" ]]
require_json_field "${response[1]}" 'body.get("schema_version") == "video.transport.v1" and body.get("providers") == []'

submit_body='{"operation":"text_to_video","model":"unverified-model","prompt":"localhost smoke test","duration_sec":5,"aspect_ratio":"16:9"}'
mapfile -t response < <(request POST '/video/v1/providers/kling_official/tasks' "$submit_body")
[[ "${response[0]}" == "422" ]]
require_json_field "${response[1]}" 'body.get("error", {}).get("code") == "UNSUPPORTED_CAPABILITY"'

mapfile -t response < <(request GET '/video/v1/providers/kling_official/tasks/smoke-task-id')
[[ "${response[0]}" == "404" ]]
require_json_field "${response[1]}" 'body.get("error", {}).get("code") == "NOT_FOUND"'

mapfile -t response < <(request GET '/video/v1/providers/kling_official/tasks/by-key/smoke-idempotency-key')
[[ "${response[0]}" == "501" ]]
require_json_field "${response[1]}" 'body.get("error", {}).get("code") == "RECONCILE_UNSUPPORTED"'

printf 'video facade curl smoke passed against %s\n' "$BASE_URL"
