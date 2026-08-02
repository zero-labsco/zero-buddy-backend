// 运行时配置，全部来自环境变量。
// 集中在一处管理，方便日后接入配置中心（etcd/Consul）做分布式部署。
#[derive(Debug, Clone)]
pub struct Config {
    pub llm_base_url: String,  // LLM / 向量化接口地址（OpenAI 兼容）
    pub llm_api_key: String,   // API 密钥（缺失则进入离线模式）
    pub llm_model: String,     // 对话模型名
    pub embed_model: String,   // 向量化模型名
    pub cors_origin: String,   // 允许跨域的前端来源（默认本地前端）
    pub cache_version: String, // 知识库版本；变更后可使答案缓存整体失效

    // —— 品牌与文案（改名只改这里，无需动代码）——
    pub product_name: String, // 助手/产品名，如 ZeroBuddy
    pub org_name: String,     // 所属组织名，如 Zero Labs

    // —— 可调参数（配置化，便于调优而无需重新编译）——
    pub rag_top_k: usize,         // RAG 最多取几条文档
    pub rag_min_score: f32,       // RAG 相似度阈值
    pub cache_similarity: f32,    // 语义缓存近邻阈值
    pub max_message_chars: usize, // 单条消息最大字符数
    pub max_messages: usize,      // 消息条数上限

    // —— 话题范围拦截（Scope Guard）：防止被用于回答与项目无关的问题，
    //     既避免浪费 LLM token，也防止被恶意当作通用 AI 使用 ——
    pub scope_guard_enabled: bool,   // 总开关：是否启用话题范围限制
    pub scope_guard_mode: ScopeMode, // prompt=靠 LLM 自觉遵守；hard=后端硬拦截（省 token）
    pub scope_allow_keywords: Vec<String>, // 白名单触发词，命中即放行（大小写不敏感）
    pub scope_allow_patterns: Vec<String>, // 白名单正则，命中即放行（如产品名、邮箱、官网域名）
    pub scope_refuse_reply_zh: String, // 超范围时返回的中文拒绝话术
    pub scope_refuse_reply_en: String, // 超范围时返回的英文拒绝话术

    // —— 使用限制（防滥用 / 防刷 / 控成本）——
    pub rate_limit_per_min: u32, // 同一客户端每分钟最多请求数（0=不限制）
    pub rate_limit_per_day: u32, // 同一客户端每天最多请求数（0=不限制）
    pub rate_limit_reply_zh: String, // 触发限制时返回的中文提示
    pub rate_limit_reply_en: String, // 触发限制时返回的英文提示
}

/// 话题范围限制的工作模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScopeMode {
    /// 在 system prompt 中声明范围策略，由 LLM 自觉遵守（轻量，依赖模型配合）。
    #[default]
    Prompt,
    /// 后端先做关键词白名单硬判断，超范围直接返回拒绝语，不调用 LLM（省 token，强制生效）。
    Hard,
}

impl ScopeMode {
    fn from_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "hard" => ScopeMode::Hard,
            _ => ScopeMode::Prompt,
        }
    }
}

