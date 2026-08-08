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
        let emb: Option<&Vec<f32>> = if d.embedding.is_empty() {
            None
        } else {
            Some(&d.embedding)
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_identical_vectors_is_one() {
        let v = [1.0, 2.0, 3.0, 4.0];
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_orthogonal_is_zero() {
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
    }

    #[test]
    fn cosine_zero_vector_is_zero() {
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
        assert_eq!(cosine(&[1.0, 1.0], &[0.0, 0.0]), 0.0);
    }

    #[test]
    fn cosine_similar_vs_dissimilar() {
        // 同向向量相似度高于反向向量
        let a = [1.0, 1.0, 1.0];
        let b = [2.0, 2.0, 2.0]; // 与 a 同向
        let c = [-1.0, -1.0, -1.0]; // 与 a 反向
        assert!(cosine(&a, &b) > cosine(&a, &c));
        assert!((cosine(&a, &c) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn tokenize_lowercases_and_filters_short() {
        let t = tokenize("Hello Flutter and Dart");
        assert!(t.contains("hello"));
        assert!(t.contains("flutter"));
        assert!(t.contains("dart"));
        assert!(t.contains("and"), "3 字母词 'and' 应被保留");
        assert!(!t.contains("he"), "2 字母短词应被过滤");
    }

    #[test]
    fn score_keyword_overlap_ratio() {
        // 全部重叠 -> 1.0
        assert_eq!(
            score_keyword("zero buddy install", "zero buddy install"),
            1.0
        );
        // 重叠 2/3（zero、buddy 命中）-> 2/3
        let s = score_keyword("zero buddy deploy", "zero buddy install");
        assert!((s - 2.0 / 3.0).abs() < 1e-6, "实际 = {s}");
        // 无重叠 -> 0
        assert_eq!(score_keyword("abc def", "xyz qrs"), 0.0);
    }

    #[test]
    fn score_keyword_empty_query_is_zero() {
        assert_eq!(score_keyword("anything here", ""), 0.0);
    }
}
