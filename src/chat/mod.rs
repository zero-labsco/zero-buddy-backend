use crate::config::Config;
use crate::llm::LlmClient;
use crate::models::{ChatRequest, Document};
use crate::retrieval::{load, retrieve, retrieve_scored, AnswerCache, FaqStore};
use anyhow::Result;
use serde_json::json;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// 粗略判断文本是否包含中文（CJK）字符，用于离线兜底文案的语言选择。
fn is_cjk(text: &str) -> bool {
    text.chars().any(|c| {
        let u = c as u32;
        // CJK 统一表意文字基本区 + 扩展 A
        (0x4E00..=0x9FFF).contains(&u) || (0x3400..=0x4DBF).contains(&u)
    })
}

/// 把一个完整文本“打字机式”逐片推送为 SSE 的多个 delta 事件，最后发 done。
/// 用于 FAQ / 缓存 / 离线 / scope-guard 等原本一次性返回的整段答案，
/// 让前端获得与 LLM 流式一致的分批显示体验。前端 onDelta 会累加增量，
/// 因此这里每次 delta 只携带“新增片段”而非整段。
fn streamed_reply(
    text: String,
    source: String,
    url: Option<String>,
) -> axum::response::sse::Sse<
    Pin<
        Box<
            dyn futures_core::Stream<
                    Item = Result<axum::response::sse::Event, std::convert::Infallible>,
                > + Send,
        >,
    >,
> {
    use axum::response::sse::{Event, KeepAlive, Sse};
    let s: Sse<
        Pin<
            Box<
                dyn futures_core::Stream<
                        Item = Result<axum::response::sse::Event, std::convert::Infallible>,
                    > + Send,
            >,
        >,
    > = Sse::new(Box::pin(async_stream::stream! {
        // 按字符切片，每块最多 CHUNK 个字符，间隔 SLEEP_MS 毫秒，模拟打字节奏。
        const CHUNK: usize = 4;
        const SLEEP_MS: u64 = 12;
        let mut idx = 0;
        let chars: Vec<char> = text.chars().collect();
        while idx < chars.len() {
            let end = (idx + CHUNK).min(chars.len());
            let piece: String = chars[idx..end].iter().collect();
            idx = end;
            yield Ok::<_, std::convert::Infallible>(Event::default().data(
                json!({ "type": "delta", "content": piece }).to_string(),
            ));
            if idx < chars.len() {
                tokio::time::sleep(std::time::Duration::from_millis(SLEEP_MS)).await;
            }
        }
        yield Ok::<_, std::convert::Infallible>(Event::default().data(
            json!({ "type": "done", "source": source, "url": url }).to_string(),
        ));
    }));
    s.keep_alive(KeepAlive::default())
}

// 单条消息最大字符数 / 消息条数上限的默认值，实际值从 Config 读取。
// （保留为常量仅作文档说明，运行时以 cfg.max_* 为准）
const _DEFAULT_MAX_MESSAGE_CHARS: usize = 4000;
const _DEFAULT_MAX_MESSAGES: usize = 50;