impl Default for Config {
    /// 返回一套内置默认值，覆盖本地开发最常用的配置。
    /// 环境变量读取时以此为基准，仅在对应变量非空时才覆盖。
    fn default() -> Self {
        Self {
            llm_base_url: "https://api.openai.com/v1".into(),
            llm_api_key: String::new(),
            llm_model: "gpt-4o-mini".into(),
            embed_model: "text-embedding-3-small".into(),
            cors_origin: "http://localhost:3040".into(),
            cache_version: "v1".into(),
            product_name: "ZeroBuddy".into(),
            org_name: "Zero Labs".into(),
            rag_top_k: 3,
            rag_min_score: 0.2,
            cache_similarity: 0.92,
            max_message_chars: 4000,
            max_messages: 50,
            scope_guard_enabled: true,
            scope_guard_mode: ScopeMode::Prompt,
            scope_allow_keywords: vec![
                // 品牌 / 组织
                "zero".into(),
                "zerolabs".into(),
                "labs".into(),
                // 产品名
                "flutter agent kit".into(),
                "flutteragentkit".into(),
                "flutter".into(),
                "zero inspector kit".into(),
                "inspector".into(),
                "wizardplayer".into(),
                "invoice zero".into(),
                "invoice".into(),
                "zerobuddy".into(),
                "buddy".into(),
                // 技术 / 文档相关
                "kit".into(),
                "sdk".into(),
                "api".into(),
                "文档".into(),
                "部署".into(),
                "配置".into(),
                "安装".into(),
                "教程".into(),
                "使用".into(),
                "功能".into(),
                "报价".into(),
                "价格".into(),
                "license".into(),
                "licence".into(),
                "开源".into(),
                "仓库".into(),
                "github".into(),
                "support".into(),
                "联系".into(),
            ],
            scope_allow_patterns: vec![
                // 产品名变体（含连字符/无空格）
                r"(?i)zero[\s-]?(inspector|flutter[\s-]?agent|invoice)?[\s-]?kit".into(),
                r"(?i)wizard[\s-]?player".into(),
                r"(?i)invoice[\s-]?zero".into(),
                r"(?i)zero[\s-]?buddy".into(),
                // 官网 / 邮箱 / 仓库
                r"(?i)zerolabsco\.com".into(),
                r"(?i)github\.com/zero-labsco".into(),
                r"(?i)support@zerolabsco\.com".into(),
                // 通用技术栈（与本产品技术相关）
                r"(?i)\b(dart|rust|next\.?js|react|axum|tokio|cors|llm|embedding|rag|openai)\b".into(),
            ],
            scope_refuse_reply_zh:
                "抱歉，我只能回答与 Zero Labs 产品（如 Zero Inspector Kit、Flutter Agent Kit、WizardPlayer、Invoice Zero）相关的问题。其他话题请联系 support@zerolabsco.com。"
                    .into(),
            scope_refuse_reply_en:
                "Sorry, I can only help with questions about Zero Labs products (e.g. Zero Inspector Kit, Flutter Agent Kit, WizardPlayer, Invoice Zero). For anything else, please contact support@zerolabsco.com."
                    .into(),
            rate_limit_per_min: 10,
            rate_limit_per_day: 500,
            rate_limit_reply_zh:
                "您请求过于频繁，请稍后再试（每分钟/每天上限）。如需更高额度请联系 support@zerolabsco.com。"
                    .into(),
            rate_limit_reply_en:
                "Too many requests. Please slow down (per-minute / per-day limit reached). For higher limits contact support@zerolabsco.com."
                    .into(),
        }
    }
}

