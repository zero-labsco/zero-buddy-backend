#!/usr/bin/env node
/**
 * Zero Buddy 后端 API 集成自测脚本（零依赖，Node >= 18 内置 fetch）。
 *
 * 覆盖：/health、/api/chat（成功/FAQ/缓存/LLM/校验错误/畸形 body/CORS/超大 body）、
 *       /api/chat/stream（正常 SSE / 空消息 error 事件）、限流。
 *
 * 用法：
 *   BASE_URL=http://127.0.0.1:3030 node api_integration_test.mjs
 *   # 额外指定限流测试实例（功能测试与限流测试分开，避免功能用例自身触发 10/min 限流）：
 *   BASE_URL=http://127.0.0.1:3030 RATE_LIMIT_URL=http://127.0.0.1:3031 node api_integration_test.mjs
 *
 * 退出码：全部 PASS 返回 0，任一 FAIL 返回 1。
 */

import http from 'node:http';

const BASE = process.env.BASE_URL || 'http://127.0.0.1:3030';
const RATE_LIMIT_URL = process.env.RATE_LIMIT_URL || '';
const REQ_TIMEOUT_MS = 90_000; // LLM/联网兜底可能较慢，放宽

// 用 node http 模块发原始请求（fetch/undici 会拦截手工 Content-Length 声明）。
// 返回 { status, error }；error 为连接层错误码（如 ECONNRESET）。
function rawPost(path, headers, body) {
  const url = new URL(`${BASE}${path}`);
  return new Promise((resolve) => {
    const req = http.request(
      {
        hostname: url.hostname,
        port: url.port,
        path: url.pathname,
        method: 'POST',
        headers,
      },
      (res) => {
        res.resume(); // 丢弃响应体
        res.on('end', () => resolve({ status: res.statusCode }));
      },
    );
    req.on('error', (e) => resolve({ error: e.code ?? e.message }));
    req.end(body);
  });
}

let pass = 0;
let fail = 0;
let skipped = 0;
const failures = [];

function record(name, ok, detail) {
  if (ok) {
    pass += 1;
    console.log(`  \x1b[32mPASS\x1b[0m  ${name}`);
  } else {
    fail += 1;
    failures.push({ name, detail });
    console.log(`  \x1b[31mFAIL\x1b[0m  ${name}\n        ${String(detail)}`);
  }
}

function skip(name, why) {
  skipped += 1;
  console.log(`  \x1b[90mSKIP\x1b[0m  ${name} (${why})`);
}

// 带超时的 fetch 封装
async function req(path, init = {}, timeoutMs = REQ_TIMEOUT_MS) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    return await fetch(`${BASE}${path}`, { ...init, signal: controller.signal });
  } finally {
    clearTimeout(timer);
  }
}

// 等待后端就绪（轮询 /health）
async function waitHealthy(url, maxSecs = 120) {
  const start = Date.now();
  while (Date.now() - start < maxSecs * 1000) {
    try {
      const r = await fetch(`${url}/health`, { signal: AbortSignal.timeout(5000) });
      if (r.status === 200) return true;
    } catch {
      /* not ready yet */
    }
    await new Promise((r) => setTimeout(r, 1500));
  }
  return false;
}

// 解析 SSE：返回 [{type, content, source, url, message}, ...]
async function readSse(res) {
  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buf = '';
  const events = [];
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    buf += decoder.decode(value, { stream: true });
    let idx;
    while ((idx = buf.indexOf('\n')) >= 0) {
      const line = buf.slice(0, idx).trim();
      buf = buf.slice(idx + 1);
      if (line.startsWith('data:')) {
        const payload = line.slice(5).trim();
        if (payload) {
          try {
            events.push(JSON.parse(payload));
          } catch {
            /* skip malformed */
          }
        }
      }
    }
  }
  return events;
}

function chatBody(query, extra = {}) {
  return { messages: [{ role: 'user', content: query }], ...extra };
}

// ============================================================
// 1. 健康检查
// ============================================================
async function testHealth() {
  const r = await req('/health', { method: 'GET' });
  const json = await r.json();
  record(
    'GET /health -> 200 + 空信封',
    r.status === 200 &&
      json.code === 200 &&
      json.message === 'success' &&
      json.body === null,
    `status=${r.status} json=${JSON.stringify(json)}`,
  );
}

