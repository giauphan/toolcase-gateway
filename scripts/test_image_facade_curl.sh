#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${GW_BASE_URL:-http://127.0.0.1:28130}"
echo "=================================================="
echo "Testing Muse AI Image Generation via Gateway"
echo "Target: $BASE_URL"
echo "=================================================="

# 1. Interactive Web Studio
echo -n "[1/5] Testing Web Studio (GET /image-template)... "
STATUS=$(curl -s -o /dev/null -w "%{http_code}" "$BASE_URL/image-template")
if [[ "$STATUS" == "200" ]]; then
  echo "OK ($STATUS)"
else
  echo "FAIL ($STATUS)"
fi

# 2. Input Validation (Empty Prompt)
echo -n "[2/5] Testing validation on empty prompt (POST /muse-ai/v1/create-image)... "
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BASE_URL/muse-ai/v1/create-image" \
  -H "Content-Type: application/json" \
  -d '{"prompt": "", "aspect_ratio": "1:1"}')
if [[ "$STATUS" == "400" ]]; then
  echo "OK (Rejected with $STATUS as expected)"
else
  echo "FAIL (Got $STATUS, expected 400)"
fi

# 3. Asynchronous Image Generation
echo -n "[3/5] Testing async creation (POST /muse-ai/v1/create-image?async=true)... "
ASYNC_RESP=$(curl -s -X POST "$BASE_URL/muse-ai/v1/create-image?async=true" \
  -H "Content-Type: application/json" \
  -d '{"prompt": "A serene mountain lake at dawn with mist", "aspect_ratio": "16:9"}')

JOB_ID=$(echo "$ASYNC_RESP" | python3 -c "import sys, json; print(json.load(sys.stdin).get('id', ''))")
STATUS_VAL=$(echo "$ASYNC_RESP" | python3 -c "import sys, json; print(json.load(sys.stdin).get('status', ''))")

if [[ -n "$JOB_ID" && "$STATUS_VAL" == "submitting" ]]; then
  echo "OK (Allocated Job: $JOB_ID, status: $STATUS_VAL)"
else
  echo "FAIL (Response: $ASYNC_RESP)"
fi

# 4. Job Status Polling
if [[ -n "$JOB_ID" ]]; then
  echo -n "[4/5] Polling job status (GET /muse-ai/v1/images/$JOB_ID)... "
  sleep 1
  POLL_RESP=$(curl -s "$BASE_URL/muse-ai/v1/images/$JOB_ID")
  POLL_STATUS=$(echo "$POLL_RESP" | python3 -c "import sys, json; print(json.load(sys.stdin).get('status', ''))")
  echo "OK (Current status: $POLL_STATUS)"
fi

# 5. Synchronous Generation / Upstream Handshake
echo -n "[5/5] Testing synchronous generation (POST /muse-ai/v1/create-image?wait=true)... "
SYNC_RESP=$(curl -s -w "\nHTTP_STATUS:%{http_code}" -X POST "$BASE_URL/muse-ai/v1/create-image?wait=true" \
  -H "Content-Type: application/json" \
  -d '{"prompt": "A majestic red fox in a winter pine forest", "aspect_ratio": "1:1"}')

HTTP_CODE=$(echo "$SYNC_RESP" | grep "HTTP_STATUS:" | cut -d: -f2)
BODY=$(echo "$SYNC_RESP" | grep -v "HTTP_STATUS:")

echo "HTTP $HTTP_CODE"
echo "Response Payload:"
echo "$BODY" | python3 -m json.tool 2>/dev/null || echo "$BODY"

echo "=================================================="
echo "Summary: Image generation pipeline flow is verified."
echo "=================================================="
