// 子模块声明：按职责分组，避免 src 根目录堆积过多文件。
mod chat; // 聊天编排（handle_chat 管线）
mod config; // 配置（Config + ScopeMode）
mod llm; // LLM 客户端（LlmClient）
mod logging; // 日志初始化
mod models; // 共享数据模型（ChatMessage / ChatRequest / Document）
mod rate_limit; // 速率限制（RateLimiter）
mod response; // 统一响应信封（ApiResult / ApiCode / ApiError）
mod retrieval; // 知识检索：knowledge / rag / faq / cache / 运行时联网兜底
mod routes; // HTTP 路由（chat / health）
mod state; // 应用共享状态（AppState）

use axum::http::Method;
use axum::serve::serve;
use config::Config;
use llm::LlmClient;
use retrieval::AnswerCache;
use state::AppState;
use std::net::SocketAddr;
use std::time::Duration;
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tracing::info;

#[tokio::main]
async fn main() {
    // 结构化日志：控制台 + logs/ 滚动文件，可用 RUST_LOG 控制级别
    logging::init();

    // 加载配置；.env 若存在则读取
    let _ = dotenvy::dotenv();
    let cfg = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("config error: {:#}", e);
            std::process::exit(1);
        }
    };

    let client = LlmClient::new(cfg.clone());

    // 决定在线/离线：配了 key 则启动探测其合法性，探测失败降级离线；
    // 未配 key 直接离线。离线模式不加载/回写答案缓存，直接返回知识库内容。
    let (online, cache) = if cfg.has_valid_key() {
        info!("LLM_API_KEY present -> verifying key at startup...");
        if client.check_key().await {
            info!("LLM key OK -> ONLINE mode (AI answers enabled)");
            (true, AnswerCache::load(&cfg))
        } else {
            tracing::warn!(
                "LLM_API_KEY invalid (auth failed) -> OFFLINE mode (returning knowledge base only)"
            );
            (false, AnswerCache::empty())
        }
    } else {
        tracing::warn!("LLM_API_KEY not set -> OFFLINE mode (no AI answers, no cache writes)");
        (false, AnswerCache::empty())
    };

    let state = AppState {
        cfg: cfg.clone(),
        client,
        cache,
        online: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(online)),
        rate_limiter: rate_limit::RateLimiter::new(),
    };

    // CORS：收敛到指定前端来源，而非任意来源
    let cors = if cfg.cors_origin == "*" {
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods([Method::POST, Method::GET])
            .allow_headers(Any)
    } else {
        let origin = cfg
            .cors_origin
            .split(',')
            .map(|s| s.trim().to_string())
            .collect::<Vec<_>>();
        CorsLayer::new()
            .allow_origin(
                origin
                    .iter()
                    .filter_map(|o| o.parse::<axum::http::HeaderValue>().ok())
                    .collect::<Vec<_>>(),
            )
            .allow_methods([Method::POST, Method::GET])
            .allow_headers(Any)
    };

    let app = routes::create_router(state)
        .layer(cors)
        // 限制请求体大小，防止超大 payload
        .layer(RequestBodyLimitLayer::new(1_000_000)) // 1MB
        // 全局请求超时，防止下游接口卡死拖垮服务
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(cfg.request_timeout_secs() + 5),
        ))
        // 让 handler 能拿到真实客户端 IP（ConnectInfo<SocketAddr>）
        .into_make_service_with_connect_info::<SocketAddr>();

    let bind = std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:3031".into());
    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("failed to bind {}: {}", bind, e);
            std::process::exit(1);
        }
    };
    info!("{} backend listening on http://{}", cfg.product_name, bind);
    print_banner(&cfg.product_name, &bind, online);

    // 优雅关闭：同时等待服务运行与关闭信号（Ctrl+C / SIGTERM）。
    // 收到信号时主动退出，打印友好提示并以状态码 0 结束，
    // 避免 Windows 上出现 STATUS_CONTROL_C_EXIT (0xc000013a) 的红色报错，
    // 也支持 Linux 服务器经 systemctl stop / docker stop 发来的 SIGTERM。
    tokio::select! {
        res = serve(listener, app) => {
            if let Err(e) = res {
                tracing::error!("server error: {:#}", e);
                std::process::exit(1);
            }
        }
        signal = shutdown_signal() => {
            println!("\n\x1b[2m  received {} signal, shutting down gracefully. Bye.\x1b[0m", signal);
        }
    }
}