// ============================================================
// 2. /api/chat 正常对话（中英文）
// ============================================================
async function testChatValid(lang, query) {
  const r = await req('/api/chat', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(chatBody(query)),
  });
  let json;
  try {
    json = await r.json();
  } catch {
    json = null;
  }
  const okSource = ['faq', 'cache', 'llm', 'offline'].includes(json?.body?.source);
  record(
    `POST /api/chat 有效${lang}问题 -> 200 + 信封 + 非空回复`,
    r.status === 200 &&
      json?.code === 200 &&
      typeof json?.body?.reply === 'string' &&
      json.body.reply.trim().length > 0 &&
      okSource,
    `status=${r.status} source=${json?.body?.source} replyLen=${json?.body?.reply?.length} json=${JSON.stringify(json).slice(0, 200)}`,
  );
}

// ============================================================
// 3. FAQ 命中
// ============================================================
async function testFaq() {
  const query = 'How to install Zero Inspector Kit?';
  const r = await req('/api/chat', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(chatBody(query)),
  });
  const json = await r.json();
  record(
    'FAQ 命中 -> source=faq + url',
    r.status === 200 && json?.body?.source === 'faq' && !!json?.body?.url,
    `status=${r.status} source=${json?.body?.source} url=${json?.body?.url}`,
  );
}

// ============================================================
// 4. 缓存命中（同一 query 两次，第二次应为 cache）
// ============================================================
async function testCache() {
  // 注意：不能用含 "hi"/"architecture"/"which" 等子串的查询，否则会误命中
  // greeting-en 的 "hi" 模式（子串匹配 Bug，见测试汇总）。用不含常见误命中子串的查询。
  const query = 'Describe the answer cache behavior in detail for Zero Buddy deployment';
  const opts = {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(chatBody(query)),
  };
  const first = await req('/api/chat', opts);
  const fJson = await first.json();
  const second = await req('/api/chat', opts);
  const sJson = await second.json();
  record(
    '缓存命中：第二次同 query -> source=cache',
    sJson?.body?.source === 'cache',
    `first=${fJson?.body?.source} second=${sJson?.body?.source}`,
  );
  return fJson?.body?.source; // 供汇总参考
}

// ============================================================
// 5. LLM 路径（全新唯一 query，期望 source=llm）
//    用时间戳做唯一后缀，避免被先前运行的答案缓存污染（否则可能命中 cache）。
//    离线模式（CI 无 LLM_API_KEY）时 source 会是 offline：
//    设置 ALLOW_OFFLINE=1 即接受 offline，便于在 CI 无 key 环境下跑通全链路。
// ============================================================
async function testLlmPath() {
  const marker = Date.now();
  const query = `Explain the Zero Buddy RAG retrieval pipeline in detail (test marker ${marker}).`;
  const r = await req('/api/chat', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(chatBody(query)),
  });
  const json = await r.json();
  const source = json?.body?.source;
  const allowOffline = process.env.ALLOW_OFFLINE === '1';
  const okSource = source === 'llm' || (allowOffline && source === 'offline');
  record(
    'LLM 路径：全新 query -> source=llm' + (allowOffline ? '（允许离线）' : ''),
    r.status === 200 && okSource && typeof json?.body?.reply === 'string',
    `status=${r.status} source=${source} replyLen=${json?.body?.reply?.length} (非 llm 说明在线降级/端点差异)`,
  );
  return source;
}

// ============================================================
// 6. 输入校验错误（400 信封）
// ============================================================
async function testValidationErrors() {
  const cases = [
    { name: '空 messages -> 400', body: { messages: [] }, expectMsg: 'must not be empty' },
    {
      name: '51 条消息 -> 400',
      body: { messages: Array.from({ length: 51 }, () => ({ role: 'user', content: 'hi' })) },
      expectMsg: 'too many messages',
    },
    {
      name: '单条超 4000 字 -> 400',
      body: { messages: [{ role: 'user', content: 'a'.repeat(4001) }] },
      expectMsg: 'message too long',
    },
    {
      name: '末条内容为空 -> 400',
      body: { messages: [{ role: 'user', content: '   ' }] },
      expectMsg: 'last message content is empty',
    },
  ];
  for (const c of cases) {
    const r = await req('/api/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(c.body),
    });
    let json = null;
    try {
      json = await r.json();
    } catch {
      /* non-json */
    }
    record(
      c.name,
      r.status === 400 && json?.body === null && String(json?.message).includes(c.expectMsg),
      `status=${r.status} json=${JSON.stringify(json)}`,
    );
  }
}

