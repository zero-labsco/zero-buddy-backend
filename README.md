<p align="center">
  <img src="https://img.shields.io/badge/Zero%20Buddy-Backend-5eead4?style=for-the-badge" alt="Zero Buddy Backend" />
  <img src="https://img.shields.io/badge/Rust-000?style=for-the-badge&logo=rust&logoColor=white" alt="Rust" />
  <img src="https://img.shields.io/badge/Axum-Web%20Framework-dea584?style=for-the-badge&logo=rust&logoColor=white" alt="Axum" />
  <img src="https://img.shields.io/badge/License-Apache--2.0-blue?style=for-the-badge" alt="License" />
</p>

<h1 align="center">Zero Buddy Backend</h1>

<p align="center">
  The AI assistant backend for Zero Labs — answering questions about the Zero Labs.
</p>

<p align="center">
  <a href="https://github.com/zero-labsco/zero-buddy-backend">Backend Repo</a>
  ·
  <a href="https://github.com/zero-labsco/zero-buddy-frontend">Frontend Repo</a>
  ·
  <a href="./CONTRIBUTING.md">Contributing</a>
</p>

---

> **Part of the Zero Buddy project.** This repository is the **backend** (Rust /
> Axum API). The chat UI lives in a separate repo:
> [**Zero Buddy Frontend »**](https://github.com/zero-labsco/zero-buddy-frontend)

## Table of Contents

- [Introduction](#introduction)
- [Features](#features)
- [Tech Stack](#tech-stack)
- [Project Structure](#project-structure)
- [Quick Start](#quick-start)
- [Response Format](#response-format)
- [Configuration](#configuration)
- [Contributing & CI](#contributing--ci)

## Introduction

A standalone Rust (Axum) service that powers the Zero Buddy chat assistant. It
uses a curated knowledge base about Zero Labs' projects with a lightweight **RAG
(retrieval-augmented generation)** pipeline and can call any OpenAI-compatible
LLM (OpenAI / DeepSeek / Qwen / Claude, …).

The code is organized into clear modules (`config`, `llm`, `knowledge`, `rag`,
`chat`, `cache`, `faq`) so it can later be split into independent microservices
without changing call signatures.

## Features

- **Chat API** (`POST /api/chat`) — conversational answers grounded in the
  Zero Labs knowledge base. Responses carry a `source` field
  (`faq` / `cache` / `llm` / `offline`) so the frontend knows where an answer
  came from.
- **Unified response envelope** — every response is
  `{ "code": 200, "message": "success", "body": { ... } }`. `code` comes from
  the `ApiCode` enum; `message` defaults to the enum text but can be overridden
  per call (e.g. `"too many messages (max 50)"`); `body` holds business data and
  is `null` on error.
- **RAG retrieval** — cosine-similarity retrieval over embeddings, with a
  **keyword fallback** so it still works without an API key.
- **Reply language policy** — answers follow the user's input language:
  **Chinese → Chinese, English → English, any other language → English**.
  - Online: enforced by the LLM system prompt (both simple and RAG paths).
  - Offline: `faq.json` / `knowledge.json` ship **bilingual (EN + ZH)** entries,
    so the same question in either language hits the matching entry.
- **Scope limit** — the assistant only answers questions related to Zero Labs /
  its products (features, usage, APIs, deployment, knowledge base). Off-topic
  questions are politely refused to avoid wasting tokens.
- **Answer cache** — two layers to save LLM tokens:
  - **Exact hit**: normalized query as key, stored in `data/answers_cache.json`
    (works **offline too** — see `put_offline` below, persists EN/ZH Q&A).
  - **Semantic neighbor**: query embedding compared against cached items
    (cosine ≥ `CACHE_SIMILARITY`, default `0.92`) to cover rephrased questions.
  - Bumping `CACHE_VERSION` invalidates the whole cache.
- **Offline mode** — without a valid `LLM_API_KEY`, the API returns retrieved
  knowledge snippets instead of failing. FAQ + knowledge hits are cached offline
  (exact-match, no embedding needed). When nothing matches, the reply gracefully
  asks the user to **"please ask in Chinese or English"** (in EN or ZH). The
  frontend does **not** surface an ONLINE/OFFLINE label — only a status dot.
- **Scope Guard (topic restriction)** — rejects questions unrelated to Zero Labs
  products to avoid wasting LLM tokens and misuse as a general AI. Configurable
  via `SCOPE_GUARD_ENABLED`, `SCOPE_GUARD_MODE` (`prompt` | `hard`),
  `SCOPE_ALLOW_KEYWORDS` (substring whitelist), `SCOPE_ALLOW_PATTERNS` (regex
  whitelist, e.g. product names, `zerolabsco.com`, GitHub repo), and
  `SCOPE_REFUSE_REPLY_ZH` / `SCOPE_REFUSE_REPLY_EN`. See [Configuration](#configuration).
- **Rate limiting (abuse / cost control)** — per-client-IP limits on requests
  per minute and per day (`RATE_LIMIT_PER_MIN`, `RATE_LIMIT_PER_DAY`), with
  configurable ZH/EN messages. In-memory, single-instance; use Redis for
  multi-instance. Exceeded requests return a `rate-limit` source reply.
- **OpenAI-compatible** — point `LLM_BASE_URL` at any compatible provider.
- **Hardening** — reused `reqwest::Client`, body size limit (1 MB), global
  timeout, CORS restricted to the frontend origin, and structured logging
  (`tracing` to both console and rotated `logs/` files).
- **Health check** (`GET /health`).

## Tech Stack

| Concern       | Choice                              |
| ------------- | ----------------------------------- |
| Web framework | `axum`                              |
| HTTP client   | `reqwest` (shared client, timeouts) |
| Serialization | `serde` / `serde_json`              |
| Embeddings    | Remote embedding API                |
| Config        | Environment variables (`.env`)      |
| Logging       | `tracing` / `tracing-subscriber`    |

## Project Structure

```
backend/
├── Cargo.toml
├── .env.example
├── data/
│   ├── knowledge.json        # Curated Zero Labs knowledge base
│   ├── faq.json              # FAQ rules (zero-token answers)
│   └── answers_cache.json    # Auto-generated answer cache (gitignored)
└── src/
    ├── main.rs               # Server bootstrap, routing, middleware
    ├── config/               # Env config + has_valid_key()
    ├── models/               # Request / response / document types
    ├── llm/                  # OpenAI-compatible chat & embeddings (shared client)
    ├── retrieval/            # Knowledge load + RAG + cache + faq
    ├── chat/                 # Orchestrates FAQ → cache → RAG → LLM
    ├── rate_limit/           # Per-IP rate limiting
    ├── logging/              # Tracing setup
    └── response/             # Unified ApiCode / ApiError / ApiResult envelope
```

## Quick Start

```bash
# 1. Install Rust (https://rustup.rs) if you haven't

# 2. Configure environment
cp .env.example .env
#    Set LLM_BASE_URL and LLM_API_KEY (any OpenAI-compatible provider).
#    Leaving LLM_API_KEY empty starts the service in offline mode.

# 3. Run
cargo run
#    Server listens on http://127.0.0.1:3030 (falls back to 3031 if busy)

# 4. Health check
curl http://127.0.0.1:3030/health

# 5. Try a chat (response is wrapped in the uniform envelope below)
curl -s -X POST http://127.0.0.1:3030/api/chat \
  -H 'Content-Type: application/json' \
  -d '{"messages":[{"role":"user","content":"What is Zero Buddy?"}]}'
```

## Response Format

Every response (success or error) uses the same envelope:

```json
{ "code": 200, "message": "success", "body": { "reply": "...", "source": "llm", "url": "https://zerolabsco.com" } }
```

- `code` — comes from the `ApiCode` enum (`200`, `400`, `401`, `404`, `500`…).
- `message` — defaults to the enum's text; can be overridden per call, e.g.
  `{ "code": 400, "message": "too many messages (max 50)", "body": null }`.
- `body` — the business payload; `null` on error.
  - `reply` — the answer text.
  - `source` — how the answer was produced (`faq` / `cache` / `llm` / `offline`).
  - `url` — optional link carried from the matched knowledge document (e.g. official
    website or `mailto:` email); `undefined` when no document link applies. The
    frontend renders it as a clickable link after typing finishes.

## Configuration (`.env`)

| Variable              | Default                       | Description                                                        |
| --------------------- | ----------------------------- | ------------------------------------------------------------------ |
| `BIND_ADDR`           | `127.0.0.1:3030`              | HTTP listen address (falls back to `127.0.0.1:3031` if busy)       |
| `LLM_BASE_URL`        | `https://api.openai.com/v1`   | OpenAI-compatible base URL                                         |
| `LLM_API_KEY`         | _(empty = offline mode)_      | API key for the LLM provider                                       |
| `LLM_MODEL`           | `gpt-4o-mini`                 | Chat model name                                                    |
| `EMBED_MODEL`         | `text-embedding-3-small`      | Embedding model name                                               |
| `CORS_ORIGIN`         | `http://localhost:3040`       | Allowed frontend origin(s), comma-separated                        |
| `CACHE_VERSION`       | `v1`                          | Bump to invalidate the whole cache                                 |
| `REQUEST_TIMEOUT_SECS`| `30`                          | Per-LLM-call timeout (seconds)                                     |
| `PRODUCT_NAME`        | `ZeroBuddy`                   | Assistant/product name (prompts+logs)                              |
| `ORG_NAME`            | `Zero Labs`                   | Org name (prompts)                                                 |
| `RAG_TOP_K`           | `3`                           | Max docs fed to the LLM                                            |
| `RAG_MIN_SCORE`       | `0.2`                         | Min similarity to include a doc                                    |
| `CACHE_SIMILARITY`    | `0.92`                        | Semantic cache neighbor threshold                                  |
| `MAX_MESSAGE_CHARS`   | `4000`                        | Max chars per single message                                       |
| `MAX_MESSAGES`        | `50`                          | Max messages per request                                           |
| `SCOPE_GUARD_ENABLED` | `true`                        | Enable topic-scope restriction (Scope Guard)                       |
| `SCOPE_GUARD_MODE`    | `prompt`                      | `prompt` = LLM self-enforces scope; `hard` = backend blocks off-topic before any LLM call (saves tokens) |
| `SCOPE_ALLOW_KEYWORDS`| _(see `.env.example`)_        | Comma-separated substring whitelist; a hit passes the query through. Empty = allow all |
| `SCOPE_ALLOW_PATTERNS`| _(see `.env.example`)_        | `\|`-separated regex whitelist (product names, `zerolabsco.com`, GitHub repo…). Empty = none |
| `SCOPE_REFUSE_REPLY_ZH`| _(built-in ZH refusal)_      | Reply returned when a Chinese off-topic query is blocked           |
| `SCOPE_REFUSE_REPLY_EN`| _(built-in EN refusal)_      | Reply returned when an English off-topic query is blocked          |
| `RATE_LIMIT_PER_MIN`  | `10`                          | Max requests per client IP per minute (0 = unlimited)              |
| `RATE_LIMIT_PER_DAY`  | `500`                         | Max requests per client IP per day (0 = unlimited)                 |
| `RATE_LIMIT_REPLY_ZH` | _(built-in ZH message)_       | Reply returned when a Chinese client is rate-limited               |
| `RATE_LIMIT_REPLY_EN` | _(built-in EN message)_       | Reply returned when an English client is rate-limited              |

> Without a valid key the service still starts and the chat returns retrieved
> knowledge in **offline mode** — useful for demos and testing the RAG pipeline.

### Notes

- `data/answers_cache.json` is built on the fly as users ask questions; delete it
  (or bump `CACHE_VERSION`) to rebuild. It is git-ignored.
- `Cargo.lock` is committed for reproducible builds (recommended for binaries).
- Logs are emitted via `tracing`. By default they go to **both the console and
  `backend/logs/app.YYYY-MM-DD.log`** (daily rotation, under the backend crate root). Set `RUST_LOG=debug` for verbose output.

---

## Contributing & CI

Full contribution guidelines (English + 简体中文) are in **[CONTRIBUTING.md](./CONTRIBUTING.md)**,
following the [Zero Labs contributing guidelines](https://github.com/zero-labsco/.github/blob/main/profile/CONTRIBUTING.md):

- **Commit messages** must follow [Conventional Commits](https://www.conventionalcommits.org)
  (e.g. `feat:`, `fix:`, `docs:`, `chore:`, `refactor:`, `ci:` …).
  PR commits are enforced by `wagoid/commitlint-github-action` (see `commitlint.config.js`).
- **Branch naming**: use a descriptive prefix, e.g. `feature/your-feature-name`.
- **Code style**: Rust uses `rustfmt` (Rust Style Guide). Run `cargo fmt` before pushing.
- **Pre-commit hook**: this repo ships a `cargo fmt` check in `.githooks/pre-commit`.
  Enable it once after cloning:

  ```bash
  git config core.hooksPath .githooks
  ```

  It rejects commits whose formatting does not pass `cargo fmt --all -- --check`.

### CI workflow (`.github/workflows/ci.yml`)

Runs on every push to `main`/`master` and on every PR:

1. `cargo fmt --all -- --check` — formatting check.
2. `cargo clippy --all-targets --all-features -- -D warnings` — lint, warnings treated as errors.
3. `cargo build --verbose` — compiles the service.
4. `cargo test --verbose` — runs unit tests.
5. **Commit Message Lint** (PR only) — validates each commit against Conventional Commits.

### Dependabot (`.github/dependabot.yml`)

- `cargo`: weekly dependency updates, `chore:` commit prefix.
- `github-actions`: weekly workflow updates, `ci:` commit prefix.
- **Security-first**: all updates must pass the audit / dependency-review gates below before merge; Dependabot never auto-merges.

### Dependency Audit & Auto-Fix (`.github/workflows/audit.yml`)

- Runs **every Monday 09:30 UTC** and is also manually triggerable (`workflow_dispatch`).
- **Security is a hard gate**: runs `cargo audit --deny warnings`. If RUSTSEC vulnerabilities are found,
  the job **fails (red)** on purpose (status red) until the tree is clean — an unsafe version is never
  accepted "just because it's latest" — and opens a report PR titled
  `chore(deps): address cargo audit vulnerabilities` for human review.
- The PR is **never auto-merged** and does not auto-edit `Cargo.toml` — a maintainer must
  bump the affected crate versions and run `cargo update` before merging.

### Dependency Review Gate (`.github/workflows/dependency-review.yml`)

- Runs on **every PR to `main`**. Blocks any change that introduces high/critical vulnerabilities
  (covers Dependabot's "latest version" PRs too). Ensures **secure-first, then latest**.

---

### 中文文档

中文说明请见 [`README_ZH.md`](./README_ZH.md)。
