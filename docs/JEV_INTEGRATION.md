# Jev AI Integration & Multi-Account Guide

`toolcase-gateway` provides built-in support for proxying Jev AI OpenAI-compatible endpoints (`/jev/v1/*` and `/jev-ai/v1/*`) with multi-account rotation and failover.

---

## Credit & Platform Reference

| Platform | Free Allowance / Credits | Usage / Referral Link |
| --- | --- | --- |
| **Jev AI Tools** | 3 free uses without account; login gives up to **300 credits/month** (capped at **30/day**) | [Try Jev AI Tools](https://jev.ai) |
| **Jev AI Space** | **200 free credits** one-time upon registration | [Get Jev Space Credits](https://jev.ai) |
| **Jev AI Playground** | **5 welcome credits** + daily claimable credits mechanism | [Try Jev Playground](https://jev.ai) |

---

## Multi-Account Configuration

To aggregate multiple Jev AI account keys for maximum daily throughput and automatic failover on 401 / 402 / 429 errors:

Set the comma-separated `GW_JEV_API_KEYS` environment variable:

```sh
GW_JEV_API_KEYS="sk-jev-account1,sk-jev-account2,sk-jev-account3"
```

The gateway automatically round-robins across all configured account keys on each request, and transparently fails over to the next active account if an account exhausts its daily credit cap (HTTP 429) or requires re-authentication (HTTP 401/402/403).

Per-request authorization headers (`Authorization: Bearer <key>`) are also supported when `GW_JEV_API_KEYS` is omitted.

---

## Integrating with AI Agent CLIs & IDEs

The gateway exposes standard OpenAI-compatible endpoints at:
- `http://127.0.0.1:20129/jev/v1`
- `http://127.0.0.1:20129/jev-ai/v1`

### 1. Claude Code CLI

Set standard OpenAI environment variables before running `claude`:

```sh
export OPENAI_BASE_URL="http://127.0.0.1:20129/jev/v1"
export OPENAI_API_KEY="sk-jev-gateway" # or set GW_JEV_API_KEYS in gateway env
claude
```

### 2. Codex CLI

Configure OpenAI provider endpoint in Codex settings or shell:

```sh
export OPENAI_BASE_URL="http://127.0.0.1:20129/jev/v1"
export OPENAI_API_KEY="sk-jev-gateway"
codex
```

### 3. VS Code / Cursor / JetBrains Extensions (Continue, Cline, Roo Code)

In your AI plugin configuration (`~/.continue/config.json` or plugin settings):

```json
{
  "models": [
    {
      "title": "Jev AI (Multi-Account)",
      "provider": "openai",
      "model": "gpt-4o-mini",
      "apiBase": "http://127.0.0.1:20129/jev/v1",
      "apiKey": "sk-jev-gateway"
    }
  ]
}
```

---

## API Endpoints

| Endpoint | Method | Description |
| --- | --- | --- |
| `/jev/v1/chat/completions` | `POST` | OpenAI-compatible chat completion route |
| `/jev-ai/v1/chat/completions` | `POST` | Dedicated alias route |
| `/jev/v1/models` | `GET` | Models catalog |