// ============================================================
// 7. 畸形 body -> axum Json 提取失败（非信封）
//    注意：JSON 语法错误返回 400；字段缺失（反序列化失败）返回 422。
// ============================================================
async function testMalformedBody() {
  const cases = [
    { name: '非法 JSON -> 400', body: '{bad json', expect: 400 },
    { name: '缺少 messages 字段 -> 422', body: '{"foo":"bar"}', expect: 422 },
  ];
  for (const c of cases) {
    const r = await req('/api/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: c.body,
    });
    record(c.name, r.status === c.expect, `status=${r.status} (期望 ${c.expect})`);
  }
}

// ============================================================
// 8. /api/chat/stream 正常 SSE
// ============================================================
async function testStreamNormal() {
  const r = await req('/api/chat/stream', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(chatBody('hello')),
  });
  if (r.status !== 200) {
    record('stream 正常 -> 收到 delta + done', false, `status=${r.status}`);
    return;
  }
  const events = await readSse(r);
  const deltas = events.filter((e) => e.type === 'delta').map((e) => e.content || '').join('');
  const done = events.find((e) => e.type === 'done');
  record(
    'stream 正常 -> 至少一个 delta + done 事件',
    deltas.length > 0 && !!done && !!done.source,
    `deltas=${events.filter((e) => e.type === 'delta').length} done=${JSON.stringify(done)} joinedLen=${deltas.length}`,
  );
}

// ============================================================
// 9. /api/chat/stream 空消息 -> error 事件
// ============================================================
async function testStreamError() {
  const r = await req('/api/chat/stream', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ messages: [] }),
  });
  if (r.status !== 200) {
    record('stream 空消息 -> error 事件', false, `status=${r.status}`);
    return;
  }
  const events = await readSse(r);
  const err = events.find((e) => e.type === 'error');
  record(
    'stream 空消息 -> error 事件',
    !!err && typeof err.message === 'string' && err.message.length > 0,
    `events=${JSON.stringify(events)}`,
  );
}

// ============================================================
// 10. CORS
// ============================================================
async function testCors() {
  // 允许的来源
  const ok = await req('/health', {
    method: 'GET',
    headers: { Origin: 'http://localhost:3040' },
  });
  const allow = ok.headers.get('access-control-allow-origin');
  record(
    'CORS 允许来源 -> ACAO=http://localhost:3040',
    allow === 'http://localhost:3040',
    `ACAO=${allow}`,
  );
  // 不允许的来源
  const bad = await req('/health', {
    method: 'GET',
    headers: { Origin: 'http://evil.example.com' },
  });
  const badAllow = bad.headers.get('access-control-allow-origin');
  record(
    'CORS 拒绝任意来源（evil）',
    !badAllow || badAllow === 'null',
    `ACAO=${badAllow}`,
  );
}

