#!/usr/bin/env bash
set -euo pipefail

if [[ -f .env ]]; then
  set -a
  # shellcheck disable=SC1091
  source .env
  set +a
fi

: "${GW_JEV_API_KEYS:?GW_JEV_API_KEYS must be set in environment or .env}"
GW_LISTEN_HOST="${GW_LISTEN_HOST:-127.0.0.1}"
GW_LISTEN_PORT="${GW_LISTEN_PORT:-28130}"
GW_SCHEME="${GW_SCHEME:-http}"
GATEWAY_URL="${GW_SCHEME}://${GW_LISTEN_HOST}:${GW_LISTEN_PORT}/jev/v1/systemone"
IDEMPOTENCY_KEY="live-jev-gateway-curl-smoke-$(date +%s%N)"

body='{
  "state": "A customer reports a damaged package and asks for a refund.",
  "questions": {
    "refund": {
      "type": "noul",
      "instructions": "Is the customer requesting a refund?"
    }
  }
}'

http_code=$(
  curl --silent --show-error --output /tmp/jev-gateway-smoke-response.json --write-out '%{http_code}' \
    --request POST "${GATEWAY_URL}" \
    --header 'Content-Type: application/json' \
    --header "Idempotency-Key: ${IDEMPOTENCY_KEY}" \
    --data "${body}"
)

if [[ "${http_code}" != 2* ]]; then
  printf 'Jev gateway smoke failed with status code %s\n' "${http_code}" >&2
  exit 1
fi

python3 - <<'PYJSON'
import json
from pathlib import Path
path = Path('/tmp/jev-gateway-smoke-response.json')
try:
    data = json.loads(path.read_text())
except Exception as exc:
    raise SystemExit(f'Jev gateway smoke response was not JSON: {exc}')
if not isinstance(data, dict):
    raise SystemExit('Jev gateway smoke response was not a JSON object')
print('Jev gateway smoke passed')
PYJSON
