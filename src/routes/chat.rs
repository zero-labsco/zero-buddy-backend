//! chat 模块的路由定义与请求处理器。
use axum::{
    extract::State,
    routing::post,
    Json, Router,
};
use crate::chat::handle_chat;
use crate::error::{ApiCode, ApiError, ApiResult};
use crate::models::ChatRequest;
use crate::state::AppState;
use serde_json::Value;
use tracing::warn;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/chat", post(chat_handler))
}

/// POST /api/chat 的处理器：调用核心编排 handle_chat，
/// 成功时包成统一信封 ApiResult，业务/校验错误统一映射为 ApiError(BadRequest)。
#[tracing::instrument(skip(state, req), fields(source = "api/chat"))]
async fn chat_handler(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ApiResult<Value>>, ApiError> {
    match handle_chat(&state.cfg, &state.client, &state.cache, state.online.clone(), req).await {
        Ok(body) => Ok(Json(ApiResult::ok(body))),
        Err(e) => {
            warn!("chat error: {:#}", e);
            // 业务校验错误统一映射为 BadRequest，文案带具体原因
            Err(ApiError::with_msg(ApiCode::BadRequest, e.to_string()))
        }
    }
}
