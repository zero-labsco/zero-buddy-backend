use crate::cache::AnswerCache;
use crate::config::Config;
use crate::faq::FaqStore;
use crate::knowledge::{load, retrieve};
use crate::llm::LlmClient;
use crate::models::{ChatRequest, Document};
use crate::rag::retrieve_scored;
use anyhow::Result;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// 粗略判断文本是否包含中文（CJK）字符，用于离线兜底文案的语言选择。
fn is_cjk(text: &str) -> bool {
    text.chars().any(|c| {
        let u = c as u32;
        // CJK 统一表意文字基本区 + 扩展 A
        (0x4E00..=0x9FFF).contains(&u) || (0x3400..=0x4DBF).contains(&u)
    })
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
    req: ChatRequest,
) -> Result<serde_json::Value> {
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

    let query = req
        .messages
        .last()
        .map(|m| m.content.trim())
        .unwrap_or("");
    if query.is_empty() {
        anyhow::bail!("last message content is empty");
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
    let docs: Vec<Document> = load("data/knowledge.json")?;
    if docs.is_empty() {
        anyhow::bail!("knowledge base is empty");
    }

    let (scored, _q_emb) = retrieve_scored(client, &docs, query).await?;
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
    let reply = if ctx.is_empty() {
        let sys = format!(
            "You are {}, assistant for the {} project. \
             Reply in the same language the user wrote in: if they write Chinese, answer in Chinese; \
             if they write English, answer in English; for any other language, reply in English.",
            cfg.product_name, cfg.org_name
        );
        client
            .chat(&[
                json!({ "role": "system", "content": sys }),
                json!({ "role": "user", "content": query }),
            ])
            .await
    } else {
        // 带上下文：语言策略（中文->中文，英文->英文，其他语言->英文）已在 chat_with_context 的 system 提示中实现
        client.chat_with_context(query, &ctx).await
    };

    // LLM 调用失败时的处理：区分「鉴权/额度错误」与「限流/网络错误」。
    // 前者（401/402/403，如余额耗尽、key 失效）属于不可逆故障，动态降级为离线，
    // 直接返回检索到的知识片段，避免后续每次聊天都报错；并翻转全局 online 状态。
    // 后者（429 限流、网络抖动）则不降级，向上冒泡为 400 交由前端提示。
    let reply = match reply {
        Ok(r) => r,
        Err(e) => {
            if is_auth_or_quota_error(&e) {
                tracing::warn!("LLM auth/quota error at runtime -> degrading to OFFLINE: {:#}", e);
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
