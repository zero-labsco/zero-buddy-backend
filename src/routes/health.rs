//! health 模块的路由定义与请求处理器。
use crate::response::{ok_empty, ApiResult};
use crate::state::AppState;
use axum::routing::get;
use axum::Json;
use axum::Router;
use serde_json::Value;

pub fn router() -> Router<AppState> {
    Router::new().route("/health", get(health_handler))
}

/// GET /health 的处理器：返回 200 + 空信封，供前端状态点探测后端存活。
async fn health_handler() -> Json<ApiResult<Value>> {
    Json(ok_empty())
}
