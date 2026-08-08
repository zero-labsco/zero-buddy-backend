use crate::config::Config;
use anyhow::Result;
use bytes::Bytes;
use futures::StreamExt;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

// LLM 与向量化客户端封装。
// Client 复用：reqwest::Client::new() 代价较高，整个进程应只创建一次，
// 通过 Arc 在 ChatService 内共享，避免每次请求都重建连接池。
#[derive(Clone)]
pub struct LlmClient {
    inner: Arc<reqwest::Client>,
    cfg: Config,
}

impl LlmClient {
    pub fn new(cfg: Config) -> Self {
        let timeout = Duration::from_secs(cfg.request_timeout_secs());
        let inner = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .expect("failed to build reqwest client");
        Self {
            inner: Arc::new(inner),
            cfg,
        }
    }

    // 计算文本的向量表示（用于 RAG 检索；答案缓存层也复用它做语义近邻匹配）。
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let url = format!("{}/embeddings", self.cfg.llm_base_url);
        let body = serde_json::json!({
            "model": self.cfg.embed_model,
            "input": text,
        });
        let resp = self
            .inner
            .post(&url)
            .bearer_auth(&self.cfg.llm_api_key)
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("embed request failed: {}", resp.status());
        }
        let data: EmbeddingResponse = resp.json().await?;
        Ok(data
            .data
            .into_iter()
            .next()
            .map(|e| e.embedding)
            .unwrap_or_default())
    }

    // 直接对话（不带上下文），供简单问答与缓存回写使用。
    pub async fn chat(&self, messages: &[serde_json::Value]) -> Result<String> {
        let mut content = String::new();
        self.stream_chat(messages, &mut |s| content.push_str(s))
            .await?;
        Ok(content)
    }

    /// 流式对话：与 `chat` 同样的请求，但每解析出一块 delta.content 就通过 `sink` 回调，
    /// 便于上层（如 SSE 接口）实时把 token 推给前端，实现"边生成边显示"的打字机效果。
    /// 返回完整累积文本（供缓存使用）。
    pub async fn chat_stream(
        &self,
        messages: &[serde_json::Value],
        mut sink: impl FnMut(&str) + Send,
    ) -> Result<String> {
        let mut content = String::new();
        self.stream_chat(messages, &mut |s| {
            content.push_str(s);
            sink(s);
        })
        .await?;
        Ok(content)
    }

    /// 内部实现：发请求并解析（SSE 或非流式），每解析出一段 content 调用 `sink`。
    /// `sink` 标注 `+ Send` 以保证在 `tokio::spawn` 的跨线程 future 中可被安全持有。
    async fn stream_chat(
        &self,
        messages: &[serde_json::Value],
        sink: &mut (dyn FnMut(&str) + Send),
    ) -> Result<()> {
        let url = format!("{}/chat/completions", self.cfg.llm_base_url);
        let body = serde_json::json!({
            "model": self.cfg.llm_model,
            "messages": messages,
            "temperature": 0.2,
            "stream": true,
        });
        let resp = self
            .inner
            .post(&url)
            .bearer_auth(&self.cfg.llm_api_key)
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("chat request failed: {}", resp.status());
        }
        // 真正的流式解析：用 bytes_stream() 边收边按行切分，每解析出一段 delta.content
        // 就立即 sink，从而实现「边生成边显示」的打字机效果（而非等整段响应收完再输出）。
        let mut stream = resp.bytes_stream();
        let mut line_buf = String::new();
        let mut saw_sse = false;
        while let Some(chunk) = stream.next().await {
            let bytes: Bytes = chunk?;
            // 把字节追加到行缓冲，按 \n 逐行处理（保留未结束的行）
            let text = match String::from_utf8_lossy(&bytes) {
                std::borrow::Cow::Owned(s) => s,
                std::borrow::Cow::Borrowed(b) => b.to_string(),
            };
            line_buf.push_str(&text);
            while let Some(nl) = line_buf.find('\n') {
                let raw_line = line_buf[..nl].trim().to_string();
                line_buf = line_buf[nl + 1..].to_string();
                if raw_line.is_empty() {
                    continue;
                }
                if !raw_line.starts_with("data:") {
                    // 非 SSE 文本行（如某些端点直接返回 JSON）
                    continue;
                }
                saw_sse = true;
                let payload = raw_line["data:".len()..].trim();
                if payload.is_empty() || payload == "[DONE]" {
                    continue;
                }
                if let Ok(ev) = serde_json::from_str::<SseChunk>(payload) {
                    if let Some(choices) = ev.choices {
                        for ch in choices {
                            if let Some(delta) = ch.delta {
                                if let Some(c) = delta.content {
                                    sink(&c);
                                }
                            }
                        }
                    }
                }
            }
        }
        // 兜底：若上游未走 SSE（直接返回单个 JSON 对象），最后解析整段缓冲
        let rest = line_buf.trim();
        if !saw_sse && !rest.is_empty() {
            if let Ok(data) = serde_json::from_str::<ChatResponse>(rest) {
                if let Some(c) = data.choices.into_iter().next().map(|c| c.message.content) {
                    sink(&c);
                }
            }
        }
        Ok(())
    }

    // 启动时探测 key 合法性：发一个最小对话请求，成功返回 true，失败（如 401）返回 false。
    // 用于“配了 key 但无效时降级到离线模式”，避免每次聊天都返回 400。
    pub async fn check_key(&self) -> bool {
        let probe = vec![serde_json::json!({
            "role": "user",
            "content": "ping"
        })];
        match self.chat(&probe).await {
            Ok(_) => true,
            Err(e) => {
                tracing::warn!("LLM key check failed (will run offline): {:#}", e);
                false
            }
        }
    }

    // 带知识上下文的对话（用于 RAG）。
    /// 带知识上下文 + 多轮历史的对话（用于 RAG）。
    /// `history` 为前端传来的历史消息（已剔除当前 query），每条形如
    /// `{"role":"user"/"assistant","content":"..."}`，按顺序拼在 system 之后、当前 query 之前，
    /// 使 LLM 拥有跨轮上下文记忆。
    pub async fn chat_with_context_and_history(
        &self,
        query: &str,
        context: &str,
        history: &[serde_json::Value],
    ) -> Result<String> {
        let scope_policy = if self.cfg.scope_guard_enabled {
            format!(
                "Scope policy: ONLY answer questions related to the {} / {} project (its features, usage, API, \
                 deployment, and the provided knowledge base). For anything outside this scope, politely reply that \
                 you can only help with {} topics and suggest contacting support. Never answer off-topic questions.\n",
                self.cfg.product_name, self.cfg.org_name, self.cfg.product_name
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
            self.cfg.product_name,
            self.cfg.org_name,
            scope_policy,
            context
        );
        let mut messages = vec![serde_json::json!({ "role": "system", "content": system })];
        // 追加历史消息（如前端未提供历史则为空），保留对话上下文
        for h in history {
            messages.push(h.clone());
        }
        messages.push(serde_json::json!({ "role": "user", "content": query }));
        self.chat(&messages).await
    }
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}
#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}
#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}
#[derive(Deserialize)]
struct Choice {
    message: Message,
}
#[derive(Deserialize)]
struct Message {
    content: String,
}

// SSE 流式响应的轻量解析结构，所有字段均设为可选以兼容不同厂商格式。
#[derive(Deserialize)]
struct SseChunk {
    #[serde(default)]
    choices: Option<Vec<SseChoice>>,
}
#[derive(Deserialize)]
struct SseChoice {
    #[serde(default)]
    delta: Option<SseDelta>,
}
#[derive(Deserialize)]
struct SseDelta {
    #[serde(default)]
    content: Option<String>,
}
