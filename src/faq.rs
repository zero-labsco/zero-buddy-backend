use anyhow::Context;
use serde::Deserialize;

// 单条 FAQ 规则，完全由 data/faq.json 驱动，新增/修改答案无需改代码。
//
// 字段说明：
//   id      : 规则标识
//   match   : 匹配方式 —— "contains"(包含任意词) / "starts_with"(首词匹配) / "regex"(兼容别名，等同 contains)
//   patterns: 触发词列表
//   reply   : 回答文本
#[derive(Debug, Clone, Deserialize)]
pub struct FaqRule {
    #[allow(dead_code)] // id 在 faq.json 中用于标识规则，当前代码未直接读取
    pub id: String,
    #[serde(default = "default_match")]
    pub r#match: String, // 匹配方式，默认 "contains"
    pub patterns: Vec<String>, // 触发词
    pub reply: String,   // 回答
    #[serde(default)]    // 可选：命中后可随回答一并返回前端的来源链接（邮箱/官网等）
    pub url: Option<String>,
}

// 匹配方式默认值：contains
fn default_match() -> String {
    "contains".to_string()
}

// 从 JSON 加载到内存的 FAQ 规则集合
pub struct FaqStore {
    rules: Vec<FaqRule>,
}

impl FaqStore {
    // 从 JSON 文件加载规则；文件缺失或格式错误都直接报错（启动即发现，避免静默失效）
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read FAQ file: {path}"))?;
        let rules: Vec<FaqRule> = serde_json::from_str(&content)
            .with_context(|| format!("failed to parse FAQ file: {path}"))?;
        Ok(Self { rules })
    }

    // 按文件顺序匹配查询：命中第一条即返回其回复与可选链接，未命中返回 None。
    pub fn answer(&self, query: &str) -> Option<(String, Option<String>)> {
        // 统一转小写、去首尾空白，方便不区分大小写匹配
        let q = query.to_lowercase();
        let q = q.trim();

        for rule in &self.rules {
            let hit = match rule.r#match.as_str() {
                "starts_with" => {
                    // 首词匹配：查询的首词等于某 pattern，或查询以 "pattern " 开头
                    let first = q.split_whitespace().next().unwrap_or("");
                    rule.patterns.iter().any(|p| {
                        let p = p.to_lowercase();
                        first == p || q == p || q.starts_with(&format!("{p} "))
                    })
                }
                "regex" => rule.patterns.iter().any(|p| q.contains(&p.to_lowercase())),
                _ => {
                    // 默认 contains：查询包含任意 pattern 即命中
                    rule.patterns.iter().any(|p| q.contains(&p.to_lowercase()))
                }
            };
            if hit {
                // 仅当 url 非空才随回答带回，避免向前端传递空链接
                let url = rule.url.as_ref().filter(|u| !u.trim().is_empty()).cloned();
                return Some((rule.reply.clone(), url));
            }
        }
        None
    }
}
