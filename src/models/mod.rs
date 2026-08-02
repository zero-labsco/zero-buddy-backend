use serde::{Deserialize, Serialize};

// 单条聊天消息：角色（user/assistant/system）+ 内容
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,    // 消息角色：user / assistant / system
    pub content: String, // 消息正文
}

// 前端发来的对话请求：多轮消息列表
#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>, // 完整对话历史，用于多轮上下文
}

// 知识库中的一条可检索文档（对应 knowledge.json 中的一项）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: String,          // 文档唯一标识（如 org-overview）
    pub project: String,     // 所属项目名
    pub title: String,       // 文档标题
    pub content: String,     // 文档正文
    pub url: Option<String>, // 相关链接（可选）
    #[serde(default)]
    pub embedding: Vec<f32>, // 预计算向量（来自 embeddings.json），可由 build 步骤注入
}