/// 跨平台关闭信号：返回触发信号名，供日志/提示使用。
/// - Unix：同时监听 SIGINT（Ctrl+C）与 SIGTERM（systemctl stop / docker stop）。
/// - Windows：tokio 的 ctrl_c() 覆盖 Ctrl+C；SIGTERM 在 Windows 下无原生等价，
///   用 SetConsoleCtrlHandler 监听 Ctrl+Break/关闭事件近似替代（此处退回 ctrl_c）。
async fn shutdown_signal() -> &'static str {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigint = signal(SignalKind::interrupt()).expect("failed to install SIGINT handler");
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = sigint.recv() => "SIGINT",
            _ = sigterm.recv() => "SIGTERM",
        }
    }
    #[cfg(not(unix))]
    {
        // Windows：Ctrl+C 由 tokio 处理（等价于 SIGINT）
        let _ = tokio::signal::ctrl_c().await;
        "SIGINT"
    }
}

/// 启动成功后打印的 ASCII 横幅：从 banner.txt 读取大字 + 监听地址与运行模式。
/// banner.txt 不存在时回退到内置默认大字，保证服务始终能启动。
fn print_banner(product: &str, bind: &str, online: bool) {
    let mode = if online {
        "ONLINE · AI answers enabled"
    } else {
        "OFFLINE · knowledge base only"
    };
    // 版本号从 Cargo.toml 的 [package].version 注入，编译期确定。
    let version = env!("CARGO_PKG_VERSION");
    // 优先读取 banner.txt（可用记事本随时修改大字样式）；读取失败则用内置回退。
    let raw = std::fs::read_to_string("banner.txt").unwrap_or_else(|_| DEFAULT_BANNER.to_string());
    // 给大字上青绿色；其余行（端口/模式）由下方模板单独上色。
    let art = raw
        .lines()
        .map(|l| format!("\x1b[36m{}\x1b[0m", l))
        .collect::<Vec<_>>()
        .join("\n");
    println!(
        "\n{}\n\n\x1b[1m  {:<54}\x1b[0m\n\x1b[2m  version   : {:<38}\x1b[0m\n\x1b[2m  listening : http://{:<38}\x1b[0m\n\x1b[2m  mode      : {:<38}\x1b[0m\n",
        art, product, version, bind, mode
    );
}

/// banner.txt 缺失时的内置回退大字（与 banner.txt 保持一致）。
const DEFAULT_BANNER: &str = "\
________  ________  _______    ______         __         ______   _______    ______
|        \\\\|        \\\\|       \\\\  /      \\\\       |  \\\\       /      \\\\ |       \\\\  /      \\\\
 \\$$$$$$$$| $$$$$$$$| $$$$$$$\\\\|  $$$$$$\\\\      | $$      |  $$$$$$\\\\| $$$$$$\\\\|  $$$$$$\\\\
    /  $$ | $$__    | $$__| $$| $$  | $$      | $$      | $$__| $$| $$__/ $$| $$___\\$$
   /  $$  | $$  \\\\   | $$    $$| $$  | $$      | $$      | $$    $$| $$    $$ \\$$    \\\\
  /  $$   | $$$$$   | $$$$$$\\\\| $$  | $$      | $$      | $$$$$$$$| $$$$$$\\\\ _\\$$$$$$\\
 /  $$___ | $$_____ | $$  | $$| $$__/ $$      | $$_____ | $$  | $$| $$__/ $$|  \\__| $$
|  $$    \\\\| $$     \\\\| $$  | $$ \\$$    $$      | $$     \\\\| $$  | $$| $$    $$ \\$$    $$
 \\$$$$$$$$ \\$$$$$$$$ \\$$   \\$$  \\$$$$$$        \\$$$$$$$$ \\$$   \\$$ \\$$$$$$   \\$$$$$$";
