// 路由模块入口：组合所有子路由，最外层统一注入 AppState。
mod chat;
mod health;

use crate::state::AppState;
use axum::Router;

/// 组装完整路由树：合并各子路由模块（chat / health），
/// 并在最外层统一注入共享 AppState，供所有 handler 通过 State 提取。
pub fn create_router(state: AppState) -> Router {
    let api = Router::new().merge(chat::router()).merge(health::router());
    api.with_state(state)
}
