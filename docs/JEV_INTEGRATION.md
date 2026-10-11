# Jev Decision API

Jev is a decision service, not an OpenAI-compatible chat API. Send `state` and `questions` to the gateway; it forwards the JSON unchanged to Jev and returns Jev's JSON response.

## Configuration

Set `GW_JEV_API_KEYS` to one key or a comma-separated list. Keys rotate between requests and fail over to the next key on account errors.

```sh
GW_JEV_API_KEYS="<key-1>,<key-2>"
GW_JEV_BASE_URL="https://jev-ai.org"
```

`GW_JEV_BASE_URL` defaults to `https://jev-ai.org`. Set it to another `http://` or `https://` base URL for a test or alternate deployment. When `GW_JEV_API_KEYS` is empty, requests can supply `Authorization: Bearer <key>` or `x-api-key: <key>`. Keep keys out of public clients and source control.

## Routes

| Gateway route | Method | Jev upstream route |
| --- | --- | --- |
| `/jev/v1/systemone` | POST | `/api/v1/systemone/` |
| `/jev/v1/models` | GET | `/api/v1/models/` |
| `/jev/v1/requests/{id}` | GET | `/api/v1/requests/{id}/` |
| `/jev/v1/requests/{id}/cancel` | POST | `/api/v1/requests/{id}/cancel/` |

The `/jev-ai/v1/` prefix is an alias. Canonical upstream paths include trailing slashes. `/jev/v1/chat/completions` is not a chat endpoint; use `/jev/v1/systemone` with Jev's decision schema instead.

## Example

```sh
curl -X POST "http://127.0.0.1:20129/jev/v1/systemone" \
  -H "Content-Type: application/json" \
  -H "Idempotency-Key: ticket-8814-20261010" \
  -d '{
    "model": "jev-1.13",
    "state": "A customer wants a refund for a damaged product.",
    "questions": {
      "refund": {
        "type": "noul",
        "instructions": "Is the customer requesting a refund?"
      }
    }
  }'
```

When no key is configured on the gateway, add `-H "Authorization: Bearer <key>"` to the request. Do not use a dummy token with configured keys: gateway uses its configured account pool. The optional `Idempotency-Key` is forwarded unchanged across retries; use the same key for the same logical request.

Account errors `401`, `402`, `403`, and `429` trigger failover when another key is available. Jev marks `429`, `499`, `502`, `503`, and `504` retryable; malformed input (`400`, `404`, `413`, `422`) is returned without account rotation. Jev bills only successful `200` decisions; models, status lookups, and cancellation are free. See [the Jev API guide](../target/jev_ai_api_integration_guide_en.md) for schema, rate limits, and error codes.
