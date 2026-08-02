use crate::llm::LlmClient;
use crate::models::Document;
use anyhow::Result;
use std::collections::HashSet;

// 检索：把查询与知识库做相似度打分。
// 注意：向量相似度用于排序，关键词相似度作为兜底（避免向量模型把同义词打低分）。

pub async fn retrieve_scored(
    client: &LlmClient,
    docs: &[Document],
    query: &str,
) -> Result<(Vec<(usize, f32)>, Option<Vec<f32>>)> {
    let q_embed = client.embed(query).await.ok();
    let mut scored: Vec<(usize, f32)> = Vec::new();
    for (i, d) in docs.iter().enumerate() {
        let emb: Option<&Vec<f32>> = if d.embedding.is_empty() { None } else { Some(&d.embedding) };
        let score = match (&q_embed, emb) {
            (Some(q), Some(e)) => cosine(q, e),
            _ => 0.0,
        };
        let kw = score_keyword(&d.content, query);
        let combined = score.max(kw); // 取两者较高值，提升召回
        scored.push((i, combined));
    }
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    Ok((scored, q_embed))
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..n {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

// 关键词相似度：Jaccard 风格的 token 重叠率（针对短查询做兜底召回）。
pub fn score_keyword(content: &str, query: &str) -> f32 {
    let ct: HashSet<String> = tokenize(content);
    let qt: HashSet<String> = tokenize(query);
    if qt.is_empty() {
        return 0.0;
    }
    let inter = ct.intersection(&qt).count() as f32;
    inter / qt.len() as f32
}

pub fn tokenize(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() > 2)
        .map(|s| s.to_string())
        .collect()
}
