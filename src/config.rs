// 运行时配置，全部来自环境变量。
// 集中在一处管理，方便日后接入配置中心（etcd/Consul）做分布式部署。
#[derive(Debug, Clone)]
pub struct Config {
    pub llm_base_url: String,  // LLM / 向量化接口地址（OpenAI 兼容）
    pub llm_api_key: String,   // API 密钥（缺失则进入离线模式）
    pub llm_model: String,    // 对话模型名
    pub embed_model: String,   // 向量化模型名
    pub cors_origin: String,  // 允许跨域的前端来源（默认本地前端）
    pub cache_version: String, // 知识库版本；变更后可使答案缓存整体失效

    // —— 品牌与文案（改名只改这里，无需动代码）——
    pub product_name: String, // 助手/产品名，如 ZeroBuddy
    pub org_name: String,     // 所属组织名，如 Zero Labs

    // —— 可调参数（配置化，便于调优而无需重新编译）——
    pub rag_top_k: usize,           // RAG 最多取几条文档
    pub rag_min_score: f32,         // RAG 相似度阈值
    pub cache_similarity: f32,      // 语义缓存近邻阈值
    pub max_message_chars: usize,   // 单条消息最大字符数
    pub max_messages: usize,        // 消息条数上限
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