// ============================================================
// 11. 超大 body -> 413
//    (a) 声明超大 Content-Length 头 -> 服务端读 body 前即返回 413（稳定）。
//    (b) 实际传输 >1MB body -> 服务端在收体中段断开连接，客户端可能收到
//        ECONNRESET 而非 413（curl 正常收到 413，Node 客户端差异）。
// ============================================================
async function testLargeBody() {
  const big = 'a'.repeat(1_100_000);
  const fullBody = JSON.stringify(chatBody(big));

  // (a) 声明超大 Content-Length 但只发小 body：RequestBodyLimitLayer 依据
  //     Content-Length 头在读 body 前直接 413（与 curl 行为一致）。
  const a = await rawPost('/api/chat', {
    'Content-Type': 'application/json',
    'Content-Length': String(1_100_000),
  }, JSON.stringify(chatBody('hi')));
  record(
    '超大 Content-Length 头 -> 413',
    a.status === 413,
    `status=${a.status} error=${a.error ?? '-'}`,
  );

  const b = await rawPost('/api/chat', {
    'Content-Type': 'application/json',
    'Content-Length': String(Buffer.byteLength(fullBody)),
  }, fullBody);
  // 服务端在收到中段超大 body 时会提前关闭连接，客户端可能收到 ECONNRESET
  // (curl) 或 EPIPE (Node 写完对端已关闭的 socket)；两者都表示“服务端拒绝超 body”，
  // 与 413 等价，均视为通过。
  const refused = b.error === 'ECONNRESET' || b.error === 'EPIPE';
  record(
    '实际传 >1MB body：得到 413 或连接中断(服务端拒绝)',
    b.status === 413 || refused,
    `status=${b.status} error=${b.error ?? '-'}`,
  );
}

// ============================================================
// 12. 限流（对 RATE_LIMIT_URL 实例，需 RATE_LIMIT_PER_MIN=10）
// ============================================================
async function testRateLimit() {
  if (!RATE_LIMIT_URL) {
    skip('限流（需 RATE_LIMIT_URL 指向 limit=10 的实例）', '未提供 RATE_LIMIT_URL');
    return;
  }
  const ok = await waitHealthy(RATE_LIMIT_URL, 60);
  if (!ok) {
    record('限流实例就绪', false, `无法连接 ${RATE_LIMIT_URL}/health`);
    return;
  }
  const body = JSON.stringify({ messages: [{ role: 'user', content: 'hello' }] });
  const results = [];
  for (let i = 0; i < 11; i += 1) {
    const r = await fetch(`${RATE_LIMIT_URL}/api/chat`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body,
      signal: AbortSignal.timeout(30_000),
    });
    let json = null;
    try {
      json = await r.json();
    } catch {
      /* ignore */
    }
    results.push({ i: i + 1, status: r.status, source: json?.body?.source });
  }
  const blocked = results.filter((x) => x.source === 'rate-limit');
  const lastBlocked = results[results.length - 1]?.source === 'rate-limit';
  record(
    '限流：11 个快速请求，第 11 个 source=rate-limit',
    lastBlocked,
    `results=${JSON.stringify(results)}`,
  );
}

// ============================================================
// 主流程
// ============================================================
async function main() {
  console.log(`\nZero Buddy 后端 API 集成自测\n目标: ${BASE}${RATE_LIMIT_URL ? `  限流实例: ${RATE_LIMIT_URL}` : ''}\n`);
  if (!(await waitHealthy(BASE))) {
    console.log(`\x1b[31m后端未就绪: ${BASE}/health 在 120s 内未返回 200，请先启动后端。\x1b[0m`);
    process.exit(1);
  }

  console.log('\n[1] 健康检查');
  await testHealth();

  console.log('\n[2] 正常对话');
  await testChatValid('英文', 'What is Zero Buddy?');
  await testChatValid('中文', 'Zero Buddy 是什么？');

  console.log('\n[3] FAQ');
  await testFaq();

  console.log('\n[4] 缓存');
  await testCache();

  console.log('\n[5] LLM 路径');
  await testLlmPath();

  console.log('\n[6] 输入校验');
  await testValidationErrors();

  console.log('\n[7] 畸形 body');
  await testMalformedBody();

  console.log('\n[8] 流式 SSE');
  await testStreamNormal();
  await testStreamError();

  console.log('\n[9] CORS');
  await testCors();

  console.log('\n[10] 超大 body');
  await testLargeBody();

  console.log('\n[11] 限流');
  await testRateLimit();

  console.log(`\n========== 汇总 ==========`);
  console.log(`  PASS: ${pass}  FAIL: ${fail}  SKIP: ${skipped}`);
  if (failures.length) {
    console.log('\n失败用例：');
    failures.forEach((f) => console.log(`  - ${f.name}: ${f.detail}`));
  }
  console.log('');
  process.exit(fail > 0 ? 1 : 0);
}

main().catch((e) => {
  console.error('脚本异常:', e);
  process.exit(2);
});
