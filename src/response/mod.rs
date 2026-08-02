use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use serde_json::Value;

// 统一响应信封，对应 Java 的 Result<Body>。
// 成功时 body 承载业务数据，失败时 body 为 null。
#[derive(Serialize)]
pub struct ApiResult<T: Serialize> {
    pub code: u16,
    pub message: String,
    pub body: Option<T>,
}

// 可枚举的接口状态码 + 默认文案。新增错误只需在此追加一项。
// 部分状态码（Unauthorized/NotFound/Internal）当前未直接构造，预留给未来错误处理使用。
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum ApiCode {
    Ok,
    BadRequest,
    Unauthorized,
    NotFound,
    Internal,
}

impl ApiCode {
    /// 返回该状态码对应的 HTTP 数字码（如 200 / 400 / 500）。
    pub fn code(&self) -> u16 {
        match self {
            ApiCode::Ok => 200,
            ApiCode::BadRequest => 400,
            ApiCode::Unauthorized => 401,
            ApiCode::NotFound => 404,
            ApiCode::Internal => 500,
        }
    }

    /// 返回该状态码对应的默认中文文案；调用处可用 ApiError::with_msg 覆盖。
    pub fn default_message(&self) -> &'static str {
        match self {
            ApiCode::Ok => "success",
            ApiCode::BadRequest => "参数错误",
            ApiCode::Unauthorized => "未授权",
            ApiCode::NotFound => "资源不存在",
            ApiCode::Internal => "服务器内部错误",
        }
    }
}

// 失败响应：枚举决定 code，message 默认取枚举文案，但可在调用处覆盖。
pub struct ApiError {
    code: ApiCode,
    message: String,
}

impl ApiError {
    // 用自定义文案覆盖枚举默认（对应 Java: Result.fail(CODE, "自定义msg")）
    pub fn with_msg(code: ApiCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

// 实现 IntoResponse：自动序列化成 { code, message, body: null }
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.code.code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = ApiResult::<()> {
            code: self.code.code(),
            message: self.message,
            body: None,
        };
        (status, Json(body)).into_response()
    }
}

// 便捷构造成功的信封
impl<T: Serialize> ApiResult<T> {
    pub fn ok(body: T) -> Self {
        Self {
            code: ApiCode::Ok.code(),
            message: ApiCode::Ok.default_message().to_string(),
            body: Some(body),
        }
    }
}

// 支持无 body 的成功响应（如健康检查返回 200）
pub fn ok_empty() -> ApiResult<Value> {
    ApiResult {
        code: ApiCode::Ok.code(),
        message: ApiCode::Ok.default_message().to_string(),
        body: None,
    }
}
