use crate::cache::AnswerCache;
use crate::config::Config;
use crate::llm::LlmClient;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

// 共享应用状态：在 handler 之间克隆传递（内部均为 Arc/Mutex，开销很小）。
#[derive(Clone)]
pub struct AppState {
    pub cfg: Config,
    pub client: LlmClient,
    pub cache: AnswerCache,
    // 运行时是否在线（key 已配且启动探测通过）。false 时走离线模式。
    // 用 AtomicBool 以便运行时在「鉴权/额度错误」时动态降级为离线，
    // 对所有后续请求立即生效（Atomic 保证多线程可见性）。
    pub online: Arc<AtomicBool>,
}
