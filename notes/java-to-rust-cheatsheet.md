# Java → Rust（Axum）概念速查表

给从 Java/Spring 转过来的同学，对照本项目（Zero Buddy 后端）讲清楚两边概念怎么对应。
> 本文件已随代码同步更新，覆盖当前实现的模块：`config / llm / knowledge / rag / cache / faq / chat`。

## 1. 整体心智模型

| Java / Spring Boot | Rust / Axum | 说明 |
|---|---|---|
| `@SpringBootApplication` + `main()` | `#[tokio::main] async fn main()` | 程序入口 |
| 内嵌 Tomcat | `tokio::net::TcpListener` + `axum::serve` | HTTP 服务器 |
| `@RestController` 类 | 普通 `async fn` + `Router::route` 注册 | 没有注解，路由手动登记 |
| `@Autowired` / IoC 容器 | `State<AppState>` 共享状态（`.with_state`） | 没有 IoC 容器，状态自己塞进 Router |
| `@Service` 类 | 普通 `struct` + `impl` 方法（如 `LlmClient`） | 业务逻辑分层靠 `mod` + `struct` |
| `@Repository` 类 | 读写文件的模块（`knowledge.rs` / `faq.rs`） | 数据访问即函数 |
| `application.yml` / `@Value` | `Config::from_env()` 读环境变量 | 配置从 `.env` / 环境变量来，全部集中在 `config.rs` |
| `@ConfigurationProperties` | `Config` 结构体（自带默认值 + 可选覆盖） | 改名/调参只改 `.env`，不动代码 |
| SLF4J / Logback | `tracing` + `tracing-subscriber` + `tracing-appender` | 结构化日志，用 `RUST_LOG=debug` 调级别；`tracing-appender` 的 `rolling::daily` 对应 Logback 的 `RollingFileAppender`，日志落到 `logs/app.YYYY-MM-DD.log` |

## 2. Controller 层（最关心的一层）

### Java 写法（你熟悉的）
```java
@RestController
@RequestMapping("/api")
public class ChatController {
    @Autowired private ChatService chat;

    @PostMapping("/chat")
    public ChatResponse chat(@RequestBody ChatRequest req) {
        return chat.handle(req);
    }

    @GetMapping("/health")
    public HealthResponse health() { return ...; }
}
```

### Rust 等价写法（本项目 `src/main.rs`）
```rust
// 1) 路由注册（替代 @RequestMapping / @PostMapping / @GetMapping）
//    注意：本项目在 Router 上叠加了 CORS、请求体大小限制、全局超时三层中间件
let app = Router::new()
    .route("/api/chat", post(chat_handler))
    .layer(cors)                       // CORS 收敛到前端来源
    .layer(RequestBodyLimitLayer::new(1_000_000)) // 限制 body 1MB
    .layer(TimeoutLayer::new(Duration::from_secs(cfg.request_timeout_secs() + 5)))
    .with_state(state);

// 2) handler 函数（替代 Controller 里的每个方法）
async fn chat_handler(
    State(state): State<AppState>,     // ≈ @Autowired AppState
    Json(req): Json<ChatRequest>,      // ≈ @RequestBody
) -> Result<Json<Value>, (StatusCode, String)> {
    handle_chat(&state.cfg, &state.client, &state.cache, req)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}
```

> 要点：**Rust 没有注解扫描**。你新加一个接口，必须两步都做——写 `fn` + 在 `Router::new()` 里 `.route(...)` 登记，否则请求进不来。

## 3. 分层对照（本项目文件职责）

| 层 | Java | 本项目 Rust 文件 | 作用 |
|---|---|---|---|
| Controller | `ChatController` | `main.rs` 的 `chat_handler` | 接收 HTTP 请求、错误映射为状态码 |
| Service | `ChatService` | `chat.rs` 的 `handle_chat` | 编排业务：FAQ→缓存→RAG→LLM |
| Repository/Data | `FaqRepository` | `faq.rs`（`FaqStore` 读 `data/faq.json`） | 本地问答规则（零 token） |
| | `RagRepository` | `rag.rs`（`retrieve_scored`） | 向量检索 + 关键词兜底 |
| 缓存层 | `@Cacheable` | `cache.rs`（`AnswerCache`） | 两层答案缓存：精确命中 + 语义近邻 |
| Model / DTO | `ChatRequest` / `ChatResponse` | `models.rs` 的结构体 | 请求/响应数据结构 |
| Config | `@ConfigurationProperties` | `config.rs` 的 `Config` | 运行时配置（含品牌名、阈值） |
| Client | `RestTemplate` / `WebClient` | `llm.rs`（`LlmClient`，复用 `reqwest::Client`） | 调外部 LLM / 向量化 API |
| 启动初始化 | `@PostConstruct` | `main()` 里加载 knowledge / faq / cache | 启动期一次完成 |

## 4. 常用语法对照

| 概念 | Java | Rust |
|---|---|---|
| 类 | `class Foo { }` | `struct Foo { }` + `impl Foo { }` |
| 接口 | `interface Foo` | `trait Foo`（本项目暂未大量用） |
| 依赖注入字段 | `@Autowired Foo foo;` | 字段写在 `struct` 里，构造时传入 |
| `null` | `Foo foo = null;` | `Option<Foo>`（`Some` / `None`，编译器强制判空） |
| 异常 | `throw new RuntimeException()` | `Result<T, E>`（用 `?` 向上传，或 `anyhow::Result`） |
| try/catch | `try { } catch (e) { }` | `match result { Ok(v) => .., Err(e) => .. }` 或 `?` |
| List | `List<String>` | `Vec<String>` |
| Map | `Map<String,String>` | `HashMap<String, String>` |
| JSON 序列化 | Jackson `@RequestBody` | `serde` 派生 `#[derive(Deserialize/Serialize)]` |
| 异步 | `@Async` / `CompletableFuture` | `async fn` + `.await` |
| 单例 Bean | `@Service @Bean` | `AppState` 经 `.with_state` 跨请求共享 |
| 连接池复用 | `RestTemplate` 单例 | `reqwest::Client` 包进 `Arc`，全进程复用 |

## 5. 编译/运行对照

| 操作 | Java (Maven) | Rust (Cargo) |
|---|---|---|
| 拉依赖 | `mvn install` | `cargo build`（自动拉 `Cargo.toml`） |
| 启动 | `mvn spring-boot:run` | `cargo run` |
| 只检查编译 | `mvn compile` | `cargo check`（快，不改产物） |
| 打包 | `mvn package` → jar | `cargo build --release` → 二进制 |
| 依赖声明 | `pom.xml` | `Cargo.toml` |
| 依赖锁文件 | 无（靠版本号） | `Cargo.lock`（锁死精确版本，本项目已提交） |

## 6. 新人最快上手路径

1. 想看「接口在哪」→ 打开 `src/main.rs`，看 `.route(...)`
2. 想看「某个接口做什么」→ 找对应的 `xxx_handler` 函数
3. 想看「业务怎么编排」→ `src/chat.rs` 的 `handle_chat`
4. 想加本地问答（不改代码）→ 编辑 `data/faq.json`
5. 想省 token（重复问题）→ 看 `src/cache.rs`，改 `CACHE_SIMILARITY` 调近邻阈值
6. 想加新接口 → `main.rs` 写 `fn` + 加 `.route(...)` 两步走
7. 想改名/调参 → 改 `.env`（`PRODUCT_NAME`、`RAG_MIN_SCORE` 等），无需动代码

> 约定：所有中文注释已补齐，改逻辑时优先读注释对应的区块；日志统一用 `tracing::info!/warn!`，不要用 `println!`。
