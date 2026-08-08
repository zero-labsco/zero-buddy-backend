//! 运行时联网兜底：当本地知识库（RAG）检索不到相关内容时，
//! 主动去 Zero Labs 官网、GitHub 仓库以及 Invoice Zero 产品官网抓取页面文本，作为 LLM 的上下文。
//!
//! 抓不到（网络不通 / 超时 / 解析失败）时优雅降级为「无上下文纯对话」，不影响主流程。
//! 抓取结果带进程内 TTL 缓存，避免每个请求都去联网（见 `WEB_FALLBACK_TTL_SECS`）。

use anyhow::Result;
use reqwest::Client;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

// 抓取目标：Zero Labs 官网 + GitHub 组织页 + Invoice Zero 产品官网
// （GitHub 会自动渲染 README 为 HTML；各产品官网提供产品级详情）。
const TARGETS: &[&str] = &[
    "https://zerolabsco.com",
    "https://github.com/zero-labsco",
    "https://invoicezero.net",
];

// 单次抓取超时（秒）：防止外部站点卡死拖垮请求。
const FETCH_TIMEOUT_SECS: u64 = 8;
// 每个页面最多保留的字符数，避免上下文过长挤占 LLM 窗口。
const MAX_CHARS_PER_PAGE: usize = 4000;
// 兜底上下文总上限。
const MAX_TOTAL_CHARS: usize = 8000;

/// 进程内缓存：TTL 过期后重新抓取。
struct CacheEntry {
    text: String,
    fetched_at: Instant,
}

static CACHE: OnceLock<Mutex<Option<CacheEntry>>> = OnceLock::new();

/// 读取 TTL（秒）。0 表示不缓存（每次都重新抓取）；默认 3600 秒（1 小时）。
fn ttl() -> Duration {
    match std::env::var("WEB_FALLBACK_TTL_SECS") {
        Ok(v) => match v.parse::<u64>() {
            Ok(secs) => Duration::from_secs(secs),
            Err(_) => Duration::from_secs(3600),
        },
        Err(_) => Duration::from_secs(3600),
    }
}

/// 抓取并按需清洗所有目标页面，拼成一段纯文本上下文。
/// 返回空字符串表示全部抓取失败（调用方应降级为纯对话）。
/// 命中有效缓存时直接返回缓存内容，不联网。
pub async fn fetch_zero_labs_context() -> String {
    // 通过环境变量可关闭联网兜底（默认开启）
    if std::env::var("WEB_FALLBACK_ENABLED")
        .map(|v| v == "0" || v.eq_ignore_ascii_case("false"))
        .unwrap_or(false)
    {
        tracing::info!("[web fallback] disabled by env WEB_FALLBACK_ENABLED=0");
        return String::new();
    }

    let ttl = ttl();
    let cache = CACHE.get_or_init(|| Mutex::new(None));

    // 1) 命中缓存且未过期 → 直接返回
    if ttl.as_secs() > 0 {
        if let Ok(guard) = cache.lock() {
            if let Some(entry) = guard.as_ref() {
                if entry.fetched_at.elapsed() < ttl {
                    tracing::info!(
                        "[web fallback] cache hit (age {}s < ttl {}s, {} chars) — skip network",
                        entry.fetched_at.elapsed().as_secs(),
                        ttl.as_secs(),
                        entry.text.len()
                    );
                    return entry.text.clone();
                }
            }
        }
    }

    // 2) 缓存未命中或已过期 → 重新抓取
    let fresh = fetch_fresh().await;
    if let Ok(mut guard) = cache.lock() {
        *guard = Some(CacheEntry {
            text: fresh.clone(),
            fetched_at: Instant::now(),
        });
    }
    fresh
}

/// 真正执行联网抓取（无缓存逻辑）。
async fn fetch_fresh() -> String {
    let client = Client::builder()
        .timeout(Duration::from_secs(FETCH_TIMEOUT_SECS))
        .user_agent("ZeroBuddy/1.0 (+https://zerolabsco.com)")
        .build();

    let client = match client {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("[web fallback] http client build failed: {:#}", e);
            return String::new();
        }
    };

    let mut out = String::new();
    for url in TARGETS {
        match fetch_one(&client, url).await {
            Ok(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let chunk = if trimmed.len() > MAX_CHARS_PER_PAGE {
                    &trimmed[..MAX_CHARS_PER_PAGE]
                } else {
                    trimmed
                };
                out.push_str(&format!("\n---\nSource: {}\n{}\n", url, chunk));
                if out.len() >= MAX_TOTAL_CHARS {
                    break;
                }
            }
            Err(e) => {
                tracing::warn!("[web fallback] fetch {} failed: {:#}", url, e);
            }
        }
    }

    out.truncate(MAX_TOTAL_CHARS);
    if out.is_empty() {
        tracing::warn!("[web fallback] all targets failed; falling back to pure chat");
    } else {
        tracing::info!(
            "[web fallback] fetched {} chars of context from zerolabsco.com / github",
            out.len()
        );
    }
    out
}

/// 抓取单个页面并清洗为纯文本：
/// - 移除 `<script>`/`<style>` 块
/// - 剥离所有 HTML 标签
/// - 将常见空白折叠为单个空格
async fn fetch_one(client: &Client, url: &str) -> Result<String> {
    let resp = client.get(url).send().await?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("HTTP {}", status);
    }
    let body = resp.text().await?;
    Ok(strip_html(&body))
}

/// 极简 HTML 清洗（无外部依赖）：
/// 足够应对官网 / GitHub README 这类以正文文本为主的页面。
fn strip_html(html: &str) -> String {
    let mut s = html.to_string();

    // 1) 删除 <script>...</script> 与 <style>...</style>（含多行）
    s = remove_blocks(&s, "<script", "</script>");
    s = remove_blocks(&s, "<style", "</style>");
    s = remove_blocks(&s, "<head", "</head>");
    s = remove_blocks(&s, "<svg", "</svg>");
    s = remove_blocks(&s, "<!--", "-->");

    // 2) 把换行/块级标签转成空格，避免连字
    s = s
        .replace("<br", " ")
        .replace("<p", " ")
        .replace("<div", " ")
        .replace("<li", " ")
        .replace("<tr", " ")
        .replace("<td", " ")
        .replace("<th", " ")
        .replace("<h1", " ")
        .replace("<h2", " ")
        .replace("<h3", " ")
        .replace("<h4", " ")
        .replace("<h5", " ")
        .replace("<h6", " ");

    // 3) 剥离剩余所有标签 <...>
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }

    // 4) 解码最常见的 HTML 实体
    out = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&apos;", "'");

    // 5) 折叠空白
    let mut collapsed = String::with_capacity(out.len());
    let mut prev_ws = false;
    for c in out.chars() {
        if c.is_whitespace() {
            if !prev_ws {
                collapsed.push(' ');
            }
            prev_ws = true;
        } else {
            collapsed.push(c);
            prev_ws = false;
        }
    }
    collapsed.trim().to_string()
}

/// 删除从 `open`（含）到 `close`（含）之间的所有内容（大小写不敏感、跨多行）。
fn remove_blocks(s: &str, open: &str, close: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = find_case_insensitive(rest, open) {
        result.push_str(&rest[..start]);
        if let Some(rel_close) = find_case_insensitive(&rest[start..], close) {
            let end = start + rel_close + close.len();
            rest = &rest[end..];
        } else {
            // 没有闭合标签，丢弃剩余全部
            rest = "";
            break;
        }
    }
    result.push_str(rest);
    result
}

fn find_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    haystack.to_lowercase().find(&needle.to_lowercase())
}