/// 处理一次聊天请求的核心编排逻辑。
///
/// 管线顺序：输入校验 → FAQ 命中 → 答案缓存(在线) → RAG 检索 → LLM 生成(在线) / 离线返回知识片段。
/// `online` 为 `Arc<AtomicBool>`：启动探测通过时为 true，启用缓存与 LLM；
/// 若运行中发生鉴权/额度错误（如余额耗尽）会就地翻转为 false（动态降级离线），
/// 对所有后续请求立即生效。返回 `{ reply, source }` 形式的 JSON，`source` 标明答案来源。
pub async fn handle_chat(
    cfg: &Config,
    client: &LlmClient,
    cache: &AnswerCache,
    online: Arc<AtomicBool>,
    limiter: &crate::rate_limit::RateLimiter,
    client_ip: &str,
    req: ChatRequest,
) -> Result<serde_json::Value> {
    // 0) 速率限制（防滥用 / 控成本）：按客户端 IP 计数，超限直接拒绝。
    if !limiter.check_and_record(client_ip, cfg.rate_limit_per_min, cfg.rate_limit_per_day) {
        let probe = req
            .messages
            .last()
            .map(|m| m.content.as_str())
            .unwrap_or("");
        let cjk = is_cjk(probe);
        let reply = if cjk {
            &cfg.rate_limit_reply_zh
        } else {
            &cfg.rate_limit_reply_en
        };
        tracing::warn!("rate limit exceeded for {}", client_ip);
        return Ok(json!({ "reply": reply, "source": "rate-limit" }));
    }

    // 输入校验
    if req.messages.is_empty() {
        anyhow::bail!("messages must not be empty");
    }
    if req.messages.len() > cfg.max_messages {
        anyhow::bail!("too many messages (max {})", cfg.max_messages);
    }
    for m in &req.messages {
        if m.content.chars().count() > cfg.max_message_chars {
            anyhow::bail!("message too long (max {} chars)", cfg.max_message_chars);
        }
    }

    let query = req.messages.last().map(|m| m.content.trim()).unwrap_or("");
    if query.is_empty() {
        anyhow::bail!("last message content is empty");
    }

    // 0) 话题范围硬拦截（Scope Guard / hard 模式）：
    // 启用且为 hard 模式时，先用关键词白名单判断；超范围直接返回拒绝语，
    // 完全不调用 LLM，也不走 FAQ/缓存/RAG，最大程度省 token、防滥用。
    // prompt 模式不在此拦截，交由 LLM 在 system prompt 中自觉遵守。
    if cfg.scope_guard_enabled
        && cfg.scope_guard_mode == crate::config::ScopeMode::Hard
        && !cfg.in_scope(query)
    {
        let cjk = is_cjk(query);
        let reply = if cjk {
            &cfg.scope_refuse_reply_zh
        } else {
            &cfg.scope_refuse_reply_en
        };
        tracing::info!("scope guard (hard): rejected off-topic query");
        return Ok(json!({ "reply": reply, "source": "scope-guard" }));
    }

    // 1) FAQ 优先（高频固定问答，零 token 成本）
    let faq = FaqStore::load("data/faq.json")?;
    if let Some((answer, faq_url)) = faq.answer(query) {
        tracing::info!("answered from FAQ");
        // FAQ 命中也写入缓存（离线用 put_offline，不依赖 embedding），
        // 使中文问答在离线模式下可复用、持久化到 answers_cache.json
        let _ = cache.put_offline(query, &answer).await;
        // FAQ 命中时若有配置链接（如邮箱/官网），一并随回答返回前端渲染成可点击链接
        if let Some(u) = faq_url {
            return Ok(json!({ "reply": answer, "source": "faq", "url": u }));
        }
        return Ok(json!({ "reply": answer, "source": "faq" }));
    }

    // 2) 答案缓存（方案2 精确 + 方案3 语义近邻）。
    // 精确命中不依赖 embedding，离线也可用；语义近邻因离线无 key 自动跳过。
    if let Some(reply) = cache.get(client, query, cfg.cache_similarity).await {
        tracing::info!("answered from answer cache");
        return Ok(json!({ "reply": reply, "source": "cache" }));
    }

    // 3) RAG + LLM
    // 注意：embedding 模型可能不可用（如当前 LLM 端点只提供 chat 模型）。
    // 此时 RAG 检索应优雅降级为"无上下文"，而非让整个请求 500 崩溃——
    // 下方在线逻辑本身已支持 ctx 为空时直接纯对话（见第 165 行分支）。
    let docs: Vec<Document> = load("data/knowledge.json")?;
    if docs.is_empty() {
        anyhow::bail!("knowledge base is empty");
    }

    let (scored, _q_emb) = match retrieve_scored(client, &docs, query).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                "RAG embedding unavailable ({}); falling back to pure chat without knowledge context",
                e
            );
            // 无向量可用：所有文档相似度记 0，等价于不检索知识库，直接走纯对话
            (Vec::new(), None)
        }
    };
    let top: Vec<usize> = scored
        .iter()
        .filter(|(_, s)| *s >= cfg.rag_min_score)
        .take(cfg.rag_top_k)
        .map(|(i, _)| *i)
        .collect();
    let ctx = retrieve(&docs, &top).join("\n---\n");
    // 取命中文档里的第一个可用链接（如官网/邮箱），随回答一并返回前端、渲染成可点击链接
    let doc_url: Option<&str> = top
        .iter()
        .filter_map(|i| docs.get(*i))
        .filter_map(|d| d.url.as_deref())
        .find(|u| !u.is_empty());

    // 联网兜底：始终尝试抓取官网/GitHub/Invoice Zero 内容并追加到 ctx（带 TTL 缓存，首次略慢、之后秒回）。
    // 兜底内容来自官方/产品页面，天然属于项目范围，因此命中兜底时不附加 scope 限制（见下）。
    // 即便 RAG 命中了泛化的产品列表文档，官网内容也能补充「收费/套餐」等细节，避免回答「没有相关信息」。
    let mut ctx = ctx;
    let mut ctx_from_web = false;
    let web_ctx = crate::retrieval::fetch_zero_labs_context().await;
    if !web_ctx.is_empty() {
        if ctx.is_empty() {
            ctx = format!(
                "Retrieved from Zero Labs official website / GitHub / Invoice Zero site:\n{}",
                web_ctx
            );
        } else {
            ctx = format!(
                "{}\n\n---\nRetrieved from Zero Labs official website / GitHub / Invoice Zero site:\n{}",
                ctx, web_ctx
            );
        }
        ctx_from_web = true;
        tracing::info!("using web-fallback context for query: {}", query);
    }

    // 离线模式：直接返回检索到的知识片段（不调用 LLM，零 token）
    if !online.load(Ordering::Relaxed) {
        let cjk = is_cjk(query);
        if ctx.is_empty() {
            // 离线且无命中：优雅引导用户用中文或英文提问（中文或英文提示）
            let reply = if cjk {
                "抱歉，我暂时没有找到相关内容。请使用中文或英文提问，我会尽力为你解答。"
            } else {
                "Sorry, I couldn't find anything relevant. Please ask in Chinese or English and I'll do my best to help."
            };
            let _ = cache.put_offline(query, reply).await;
            return Ok(json!({
                "reply": reply,
                "source": "offline"
            }));
        }
        let suffix = if cjk {
            "（离线模式：展示原始知识片段。配置 LLM_API_KEY 可启用 AI 回答。）"
        } else {
            "(Offline mode: showing raw knowledge. Set LLM_API_KEY to enable AI answers.)"
        };
        let reply = format!("{}\n\n{}", ctx, suffix);
        let _ = cache.put_offline(query, &reply).await;
        return Ok(json!({
            "reply": reply,
            "source": "offline",
            "url": doc_url
        }));
    }

    // 在线模式：调 LLM 生成答案
    // 从前端传来的多轮 messages 中抽取历史（除最后一条当前 query 外），
    // 构造为 LLM 可消费的 history，实现跨轮上下文记忆。
    let history: Vec<serde_json::Value> = req
        .messages
        .iter()
        .take(req.messages.len().saturating_sub(1))
        .map(|m| json!({ "role": m.role, "content": m.content }))
        .collect();
    let reply = if ctx.is_empty() {
        let scope_policy = if cfg.scope_guard_enabled && !ctx_from_web {
            format!(
                " Scope policy: ONLY answer questions about the {} / {} project. \
                 For anything outside this scope, politely say you can only help with {} topics \
                 and suggest contacting support.",
                cfg.product_name, cfg.org_name, cfg.product_name
            )
        } else {
            String::new()
        };
        let sys = format!(
            "You are {}, assistant for the {} project. \
             Reply in the same language the user wrote in: if they write Chinese, answer in Chinese; \
             if they write English, answer in English; for any other language, reply in English.{}",
            cfg.product_name, cfg.org_name, scope_policy
        );
        let mut messages = vec![json!({ "role": "system", "content": sys })];
        for h in &history {
            messages.push(h.clone());
        }
        messages.push(json!({ "role": "user", "content": query }));
        client.chat(&messages).await
    } else {
        // 带上下文：语言策略（中文->中文，英文->英文，其他语言->英文）已在 chat_with_context 的 system 提示中实现
        client
            .chat_with_context_and_history(query, &ctx, &history)
            .await
    };

    // LLM 调用失败时的处理：区分「鉴权/额度错误」与「限流/网络错误」。
    // 前者（401/402/403，如余额耗尽、key 失效）属于不可逆故障，动态降级为离线，
    // 直接返回检索到的知识片段，避免后续每次聊天都报错；并翻转全局 online 状态。
    // 后者（429 限流、网络抖动）则不降级，向上冒泡为 400 交由前端提示。
    let reply = match reply {
        Ok(r) => r,
        Err(e) => {
            if is_auth_or_quota_error(&e) {
                tracing::warn!(
                    "LLM auth/quota error at runtime -> degrading to OFFLINE: {:#}",
                    e
                );
                online.store(false, Ordering::Relaxed);
                let cjk = is_cjk(query);
                if ctx.is_empty() {
                    // 运行时降级到离线且无命中：优雅引导用户用中文或英文提问
                    let reply = if cjk {
                        "抱歉，我暂时没有找到相关内容。请使用中文或英文提问，我会尽力为你解答。"
                    } else {
                        "Sorry, I couldn't find anything relevant. Please ask in Chinese or English and I'll do my best to help."
                    };
                    let _ = cache.put_offline(query, reply).await;
                    return Ok(json!({
                        "reply": reply,
                        "source": "offline"
                    }));
                }
                let suffix = if cjk {
                    "（离线模式：展示原始知识片段。LLM 密钥已失效或额度用尽。）"
                } else {
                    "(Offline mode: showing raw knowledge. LLM key expired or quota exceeded.)"
                };
                let reply = format!("{}\n\n{}", ctx, suffix);
                let _ = cache.put_offline(query, &reply).await;
                return Ok(json!({
                    "reply": reply,
                    "source": "offline",
                    "url": doc_url
                }));
            }
            return Err(e);
        }
    };

    // 不缓存无效回答（"我不知道"等），避免污染缓存
    if !is_unhelpful(&reply) {
        cache.put(client, query, &reply).await;
    }

    Ok(json!({ "reply": reply, "source": "llm", "url": doc_url }))
}

