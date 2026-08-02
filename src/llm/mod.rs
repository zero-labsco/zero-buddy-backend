use crate::config::Config;
use anyhow::Result;
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
        let url = format!("{}/chat/completions", self.cfg.llm_base_url);
        let body = serde_json::json!({
            "model": self.cfg.llm_model,
            "messages": messages,
            "temperature": 0.2,
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
        let data: ChatResponse = resp.json().await?;
        Ok(data
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default())
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
    pub async fn chat_with_context(&self, query: &str, context: &str) -> Result<String> {
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
        let messages = vec![
            serde_json::json!({ "role": "system", "content": system }),
            serde_json::json!({ "role": "user", "content": query }),
        ];
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
