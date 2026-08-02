use std::path::Path;
use tracing_appender::rolling;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

// 初始化日志：同时输出到控制台(stdout) 与 logs/ 目录下的滚动文件。
// 对应 Java 里 SLF4J + Logback(RollingFileAppender) 的效果。
// 日志级别由环境变量 RUST_LOG 控制（如 RUST_LOG=info），缺省为 info。
pub fn init() {
    // 确保 logs 目录存在
    let log_dir = Path::new("logs");
    if !log_dir.exists() {
        let _ = std::fs::create_dir_all(log_dir);
    }

    // 按天滚动的文件 appender：logs/app.YYYY-MM-DD.log
    let file_appender = rolling::daily("logs", "app");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    // guard 必须长期存活，否则后台写线程会随 init() 结束而关闭、日志丢失。
    // 用 leak 让其生命周期贯穿整个进程（日志组件本就是进程级单例）。
    Box::leak(Box::new(guard));

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    // 控制台层：带颜色、可读性好。不显示 target（本就是后端日志，crate 名无意义）。
    let stdout_layer = fmt::layer()
        .with_writer(std::io::stdout)
        .with_target(false)
        .with_ansi(true);

    // 文件层：无颜色、含时间、带文件名与行号，便于追踪到具体代码位置。
    // 同样不显示 target（避免冗余的 backend 前缀），靠 file:line 定位来源。
    let file_layer = fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(false)
        .with_file(true)
        .with_line_number(true);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(stdout_layer)
        .with(file_layer)
        .init();
}