impl Config {
    // 从环境变量读取配置；LLM_API_KEY 现在可选——缺失时进入离线模式。
    pub fn from_env() -> anyhow::Result<Self> {
        let mut cfg = Config::default();
        if let Ok(v) = std::env::var("LLM_BASE_URL") {
            if !v.is_empty() {
                cfg.llm_base_url = v;
            }
        }
        // 兼容旧名 LLM_API_KEY 与新名（保持向后兼容）
        cfg.llm_api_key = std::env::var("LLM_API_KEY").unwrap_or_default();
        if let Ok(v) = std::env::var("LLM_MODEL") {
            if !v.is_empty() {
                cfg.llm_model = v;
            }
        }
        if let Ok(v) = std::env::var("EMBED_MODEL") {
            if !v.is_empty() {
                cfg.embed_model = v;
            }
        }
        if let Ok(v) = std::env::var("CORS_ORIGIN") {
            if !v.is_empty() {
                cfg.cors_origin = v;
            }
        }
        if let Ok(v) = std::env::var("CACHE_VERSION") {
            if !v.is_empty() {
                cfg.cache_version = v;
            }
        }
        if let Ok(v) = std::env::var("PRODUCT_NAME") {
            if !v.is_empty() {
                cfg.product_name = v;
            }
        }
        if let Ok(v) = std::env::var("ORG_NAME") {
            if !v.is_empty() {
                cfg.org_name = v;
            }
        }
        cfg.rag_top_k = read_usize("RAG_TOP_K", cfg.rag_top_k);
        cfg.rag_min_score = read_f32("RAG_MIN_SCORE", cfg.rag_min_score);
        cfg.cache_similarity = read_f32("CACHE_SIMILARITY", cfg.cache_similarity);
        cfg.max_message_chars = read_usize("MAX_MESSAGE_CHARS", cfg.max_message_chars);
        cfg.max_messages = read_usize("MAX_MESSAGES", cfg.max_messages);

        // —— 话题范围拦截（Scope Guard）配置 ——
        if let Ok(v) = std::env::var("SCOPE_GUARD_ENABLED") {
            if !v.is_empty() {
                cfg.scope_guard_enabled = v == "1" || v.eq_ignore_ascii_case("true");
            }
        }
        if let Ok(v) = std::env::var("SCOPE_GUARD_MODE") {
            if !v.is_empty() {
                cfg.scope_guard_mode = ScopeMode::from_str(&v);
            }
        }
        if let Ok(v) = std::env::var("SCOPE_ALLOW_KEYWORDS") {
            let kws: Vec<String> = v
                .split(',')
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect();
            if !kws.is_empty() {
                cfg.scope_allow_keywords = kws;
            }
        }
        if let Ok(v) = std::env::var("SCOPE_REFUSE_REPLY_ZH") {
            if !v.is_empty() {
                cfg.scope_refuse_reply_zh = v;
            }
        }
        if let Ok(v) = std::env::var("SCOPE_REFUSE_REPLY_EN") {
            if !v.is_empty() {
                cfg.scope_refuse_reply_en = v;
            }
        }
        if let Ok(v) = std::env::var("SCOPE_ALLOW_PATTERNS") {
            let pats: Vec<String> = v
                .split('|')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !pats.is_empty() {
                cfg.scope_allow_patterns = pats;
            }
        }
        if let Ok(v) = std::env::var("RATE_LIMIT_PER_MIN") {
            if let Ok(n) = v.parse::<u32>() {
                cfg.rate_limit_per_min = n;
            }
        }
        if let Ok(v) = std::env::var("RATE_LIMIT_PER_DAY") {
            if let Ok(n) = v.parse::<u32>() {
                cfg.rate_limit_per_day = n;
            }
        }
        if let Ok(v) = std::env::var("RATE_LIMIT_REPLY_ZH") {
            if !v.is_empty() {
                cfg.rate_limit_reply_zh = v;
            }
        }
        if let Ok(v) = std::env::var("RATE_LIMIT_REPLY_EN") {
            if !v.is_empty() {
                cfg.rate_limit_reply_en = v;
            }
        }

        // 关键校验：仅当 URL 看起来像是合法 http(s) 时才接受
        if !cfg.llm_base_url.starts_with("http://") && !cfg.llm_base_url.starts_with("https://") {
            anyhow::bail!("LLM_BASE_URL must start with http:// or https://");
        }
        Ok(cfg)
    }

    // 是否配置了有效的 API 密钥（占位符视为无效）。
    // 无效时进入离线模式，直接返回检索到的知识，避免调用接口报错。
    pub fn has_valid_key(&self) -> bool {
        !self.llm_api_key.is_empty() && self.llm_api_key != "sk-your-key-here"
    }

    // 判断查询是否在允许的话题范围内（hard 模式使用）。
    // 命中任一白名单关键词（大小写不敏感、子串匹配）或任一白名单正则即视为放行。
    // 关键词与正则都为空时视为“全部放行”（避免误伤）。
    pub fn in_scope(&self, query: &str) -> bool {
        if self.scope_allow_keywords.is_empty() && self.scope_allow_patterns.is_empty() {
            return true;
        }
        let q = query.to_lowercase();
        if self.scope_allow_keywords.iter().any(|kw| q.contains(kw)) {
            return true;
        }
        self.scope_allow_patterns.iter().any(|pat| {
            regex::Regex::new(pat)
                .map(|re| re.is_match(query))
                .unwrap_or(false)
        })
    }

    // 单次 LLM 调用的超时时间（秒），用于防止接口卡死。
    pub fn request_timeout_secs(&self) -> u64 {
        std::env::var("REQUEST_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30)
    }
}

// 读取 usize 类型的可选环境变量，读取失败回退默认值
fn read_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

// 读取 f32 类型的可选环境变量，读取失败回退默认值
fn read_f32(key: &str, default: f32) -> f32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
