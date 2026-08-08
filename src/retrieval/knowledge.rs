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

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(id: &str, title: &str, content: &str) -> Document {
        Document {
            id: id.to_string(),
            project: String::new(),
            title: title.to_string(),
            content: content.to_string(),
            url: None,
            embedding: Vec::new(),
        }
    }

    #[test]
    fn retrieve_formats_matched_docs_in_order() {
        let docs = vec![
            doc("a", "Alpha", "alpha content"),
            doc("b", "Beta", "beta content"),
            doc("c", "Gamma", "gamma content"),
        ];
        let out = retrieve(&docs, &[0, 2]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], "# Alpha\nalpha content");
        assert_eq!(out[1], "# Gamma\ngamma content");
    }

    #[test]
    fn retrieve_skips_out_of_range_indices() {
        let docs = vec![doc("a", "Alpha", "alpha content")];
        let out = retrieve(&docs, &[0, 99]);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn retrieve_empty_indices_yields_empty() {
        let docs = vec![doc("a", "Alpha", "alpha content")];
        assert!(retrieve(&docs, &[]).is_empty());
    }
}
