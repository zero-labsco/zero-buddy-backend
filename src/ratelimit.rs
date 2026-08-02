//! 基于客户端 IP 的轻量速率限制（防滥用 / 防刷 / 控成本）。
//!
//! 采用进程内内存计数（HashMap + Mutex），无需外部存储，适合单实例部署。
//! 多实例部署时若需要全局一致限制，应改用 Redis 等共享存储。
//!
//! 限制维度：
//!
//! - 每分钟请求数（`per_min`）
//! - 每天请求数（`per_day`）
//!
//! 任一维度超过上限即拒绝（返回 false）。`0` 表示该维度不限制。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

struct Entry {
    // 60 秒滑动窗口内的请求时间戳（毫秒）
    minute: Vec<u64>,
    // 当天日期字符串（YYYY-MM-DD）与当日累计计数
    day_key: String,
    day_count: u32,
}

#[derive(Clone)]
pub struct RateLimiter {
    state: std::sync::Arc<Mutex<HashMap<String, Entry>>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            state: std::sync::Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 检查并登记一次客户端请求。
    /// 返回 true 表示允许通过；false 表示被限流。
    pub fn check_and_record(&self, ip: &str, per_min: u32, per_day: u32) -> bool {
        // 任一维度为 0 视为不限制该维度；两个都为 0 直接放行
        if per_min == 0 && per_day == 0 {
            return true;
        }

        let now_ms = now_ms();
        let today = today_key();

        let mut map = self.state.lock().unwrap();
        let entry = map.entry(ip.to_string()).or_insert(Entry {
            minute: Vec::new(),
            day_key: today.clone(),
            day_count: 0,
        });

        // 跨天则重置每日计数
        if entry.day_key != today {
            entry.day_key = today.clone();
            entry.day_count = 0;
        }

        // 清理 60 秒窗口外的记录
        let cutoff = now_ms.saturating_sub(60_000);
        entry.minute.retain(|&t| t >= cutoff);

        // 维度检查
        if per_min != 0 && entry.minute.len() as u32 >= per_min {
            return false;
        }
        if per_day != 0 && entry.day_count >= per_day {
            return false;
        }

        // 登记
        entry.minute.push(now_ms);
        entry.day_count += 1;
        true
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn today_key() -> String {
    // 以 UTC 日期作为每日计数的分界
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    format!("{}", days) // 用连续天数做 key，跨天自动不同
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}
