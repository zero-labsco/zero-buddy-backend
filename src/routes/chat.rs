//! chat 模块的路由定义与请求处理器。
use crate::chat::handle_chat;
use crate::error::{ApiCode, ApiError, ApiResult};
use crate::models::ChatRequest;
use crate::state::AppState;
use axum::extract::connect_info::ConnectInfo;
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::Value;
use std::net::SocketAddr;
use tracing::warn;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/chat", post(chat_handler))
}

/// POST /api/chat 的处理器：调用核心编排 handle_chat，
/// 成功时包成统一信封 ApiResult，业务/校验错误统一映射为 ApiError(BadRequest)。
#[tracing::instrument(skip(state, req), fields(source = "api/chat"))]
async fn chat_handler(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ApiResult<Value>>, ApiError> {
    // 取真实客户端 IP（优先 X-Forwarded-For 首段，适合反向代理后部署；
    // 直连时回退到 socket 地址）。这里直接用 socket 地址即可，
    // 若前端经 nginx 反代，可在 nginx 设置 X-Forwarded-For 并解析。
    let ip = addr.ip().to_string();
    match handle_chat(
        &state.cfg,
        &state.client,
        &state.cache,
        state.online.clone(),
        &state.rate_limiter,
        &ip,
        req,
    )
    .await
    {
        Ok(body) => Ok(Json(ApiResult::ok(body))),
        Err(e) => {
            warn!("chat error: {:#}", e);
            // 业务校验错误统一映射为 BadRequest，文案带具体原因
            Err(ApiError::with_msg(ApiCode::BadRequest, e.to_string()))
        }
    }
}
