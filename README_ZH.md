<p align="center">
  <img src="https://img.shields.io/badge/Zero%20Buddy-%E5%90%8E%E7%AB%AF%20Backend-5eead4?style=for-the-badge" alt="Zero Buddy 后端" />
  <img src="https://img.shields.io/badge/Rust-000?style=for-the-badge&logo=rust&logoColor=white" alt="Rust" />
  <img src="https://img.shields.io/badge/Axum-Web%20Framework-dea584?style=for-the-badge&logo=rust&logoColor=white" alt="Axum" />
  <img src="https://img.shields.io/badge/License-Apache--2.0-blue?style=for-the-badge" alt="License" />
</p>

<h1 align="center">Zero Buddy 后端 (Backend)</h1>

<p align="center">
  Zero Labs 的 AI 助手后端，回答关于 Zero Labs 的问题。
</p>

<p align="center">
  <a href="https://github.com/zero-labsco/zero-buddy-backend">后端仓库</a>
  ·
  <a href="https://github.com/zero-labsco/zero-buddy-frontend">前端仓库</a>
  ·
  <a href="./CONTRIBUTING.md">贡献指南</a>
</p>

---

> **Zero Buddy 项目的一部分。** 本仓库是**后端**（Rust / Axum API）。聊天界面
> 位于另一个独立仓库：**[Zero Buddy 前端 »](https://github.com/zero-labsco/zero-buddy-frontend)**

## 目录

- [简介](#简介)
- [功能特性](#功能特性)
- [技术栈](#技术栈)
- [项目结构](#项目结构)
- [快速开始](#快速开始)
- [响应格式](#响应格式)
- [配置项](#配置项)
- [贡献与 CI](#贡献与-ci)

## 简介

一个独立的 Rust（Axum）服务，为 Zero Buddy 聊天助手提供能力。它基于精选的
Zero Labs 项目知识库，使用轻量的 **RAG（检索增强生成）** 流程，并可调用任意
OpenAI 兼容的大模型（OpenAI / DeepSeek / Qwen / Claude 等）。

代码按清晰的模块划分（`config`、`llm`、`knowledge`、`rag`、`chat`、`cache`、`faq`），
未来无需改写调用签名即可平滑拆分为独立的微服务。

## 功能特性

- **聊天接口**（`POST /api/chat`）——基于 Zero Labs 知识库的对话式回答。
  响应带 `source` 字段（`faq` / `cache` / `llm` / `offline`），前端可知答案来源。
- **统一响应信封**——所有响应统一为
  `{ "code": 200, "message": "success", "body": { ... } }`。`code` 来自 `ApiCode`
  枚举；`message` 默认取枚举文案，也可在调用处覆盖（如 `"too many messages (max 50)"`）；
  `body` 承载业务数据，出错时为 `null`。
- **RAG 检索**——基于 embedding 的余弦相似度检索，并带有**关键词兜底**，
  即使没有 API key 也能发挥作用。
- **回复语言策略**——答案跟随用户输入语言：**中文→中文、英文→英文、其他语言→英文**。
  - 在线：由 LLM 系统提示强制（简单问答与 RAG 两条路径均如此）。
  - 离线：`faq.json` / `knowledge.json` 提供**中英文双语**条目，中英提问各命中对应条目。
- **范围限制**——助手只回答与 Zero Labs / 产品相关的问题（功能、用法、API、部署、知识库），
  范围外的问题友好拒绝，避免浪费 token。
- **答案缓存**——两层缓存，省 LLM token：
  - **精确命中**：归一化 query 作为 key，存于 `data/answers_cache.json`
    （**离线也写缓存**——见 `put_offline`，中英文问答均持久化）。
  - **语义近邻**：query embedding 与缓存项对比（余弦 ≥ `CACHE_SIMILARITY`，默认 `0.92`），覆盖换种问法。
  - `CACHE_VERSION` 变更时整体失效。
- **离线模式**——未配置有效 `LLM_API_KEY` 时，接口直接返回检索到的知识片段，
  而不是报错。FAQ 命中与知识库命中的答案都会**离线写入缓存**（精确匹配，无需 embedding）；
  当完全无命中时，会优雅提示用户**「请使用中文或英文提问」**（按语言返回中/英文）。
  前端**不显示** ONLINE/OFFLINE 文字，仅保留一个状态点。
- **范围守卫（主题限制）**——拒绝与 Zero Labs 产品无关的问题，避免浪费 token、防止被当作通用 AI。
  可通过 `SCOPE_GUARD_ENABLED`、`SCOPE_GUARD_MODE`（`prompt` | `hard`）、
  `SCOPE_ALLOW_KEYWORDS`（子串白名单）、`SCOPE_ALLOW_PATTERNS`（正则白名单，如产品名、`zerolabsco.com`、GitHub 仓库）、
  `SCOPE_REFUSE_REPLY_ZH` / `SCOPE_REFUSE_REPLY_EN` 配置。详见[配置项](#配置项)。
- **限流（防滥用 / 控成本）**——按客户端 IP 限制每分钟与每日请求数
  （`RATE_LIMIT_PER_MIN`、`RATE_LIMIT_PER_DAY`），中英文提示可配置。内存级、单实例；
  多实例请用 Redis。超限请求返回 `rate-limit` 来源回复。
- **OpenAI 兼容**——将 `LLM_BASE_URL` 指向任意兼容的供应商即可。
- **加固**——复用 `reqwest::Client`、请求体大小限制（1 MB）、全局超时、CORS
  收敛到前端来源、结构化日志（`tracing`，同时写 `logs/` 滚动文件）。
- **健康检查**（`GET /health`）。

## 技术栈

| 关注点     | 选型                                  |
| ---------- | ------------------------------------- |
| Web 框架   | `axum`                                |
| HTTP 客户端| `reqwest`（复用 client、超时）         |
| 序列化     | `serde` / `serde_json`                |
| Embedding  | 远程 embedding API                    |
| 配置       | 环境变量（`.env`）                    |
| 日志       | `tracing` / `tracing-subscriber`     |

## 项目结构

```
backend/
├── Cargo.toml
├── .env.example
├── data/
│   ├── knowledge.json        # Zero Labs 精选知识库
│   ├── faq.json              # FAQ 规则（零 token 回答）
│   └── answers_cache.json    # 自动生成的答案缓存（已 gitignore）
└── src/
    ├── main.rs               # 服务启动、路由与中间件
    ├── config/               # 环境配置 + has_valid_key()
    ├── models/               # 请求 / 响应 / 文档类型
    ├── llm/                  # OpenAI 兼容的对话与 embedding（复用 client）
    ├── retrieval/            # 知识加载 + RAG + 缓存 + FAQ
    ├── chat/                 # 编排 FAQ → 缓存 → RAG → LLM
    ├── rate_limit/           # 按 IP 限流
    ├── logging/              # tracing 配置
    └── response/             # 统一的 ApiCode / ApiError / ApiResult 信封
```

## 快速开始

```bash
# 1. 安装 Rust（https://rustup.rs）（如尚未安装）

# 2. 配置环境变量
cp .env.example .env
#    设置 LLM_BASE_URL 与 LLM_API_KEY（任意 OpenAI 兼容供应商）。
#    留空 LLM_API_KEY 即以离线模式启动。

# 3. 运行
cargo run
#    服务监听 http://127.0.0.1:3030（端口被占用时回退到 3031）

# 4. 健康检查
curl http://127.0.0.1:3030/health

# 5. 试聊一句（响应均为下方统一信封格式）
curl -s -X POST http://127.0.0.1:3030/api/chat \
  -H 'Content-Type: application/json' \
  -d '{"messages":[{"role":"user","content":"What is Zero Buddy?"}]}'
```

## 响应格式

所有响应（无论成功或失败）都使用同一信封：

```json
{ "code": 200, "message": "success", "body": { "reply": "...", "source": "llm", "url": "https://zerolabsco.com" } }
```

- `code` — 来自 `ApiCode` 枚举（`200`、`400`、`401`、`404`、`500`…）。
- `message` — 默认取枚举文案，可在调用处覆盖，例如
  `{ "code": 400, "message": "too many messages (max 50)", "body": null }`。
- `body` — 业务负载；出错时为 `null`。
  - `reply` — 答案文本。
  - `source` — 答案来源（`faq` / `cache` / `llm` / `offline`）。
  - `url` — 命中文档携带的可选链接（如官网或 `mailto:` 邮箱）；无适用链接时为 `undefined`。前端会在打字结束后将其渲染为可点击链接。

## 配置项（`.env`）

| 变量              | 默认值                        | 说明                                                         |
| ----------------- | ----------------------------- | ------------------------------------------------------------ |
| `BIND_ADDR`       | `127.0.0.1:3030`              | HTTP 监听地址（被占用时回退到 3031）                         |
| `LLM_BASE_URL`    | `https://api.openai.com/v1`   | OpenAI 兼容的 base URL                                       |
| `LLM_API_KEY`     | _（留空 = 离线模式）_         | LLM 供应商的 API key                                         |
| `LLM_MODEL`       | `gpt-4o-mini`                 | 对话模型名                                                   |
| `EMBED_MODEL`     | `text-embedding-3-small`     | embedding 模型名                                             |
| `CORS_ORIGIN`     | `http://localhost:3040`       | 允许的前端来源（逗号分隔）                                   |
| `CACHE_VERSION`   | `v1`                          | 变更即让答案缓存整体失效                                     |
| `REQUEST_TIMEOUT_SECS` | `30`                   | 单次 LLM 调用超时（秒）                                      |
| `PRODUCT_NAME`    | `ZeroBuddy`                   | 助手/产品名（出现在提示词与日志）                            |
| `ORG_NAME`        | `Zero Labs`                   | 组织名（出现在提示词）                                       |
| `RAG_TOP_K`       | `3`                           | 喂给 LLM 的最大文档数                                        |
| `RAG_MIN_SCORE`   | `0.2`                         | 纳入文档的最低相似度                                         |
| `CACHE_SIMILARITY`| `0.92`                        | 语义缓存近邻阈值                                             |
| `MAX_MESSAGE_CHARS` | `4000`                     | 单条消息最大字符数                                           |
| `MAX_MESSAGES`    | `50`                          | 单次请求最大消息条数                                         |
| `SCOPE_GUARD_ENABLED` | `true`                    | 开启主题范围限制（Scope Guard）                              |
| `SCOPE_GUARD_MODE`| `prompt`                      | `prompt` = 由 LLM 自我约束；`hard` = 后端在调用 LLM 前拦截跑题问题（省 token） |
| `SCOPE_ALLOW_KEYWORDS` | _（见 `.env.example`）_ | 逗号分隔的子串白名单；命中即放行。留空 = 全部放行            |
| `SCOPE_ALLOW_PATTERNS` | _（见 `.env.example`）_ | `\|` 分隔的正则白名单（产品名、`zerolabsco.com`、GitHub 仓库等）。留空 = 无 |
| `SCOPE_REFUSE_REPLY_ZH` | _（内置中文拒绝语）_    | 中文跑题问题被拦截时返回的回复                               |
| `SCOPE_REFUSE_REPLY_EN` | _（内置英文拒绝语）_    | 英文跑题问题被拦截时返回的回复                               |
| `RATE_LIMIT_PER_MIN` | `10`                       | 单 IP 每分钟最大请求数（0 = 不限）                           |
| `RATE_LIMIT_PER_DAY` | `500`                      | 单 IP 每日最大请求数（0 = 不限）                             |
| `RATE_LIMIT_REPLY_ZH` | _（内置中文提示）_        | 中文客户端被限流时返回的回复                                 |
| `RATE_LIMIT_REPLY_EN` | _（内置英文提示）_        | 英文客户端被限流时返回的回复                                 |

> 未配置有效 key 时服务仍可启动，聊天会以**离线模式**返回检索到的知识，
> 方便演示与验证 RAG 流程。

## 说明

- `data/answers_cache.json` 在用户提问时自动生成；删除它（或调高
  `CACHE_VERSION`）即可重建，该文件已加入 `.gitignore`。
- `Cargo.lock` 已提交以保证可复现构建（二进制项目推荐）。
- 日志通过 `tracing` 输出，**默认同时写控制台和 `backend/logs/app.YYYY-MM-DD.log`（按天滚动，位于后端 crate 根目录下）**；设 `RUST_LOG=debug` 可看详细日志。

---

## 贡献与 CI

完整的贡献指南（英文 + 简体中文）见 **[CONTRIBUTING.md](./CONTRIBUTING.md)**，
遵循 [Zero Labs 贡献规范](https://github.com/zero-labsco/.github/blob/main/profile/CONTRIBUTING.md)：

- **提交信息**须遵循 [Conventional Commits](https://www.conventionalcommits.org)
  （如 `feat:`、`fix:`、`docs:`、`chore:`、`refactor:`、`ci:` …）。
  PR 提交由 `wagoid/commitlint-github-action` 强制校验（见 `commitlint.config.js`）。
- **分支命名**：使用有描述性的前缀，如 `feature/your-feature-name`。
- **代码风格**：Rust 使用 `rustfmt`（Rust Style Guide）。推送前请运行 `cargo fmt`。
- **Pre-commit 钩子**：本仓库在 `.githooks/pre-commit` 内置 `cargo fmt` 检查。
  克隆后启用一次：

  ```bash
  git config core.hooksPath .githooks
  ```

  格式未通过 `cargo fmt --all -- --check` 的提交会被拒绝。

### CI 工作流（`.github/workflows/ci.yml`）

每次推送到 `main`/`master` 以及每个 PR 都会运行：

1. `cargo fmt --all -- --check` — 格式检查。
2. `cargo clippy --all-targets --all-features -- -D warnings` — lint，警告视为错误。
3. `cargo build --verbose` — 编译服务。
4. `cargo test --verbose` — 运行单元测试。
5. **提交信息 lint**（仅 PR）— 逐条校验 Conventional Commits。

### Dependabot（`.github/dependabot.yml`）

- `cargo`：每周依赖更新，提交前缀 `chore:`。
- `github-actions`：每周工作流更新，提交前缀 `ci:`。
- **安全优先**：所有更新须先通过下方的审计 / 依赖审查关卡才能合并；Dependabot 永不自动合并。

### 依赖审计与自动修复（`.github/workflows/audit.yml`）

- 每**周一 09:30 UTC** 运行，也可手动触发（`workflow_dispatch`）。
- **安全是硬关卡**：运行 `cargo audit --deny warnings`。若发现 RUSTSEC 漏洞，
  任务会**故意失败（红）**，直到依赖树干净——绝不为了"最新"而接受不安全版本——
  并开出标题为 `chore(deps): address cargo audit vulnerabilities` 的报告 PR 供人工审查。
- 该 PR **永不自动合并**，也不会自动改动 `Cargo.toml`——维护者须手动升级相关 crate 并运行 `cargo update` 后再合并。

### 依赖审查关卡（`.github/workflows/dependency-review.yml`）

- 每次对 `main` 的 **PR** 都会运行。阻断任何引入高/严重漏洞的变更
  （也覆盖 Dependabot 的"最新版本" PR）。确保**安全优先，其次最新**。

---

### English Documentation

See [`README.md`](./README.md) for the English version.
