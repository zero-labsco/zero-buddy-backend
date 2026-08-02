use anyhow::Context;

use crate::models::Document;

// 从 JSON 文件加载原始知识库（文档列表）。
// 注：向量已合并进 knowledge.json 的 embedding 字段（见 README 的 build 步骤），
// 因此这里直接得到带 embedding 的 Document，无需再单独读 embeddings.json。
pub fn load(path: &str) -> anyhow::Result<Vec<Document>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read knowledge file: {path}"))?;
    let docs: Vec<Document> = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse knowledge file: {path}"))?;
    Ok(docs)
}

// 把命中的若干文档按给定索引拼成上下文字符串。
pub fn retrieve(docs: &[Document], indices: &[usize]) -> Vec<String> {
    indices
        .iter()
        .filter_map(|&i| docs.get(i))
        .map(|d| format!("# {}\n{}", d.title, d.content))
        .collect()
}
