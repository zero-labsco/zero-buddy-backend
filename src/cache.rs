use crate::config::Config;
use crate::llm::LlmClient;
use crate::rag::cosine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::debug;

// 答案缓存：省 token 的核心层。
//
// 方案 2（精确命中）：归一化后的 query 文本作为 key，命中直接返回上次 LLM 答案。
// 方案 3（语义近邻兜底）：query 的 embedding 与缓存项的 embedding 做余弦，
//   超过阈值即视为"换种问法"，复用旧答案——覆盖用户改写提问的情况。
//
// 落盘：运行时写内存 HashMap，并以 JSON 持久化到 data/answers_cache.json。
// 失效：Config.cache_version 变更时整体失效（删除旧文件重建）。

const DEFAULT_SIMILARITY_THRESHOLD: f32 = 0.92; // 方案3 近邻阈值默认值（可用 CACHE_SIMILARITY 覆盖）
const CACHE_FILE: &str = "data/answers_cache.json";

#[derive(Clone, Serialize, Deserialize)]
struct CacheEntry {
    query: String,
    reply: String,
    embedding: Vec<f32>,
}

#[derive(Clone)]
pub struct AnswerCache {
    inner: Arc<AnswerCacheInner>,
}

struct AnswerCacheInner {
    version: String,
    map: Mutex<HashMap<String, CacheEntry>>,
    path: PathBuf,
}

impl AnswerCache {
    pub fn load(cfg: &Config) -> Self {
        let path = PathBuf::from(CACHE_FILE);
        let mut map: HashMap<String, CacheEntry> = HashMap::new();

        if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(s) => match serde_json::from_str::<CacheFile>(&s) {
                    Ok(cf) if cf.version == cfg.cache_version => {
                        for e in cf.entries {
                            map.insert(normalize(&e.query), e);
                        }
                        tracing::info!("answer cache loaded: {} entries", map.len());
                    }
                    Ok(_) => {
                        tracing::info!("answer cache version mismatch, starting fresh");
                    }
                    Err(e) => {
                        tracing::warn!("answer cache parse failed ({}), starting fresh", e);
                    }
                },
                Err(e) => tracing::warn!("answer cache read failed: {}", e),
            }
        }
        Self {
            inner: Arc::new(AnswerCacheInner {
                version: cfg.cache_version.clone(),
                map: Mutex::new(map),
                path,
            }),
        }
    }

    // 查询缓存：先精确命中，再语义近邻。返回 Some(reply) 表示命中。
    // threshold 为语义近邻阈值（来自 Config.cache_similarity）。
    pub async fn get(&self, client: &LlmClient, query: &str, threshold: f32) -> Option<String> {
        let key = normalize(query);
        {
            let guard = self.inner.map.lock().unwrap();
            if let Some(e) = guard.get(&key) {
                debug!("cache hit (exact): {}", key);
                return Some(e.reply.clone());
            }
        }
        // 方案3：语义近邻（需要在线才能算 embedding，离线模式跳过）
        if let Ok(q_emb) = client.embed(query).await {
            let guard = self.inner.map.lock().unwrap();
            let mut best: Option<(f32, String)> = None;
            for e in guard.values() {
                if e.embedding.is_empty() {
                    continue;
                }
                let s = cosine(&q_emb, &e.embedding);
                let th = if threshold <= 0.0 { DEFAULT_SIMILARITY_THRESHOLD } else { threshold };
                if s >= th {
                    best = match best {
                        Some((bs, _)) if bs >= s => best,
                        _ => Some((s, e.reply.clone())),
                    };
                }
            }
            if let Some((s, reply)) = best {
                debug!("cache hit (semantic): score={:.3}", s);
                return Some(reply);
            }
        }
        None
    }

    // 写入缓存并异步落盘（失败仅告警，不影响主流程）。
    // 调用方需保证 reply 不是"我不知道"之类的无效回答。
    pub async fn put(&self, client: &LlmClient, query: &str, reply: &str) {
        let key = normalize(query);
        let q_emb = client.embed(query).await.unwrap_or_default();
        let entry = CacheEntry {
            query: query.to_string(),
            reply: reply.to_string(),
            embedding: q_emb,
        };
        {
            let mut guard = self.inner.map.lock().unwrap();
            guard.insert(key, entry);
        }
        self.persist();
    }

    // 离线写入：不调用 LLM embedding（离线无 key），仅存 query/reply 供精确命中复用。
    // 语义近邻因 embedding 为空会自动跳过，不影响在线已缓存项的近邻匹配。
    // 用于离线模式下 FAQ 命中 / RAG 命中的中文问答，使重复提问可直接命中缓存。
    pub async fn put_offline(&self, query: &str, reply: &str) {
        let key = normalize(query);
        let entry = CacheEntry {
            query: query.to_string(),
            reply: reply.to_string(),
            embedding: Vec::new(),
        };
        {
            let mut guard = self.inner.map.lock().unwrap();
            guard.insert(key, entry);
        }
        self.persist();
    }

    fn persist(&self) {
        let guard = self.inner.map.lock().unwrap();
        let cf = CacheFile {
            version: self.inner.version.clone(),
            entries: guard.values().cloned().collect(),
        };
        match serde_json::to_string_pretty(&cf) {
            Ok(s) => {
                if let Err(e) = std::fs::write(&self.inner.path, s) {
                    tracing::warn!("answer cache persist failed: {}", e);
                }
            }
            Err(e) => tracing::warn!("answer cache serialize failed: {}", e),
        }
    }
}

// 归一化：小写并折叠空白，作为精确命中 key。
fn normalize(q: &str) -> String {
    q.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Serialize, Deserialize)]
struct CacheFile {
    version: String,
    entries: Vec<CacheEntry>,
}

// 仅供无 key 离线场景的便捷构造（不持久化、不写缓存）。
impl AnswerCache {
    pub fn empty() -> Self {
        Self {
            inner: Arc::new(AnswerCacheInner {
                version: String::new(),
                map: Mutex::new(HashMap::new()),
                path: PathBuf::from(CACHE_FILE),
            }),
        }
    }
}
