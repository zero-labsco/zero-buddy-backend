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
    pub reply: String,         // 回答
    #[serde(default)] // 可选：命中后可随回答一并返回前端的来源链接（邮箱/官网等）
    pub url: Option<String>,
}

// 匹配方式默认值：contains
fn default_match() -> String {
    "contains".to_string()
}

/// 判断查询是否命中某个触发词。
///
/// - 单个 ASCII 单词（如 "hi"/"ty"/"yo"）：用**词边界**匹配，
///   避免子串误报 —— 例如 "which" 含 "hi"、"support" 含 "sup"、"city" 含 "ty"、
///   "your" 含 "yo"，会被误当成打招呼/道谢。
/// - 多词短语与含 CJK 的触发词：保留子串匹配（CJK 无空格分词，词边界不适用；
///   短语如 "good morning" 以整串命中）。
fn pattern_hit(query: &str, pattern: &str) -> bool {
    let p = pattern.to_lowercase();
    let is_single_ascii_word = !p.is_empty()
        && p.chars().all(|c| c.is_ascii_alphanumeric())
        && !p.contains(char::is_whitespace);
    if is_single_ascii_word {
        // 用 \b 锚定整个单词；正则构造失败时退化为子串匹配，保证不误伤
        let re = regex::Regex::new(&format!(r"\b{}\b", regex::escape(&p)));
        match re {
            Ok(re) => re.is_match(query),
            Err(_) => query.contains(&p),
        }
    } else {
        query.contains(&p)
    }
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

        // 语言偏好：检测查询是否含 CJK（中日韩）字符，含则优先匹配中文条目，
        // 否则优先匹配非中文条目。这样可避免中英文共享同一 pattern 时误命中错语言。
        let prefer_cjk = query.chars().any(|c| {
            ('\u{4e00}'..='\u{9fff}').contains(&c)
                || ('\u{3040}'..='\u{30ff}').contains(&c)
                || ('\u{ac00}'..='\u{d7af}').contains(&c)
        });

        // 两遍扫描：第一遍仅看偏好语言的条目，第二遍回退到全部条目
        for pass in 0..2 {
            for rule in &self.rules {
                let rule_is_cjk = rule
                    .reply
                    .chars()
                    .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c));
                if pass == 0 && rule_is_cjk != prefer_cjk {
                    continue; // 第一遍只匹配偏好语言
                }
                let hit = match rule.r#match.as_str() {
                    "starts_with" => {
                        // 首词匹配：查询的首词等于某 pattern，或查询以 "pattern " 开头
                        let first = q.split_whitespace().next().unwrap_or("");
                        rule.patterns.iter().any(|p| {
                            let p = p.to_lowercase();
                            first == p || q == p || q.starts_with(&format!("{p} "))
                        })
                    }
                    "regex" => rule.patterns.iter().any(|p| pattern_hit(q, p)),
                    _ => {
                        // 默认 contains：查询包含任意 pattern 即命中
                        // （单个 ASCII 单词用词边界匹配，见 pattern_hit）
                        rule.patterns.iter().any(|p| pattern_hit(q, p))
                    }
                };
                if hit {
                    // 仅当 url 非空才随回答带回，避免向前端传递空链接
                    let url = rule.url.as_ref().filter(|u| !u.trim().is_empty()).cloned();
                    return Some((rule.reply.clone(), url));
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: &str, r#match: &str, patterns: &[&str], reply: &str) -> FaqRule {
        FaqRule {
            id: id.to_string(),
            r#match: r#match.to_string(),
            patterns: patterns.iter().map(|s| s.to_string()).collect(),
            reply: reply.to_string(),
            url: None,
        }
    }

    fn store() -> FaqStore {
        FaqStore {
            rules: vec![
                rule(
                    "greeting-en",
                    "contains",
                    &["hello", "hi", "hey", "yo", "sup"],
                    "Hi there!",
                ),
                rule(
                    "thanks-en",
                    "contains",
                    &["thank", "thanks", "ty"],
                    "You're welcome!",
                ),
                rule(
                    "install-en",
                    "contains",
                    &[
                        "how to install zero inspector kit",
                        "setup zero inspector kit",
                    ],
                    "Install steps here",
                ),
                rule("goodbye-en", "starts_with", &["bye", "goodbye"], "See you!"),
                rule("greeting-zh", "contains", &["你好", "哈喽"], "你好呀！"),
            ],
        }
    }

    // —— pattern_hit 词边界修复：短 ASCII 单词不得命中其所在单词的子串 ——
    #[test]
    fn hi_does_not_match_substring_inside_other_words() {
        // 回归：这些词含 "hi"/"which" 子串，但不应命中打招呼
        assert!(!pattern_hit("which product does zero labs make", "hi"));
        assert!(!pattern_hit(
            "describe the architecture of zero buddy",
            "hi"
        ));
        assert!(!pattern_hit("caching layers", "hi"));
    }

    #[test]
    fn hi_matches_standalone_word() {
        assert!(pattern_hit("hi there", "hi"));
        assert!(pattern_hit("just say hi to the team", "hi"));
    }

    #[test]
    fn sup_does_not_match_support() {
        assert!(!pattern_hit("customer support email", "sup"));
        assert!(pattern_hit("sup how are you", "sup"));
    }

    #[test]
    fn ty_does_not_match_city() {
        assert!(!pattern_hit("what is the city of invoice zero", "ty"));
        assert!(pattern_hit("ty for your help", "ty"));
    }

    #[test]
    fn multiword_phrase_still_substring() {
        // 多词短语保持子串匹配：整串出现即命中
        assert!(pattern_hit(
            "please tell me how to install zero inspector kit",
            "how to install zero inspector kit"
        ));
    }

    // —— FaqStore::answer 端到端：问候语不再误伤技术问题 ——
    #[test]
    fn greeting_does_not_hijack_technical_questions() {
        let s = store();
        assert!(s
            .answer("What does Zero Buddy's architecture look like?")
            .is_none());
        assert!(s.answer("Which products does Zero Labs make?").is_none());
    }

    #[test]
    fn greeting_matches_real_hello() {
        let s = store();
        let got = s.answer("Hi, are you there?");
        assert_eq!(got.as_ref().map(|(r, _)| r.as_str()), Some("Hi there!"));
    }

    #[test]
    fn install_phrase_matches() {
        let s = store();
        let got = s.answer("How to install Zero Inspector Kit?");
        assert_eq!(
            got.as_ref().map(|(r, _)| r.as_str()),
            Some("Install steps here")
        );
    }

    #[test]
    fn starts_with_matches_first_word() {
        let s = store();
        let got = s.answer("Goodbye my friend");
        assert_eq!(got.as_ref().map(|(r, _)| r.as_str()), Some("See you!"));
    }

    #[test]
    fn cjk_patterns_still_match() {
        let s = store();
        let got = s.answer("你好，Zero Buddy 在吗？");
        assert_eq!(got.as_ref().map(|(r, _)| r.as_str()), Some("你好呀！"));
    }
}