/// 处理流式聊天请求：管线与 `handle_chat` 前段一致（限流/校验/FAQ/缓存/RAG/离线），
/// 但最后的 LLM 生成改为流式输出：每个 token 通过 SSE 事件 `{"type":"delta","content":...}` 实时推给前端，
/// 结束时发 `{"type":"done","source":...,"url":...}`。前端据此实现"边生成边显示"。
///
/// SSE 事件协议（前端约定）：
///   data: {"type":"delta","content":"片段"}      // 增量文本
///   data: {"type":"done","source":"llm","url":null}  // 正常结束，携带来源/链接
///   data: {"type":"error","message":"..."}       // 业务/运行时错误
pub async fn handle_chat_stream(
    cfg: &Config,
    client: &LlmClient,
    cache: &AnswerCache,
    online: Arc<AtomicBool>,
    limiter: &crate::rate_limit::RateLimiter,
    client_ip: &str,
    req: ChatRequest,
) -> axum::response::sse::Sse<
    Pin<
        Box<
            dyn futures_core::Stream<
                    Item = Result<axum::response::sse::Event, std::convert::Infallible>,
                > + Send,
        >,
    >,
> {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use std::convert::Infallible;

    // 构造一条 SSE 事件的辅助闭包（供 prepare 错误回调与 LLM 流式生成回调复用）
    let sse_event =
        |payload: serde_json::Value| -> Event { Event::default().data(payload.to_string()) };

    // 复用前段逻辑（与 handle_chat 对齐）；prepare 阶段出错时直接以 error 事件结束流。
    let prepare_err = |msg: String| {
        let s: Sse<
            Pin<
                Box<
                    dyn futures_core::Stream<
                            Item = Result<axum::response::sse::Event, std::convert::Infallible>,
                        > + Send,
                >,
            >,
        > = Sse::new(Box::pin(async_stream::stream! {
            yield Ok::<_, Infallible>(Event::default().data(
                serde_json::json!({ "type": "error", "message": msg }).to_string(),
            ));
        }));
        s.keep_alive(KeepAlive::default())
    };

    if !limiter.check_and_record(client_ip, cfg.rate_limit_per_min, cfg.rate_limit_per_day) {
        let probe = req
            .messages
            .last()
            .map(|m| m.content.as_str())
            .unwrap_or("");
        let cjk = is_cjk(probe);
        let reply = if cjk {
            &cfg.rate_limit_reply_zh
        } else {
            &cfg.rate_limit_reply_en
        };
        tracing::warn!("rate limit exceeded for {}", client_ip);
        return streamed_reply(reply.to_string(), "rate-limit".to_string(), None);
    }

    if req.messages.is_empty() {
        return prepare_err("messages must not be empty".into());
    }
    if req.messages.len() > cfg.max_messages {
        return prepare_err(format!("too many messages (max {})", cfg.max_messages));
    }
    for m in &req.messages {
        if m.content.chars().count() > cfg.max_message_chars {
            return prepare_err(format!(
                "message too long (max {} chars)",
                cfg.max_message_chars
            ));
        }
    }
    let query = req.messages.last().map(|m| m.content.trim()).unwrap_or("");
    if query.is_empty() {
        return prepare_err("last message content is empty".into());
    }

    if cfg.scope_guard_enabled
        && cfg.scope_guard_mode == crate::config::ScopeMode::Hard
        && !cfg.in_scope(query)
    {
        let cjk = is_cjk(query);
        let reply = if cjk {
            &cfg.scope_refuse_reply_zh
        } else {
            &cfg.scope_refuse_reply_en
        };
        tracing::info!("scope guard (hard): rejected off-topic query");
        return streamed_reply(reply.to_string(), "scope-guard".to_string(), None);
    }

    // FAQ 命中（零 token）
    let faq = match FaqStore::load("data/faq.json") {
        Ok(f) => f,
        Err(e) => return prepare_err(e.to_string()),
    };
    if let Some((answer, faq_url)) = faq.answer(query) {
        tracing::info!("answered from FAQ (stream)");
        let _ = cache.put_offline(query, &answer).await;
        let url = faq_url.map(|u| u.to_string());
        return streamed_reply(answer, "faq".to_string(), url);
    }

    // 答案缓存命中
    if let Some(reply) = cache.get(client, query, cfg.cache_similarity).await {
        tracing::info!("answered from answer cache (stream)");
        return streamed_reply(reply, "cache".to_string(), None);
    }

    // RAG 检索（embedding 不可用时降级为纯对话）
    let docs = match load("data/knowledge.json") {
        Ok(d) => d,
        Err(e) => return prepare_err(e.to_string()),
    };
    if docs.is_empty() {
        return prepare_err("knowledge base is empty".into());
    }
    let (scored, _q_emb) = match retrieve_scored(client, &docs, query).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                "RAG embedding unavailable ({}); falling back to pure chat without knowledge context",
                e
            );
            (Vec::new(), None)
        }
    };
    let top: Vec<usize> = scored
        .iter()
        .filter(|(_, s)| *s >= cfg.rag_min_score)
        .take(cfg.rag_top_k)
        .map(|(i, _)| *i)
        .collect();
    let ctx = retrieve(&docs, &top).join("\n---\n");
    let doc_url: Option<String> = top
        .iter()
        .filter_map(|i| docs.get(*i))
        .filter_map(|d| d.url.as_deref())
        .find(|u| !u.is_empty())
        .map(|u| u.to_string());

    // 联网兜底：始终尝试抓取官网/GitHub/Invoice Zero 内容并追加到 ctx（与 handle_chat 一致）
    let mut ctx = ctx;
    let mut ctx_from_web = false;
    let web_ctx = crate::retrieval::fetch_zero_labs_context().await;
    if !web_ctx.is_empty() {
        if ctx.is_empty() {
            ctx = format!(
                "Retrieved from Zero Labs official website / GitHub / Invoice Zero site:\n{}",
                web_ctx
            );
        } else {
            ctx = format!(
                "{}\n\n---\nRetrieved from Zero Labs official website / GitHub / Invoice Zero site:\n{}",
                ctx, web_ctx
            );
        }
        ctx_from_web = true;
        tracing::info!("using web-fallback context for query (stream): {}", query);
    }

    // 离线模式：直接返回知识片段（单条 delta + done）
    if !online.load(Ordering::Relaxed) {
        let cjk = is_cjk(query);
        let reply: String = if ctx.is_empty() {
            if cjk {
                "抱歉，我暂时没有找到相关内容。请使用中文或英文提问，我会尽力为你解答。"
            } else {
                "Sorry, I couldn't find anything relevant. Please ask in Chinese or English and I'll do my best to help."
            }
            .to_string()
        } else {
            let suffix = if cjk {
                "（离线模式：展示原始知识片段。配置 LLM_API_KEY 可启用 AI 回答。）"
            } else {
                "(Offline mode: showing raw knowledge. Set LLM_API_KEY to enable AI answers.)"
            };
            format!("{}\n\n{}", ctx, suffix)
        };
        let _ = cache.put_offline(query, &reply).await;
        return streamed_reply(reply, "offline".to_string(), doc_url);
    }

    // 在线模式：构造发给 LLM 的 messages（与 handle_chat 一致）
    // 抽取多轮历史（除最后一条当前 query 外），实现跨轮上下文记忆。
    let history: Vec<serde_json::Value> = req
        .messages
        .iter()
        .take(req.messages.len().saturating_sub(1))
        .map(|m| json!({ "role": m.role, "content": m.content }))
        .collect();
    let messages: Vec<serde_json::Value> = if ctx.is_empty() {
        let scope_policy = if cfg.scope_guard_enabled && !ctx_from_web {
            format!(
                " Scope policy: ONLY answer questions about the {} / {} project. \
                 For anything outside this scope, politely say you can only help with {} topics \
                 and suggest contacting support.",
                cfg.product_name, cfg.org_name, cfg.product_name
            )
        } else {
            String::new()
        };
        let sys = format!(
            "You are {}, assistant for the {} project. \
             Reply in the same language the user wrote in: if they write Chinese, answer in Chinese; \
             if they write English, answer in English; for any other language, reply in English.{}",
            cfg.product_name, cfg.org_name, scope_policy
        );
        let mut messages = vec![json!({ "role": "system", "content": sys })];
        for h in &history {
            messages.push(h.clone());
        }
        messages.push(json!({ "role": "user", "content": query }));
        messages
    } else {
        let scope_policy = if cfg.scope_guard_enabled && !ctx_from_web {
            format!(
                "Scope policy: ONLY answer questions related to the {} / {} project (its features, usage, API, \
                 deployment, and the provided knowledge base). For anything outside this scope, politely reply that \
                 you can only help with {} topics and suggest contacting support. Never answer off-site questions.\n",
                cfg.product_name, cfg.org_name, cfg.product_name
            )
        } else {
            String::new()
        };
        let system = format!(
            "You are {}, a helpful assistant for the {} project. \
             Use the provided context to answer the user's question concisely and accurately.\n\
             Language policy: reply in the SAME language as the user's question. \
             If the question is in Chinese, answer in Chinese; if in any other non-English language, default to English.\n\
             {}\
             If the context does not contain the answer, say you don't know and suggest contacting support.\n\n\
             Context:\n{}",
            cfg.product_name, cfg.org_name, scope_policy, ctx
        );
        let mut messages = vec![json!({ "role": "system", "content": system })];
        for h in &history {
            messages.push(h.clone());
        }
        messages.push(json!({ "role": "user", "content": query }));
        messages
    };

    // 在线流式生成：用 mpsc channel 把 chat_stream 的 token 实时转成 SSE 事件推出。
    // chat_stream 的回调把每个 token 发进 channel；async_stream 从 receiver 取出并 yield。
    let client = client.clone();
    let cache_c = cache.clone();
    let query_owned = query.to_string();
    let online_c = online.clone();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(64);
    // 在独立任务里调用 chat_stream（其回调向 channel 发送 token），避免与 async_stream 的 yield 冲突。
    let gen_task = tokio::spawn(async move {
        let mut full = String::new();
        let result = client
            .chat_stream(&messages, |tok| {
                full.push_str(tok);
                // 忽略发送失败（前端断开时 channel 关闭）
                let _ = tx.try_send(tok.to_string());
            })
            .await;
        match result {
            Ok(_) => {
                // 不缓存无效回答
                if !is_unhelpful(&full) {
                    cache_c.put(&client, &query_owned, &full).await;
                }
                // 控制信令：以 __CTL__ 为前缀，后接 DONE
                let _ = tx.try_send("__CTL__DONE".to_string());
            }
            Err(e) => {
                // 鉴权/额度错误运行时降级离线
                if is_auth_or_quota_error(&e) {
                    tracing::warn!(
                        "LLM auth/quota error at runtime -> degrading to OFFLINE: {:#}",
                        e
                    );
                    online_c.store(false, Ordering::Relaxed);
                }
                let _ = tx.try_send(format!("__CTL__ERR{}", e));
            }
        }
    });

    let s: Sse<
        Pin<
            Box<
                dyn futures_core::Stream<
                        Item = Result<axum::response::sse::Event, std::convert::Infallible>,
                    > + Send,
            >,
        >,
    > = Sse::new(Box::pin(async_stream::stream! {
        // 先发一个空 delta 让前端立即进入"生成中"状态，避免首字延迟期间的空白
        yield Ok::<_, Infallible>(sse_event(serde_json::json!({ "type": "delta", "content": "" })));
        while let Some(chunk) = rx.recv().await {
            if let Some(rest) = chunk.strip_prefix("__CTL__") {
                // 控制信令
                if rest == "DONE" {
                    yield Ok::<_, Infallible>(sse_event(serde_json::json!({ "type": "done", "source": "llm", "url": doc_url })));
                } else if let Some(errmsg) = rest.strip_prefix("ERR") {
                    yield Ok::<_, Infallible>(sse_event(serde_json::json!({ "type": "error", "message": errmsg })));
                }
            } else {
                yield Ok::<_, Infallible>(sse_event(serde_json::json!({ "type": "delta", "content": chunk })));
            }
        }
        // 任务结束但循环退出（极端情况下没有收到 DONE 信号），补一个 done
        let _ = gen_task.await;
        yield Ok::<_, Infallible>(sse_event(serde_json::json!({ "type": "done", "source": "llm", "url": doc_url })));
    }));
    s.keep_alive(KeepAlive::default())
}

/// 判断 LLM 错误是否属于「鉴权/额度类」错误，需要动态降级离线。
/// 覆盖：401 Unauthorized、402 Payment Required（余额不足）、403 Forbidden。
/// 不含 429 Too Many Requests（限流，属瞬时错误，不应降级）。
fn is_auth_or_quota_error(e: &anyhow::Error) -> bool {
    let msg = e.to_string().to_lowercase();
    msg.contains("401")
        || msg.contains("402")
        || msg.contains("403")
        || msg.contains("unauthorized")
        || msg.contains("payment required")
        || msg.contains("quota")
        || msg.contains("insufficient")
        || msg.contains("forbidden")
}

// 判断 LLM 是否给出了"无答案"的废话，避免写入缓存。
fn is_unhelpful(reply: &str) -> bool {
    let r = reply.to_lowercase();
    r.contains("don't know")
        || r.contains("do not know")
        || r.contains("i'm not sure")
        || r.contains("cannot help")
        || r.contains("无法")
        || r.contains("不知道")
}
