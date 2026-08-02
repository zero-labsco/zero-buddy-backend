# Zero Buddy — 后端

> Zero Labs 的 AI 助手后端，回答关于 Zero Inspector Kit、Flutter Agent Kit、WizardPlayer、Invoice Zero 的问题。

---

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
    ├── config.rs             # 环境配置 + has_valid_key()
    ├── models.rs             # 请求 / 响应 / 文档类型
    ├── llm.rs                # OpenAI 兼容的对话与 embedding（复用 client）
    ├── knowledge.rs          # 知识加载 + retrieve
    ├── rag.rs                # 向量检索（余弦）+ 关键词兜底
    ├── cache.rs              # 答案缓存（精确 + 语义）
    ├── faq.rs                # FAQ 匹配
    └── chat.rs               # 编排 FAQ → 缓存 → RAG → LLM
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

| 变量            | 默认值                        | 说明                          |
| --------------- | ----------------------------- | ----------------------------- |
| `BIND_ADDR`     | `127.0.0.1:3030`              | HTTP 监听地址（被占用时回退到 3031） |
| `LLM_BASE_URL`  | `https://api.openai.com/v1`  | OpenAI 兼容的 base URL        |
| `LLM_API_KEY`   | _（留空 = 离线模式）_         | LLM 供应商的 API key          |
| `LLM_MODEL`     | `gpt-4o-mini`                 | 对话模型名                    |
| `EMBED_MODEL`   | `text-embedding-3-small`     | embedding 模型名              |
| `CORS_ORIGIN`   | `http://localhost:3040`       | 允许的前端来源（逗号分隔）     |
| `CACHE_VERSION` | `v1`                          | 变更即让答案缓存整体失效       |
| `REQUEST_TIMEOUT_SECS` | `30`                   | 单次 LLM 调用超时（秒）        |
| `PRODUCT_NAME`  | `ZeroBuddy`                   | 助手/产品名（出现在提示词与日志） |
| `ORG_NAME`      | `Zero Labs`                   | 组织名（出现在提示词）         |
| `RAG_TOP_K`     | `3`                           | 喂给 LLM 的最大文档数          |
| `RAG_MIN_SCORE` | `0.2`                         | 纳入文档的最低相似度           |
| `CACHE_SIMILARITY` | `0.92`                     | 语义缓存近邻阈值               |
| `MAX_MESSAGE_CHARS` | `4000`                    | 单条消息最大字符数             |
| `MAX_MESSAGES`  | `50`                          | 单次请求最大消息条数           |

> 未配置有效 key 时服务仍可启动，聊天会以**离线模式**返回检索到的知识，
> 方便演示与验证 RAG 流程。

## 说明

- `data/answers_cache.json` 在用户提问时自动生成；删除它（或调高
  `CACHE_VERSION`）即可重建，该文件已加入 `.gitignore`。
- `Cargo.lock` 已提交以保证可复现构建（二进制项目推荐）。
- 日志通过 `tracing` 输出，**默认同时写控制台和 `backend/logs/app.YYYY-MM-DD.log`（按天滚动，位于后端 crate 根目录下）**；设 `RUST_LOG=debug` 可看详细日志。
