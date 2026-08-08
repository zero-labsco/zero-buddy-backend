//! 流式聊天路由：POST /api/chat/stream，通过 SSE 把 LLM 生成的 token 实时推给前端。
use crate::chat::handle_chat_stream;
use crate::models::ChatRequest;
use crate::state::AppState;
use axum::extract::connect_info::ConnectInfo;
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use std::net::SocketAddr;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/chat/stream", post(chat_stream_handler))
}

/// POST /api/chat/stream 的处理器：调用 handle_chat_stream 返回 SSE 流。
/// handle_chat_stream 内部已对所有错误（校验/知识库/鉴权）统一以 error 事件结束流，
/// 因此此处无需再处理 Result，直接返回 SSE 即可。
#[tracing::instrument(skip(state, req), fields(source = "api/chat/stream"))]
async fn chat_stream_handler(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<ChatRequest>,
) -> axum::response::sse::Sse<
    std::pin::Pin<
        Box<
            dyn futures_core::Stream<
                    Item = Result<axum::response::sse::Event, std::convert::Infallible>,
                > + Send,
        >,
    >,
> {
    let ip = addr.ip().to_string();
    handle_chat_stream(
        &state.cfg,
        &state.client,
        &state.cache,
        state.online.clone(),
        &state.rate_limiter,
        &ip,
        req,
    )
    .await
}
