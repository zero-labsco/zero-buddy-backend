# Contributing Guide / 贡献指南

Welcome to the **zero buddy backend** (Rust · Axum). This document explains how to
contribute. It is written in **English (EN)** and **简体中文 (ZH)** — the two languages
are presented side by side under each heading.

欢迎参与 **zero buddy 后端**（Rust · Axum）的开发。本文档说明如何参与贡献。内容以
**英文（EN）** 与 **简体中文（ZH）** 双语呈现，每个小节下方并列两种语言。

---

## 1. Code of Conduct / 行为准则

**EN** — Be respectful and constructive. By participating you agree to uphold a
welcoming, harassment-free environment for everyone.

**ZH** — 请保持尊重与建设性。参与本项目即表示你同意维护一个对所有人友好、无骚扰的环境。

---

## 2. Getting Started / 开始之前

**EN**

1. Fork the repository and clone your fork.
2. Install the Rust toolchain (stable).
3. Copy the environment template and fill in your values:
   ```bash
   cp .env.example .env
   ```
4. Build and run tests:
   ```bash
   cargo build
   cargo test
   cargo run
   ```

**ZH**

1. Fork 本仓库并克隆你的副本。
2. 安装 Rust 工具链（stable 版本）。
3. 复制环境变量模板并填写你的配置：
   ```bash
   cp .env.example .env
   ```
4. 构建并运行测试：
   ```bash
   cargo build
   cargo test
   cargo run
   ```

---

## 3. Branching Model / 分支模型

**EN** — Create a feature branch from `main` using the `feature/` prefix:

```bash
git checkout -b feature/your-feature-name
```

Use prefixes consistently: `feature/`, `fix/`, `chore/`, `docs/`, `refactor/`.

**ZH** — 从 `main` 切出以 `feature/` 为前缀的功能分支：

```bash
git checkout -b feature/your-feature-name
```

请统一使用前缀：`feature/`、`fix/`、`chore/`、`docs/`、`refactor/`。

---

## 4. Commit Message Convention / 提交信息规范

**EN** — We follow [Conventional Commits](https://www.conventionalcommits.org/).
The CI rejects PRs whose commits do not match the pattern.

Format:

```
<type>(<optional scope>): <description>

[optional body]
[optional footer]
```

Types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`,
`ci`, `chore`, `revert`.

Examples:

```
feat(chat): add rate limiting by client IP
fix(scope): correct regex allowlist matching
chore(deps): bump tokio to 1.40
```

**ZH** — 我们遵循 [Conventional Commits](https://www.conventionalcommits.org/) 规范，
CI 会拒绝不符合规范的 PR 提交。

格式：

```
<type>(<可选 scope>): <描述>

[可选正文]
[可选脚注]
```

类型：`feat`、`fix`、`docs`、`style`、`refactor`、`perf`、`test`、`build`、
`ci`、`chore`、`revert`。

示例：

```
feat(chat): 新增基于客户端 IP 的速率限制
fix(scope): 修正正则白名单匹配
chore(deps): 将 tokio 升级到 1.40
```

---

## 5. Before You Push / 推送前检查

**EN** — Run the same checks the CI runs locally:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Optionally install the pre-commit hook so formatting is enforced automatically:

```bash
git config core.hooksPath .githooks
```

**ZH** — 推送前请在本地运行与 CI 相同的检查：

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

可选：安装 pre-commit 钩子，自动强制格式化：

```bash
git config core.hooksPath .githooks
```

---

## 6. Security & Dependency Audits / 安全与依赖审计

**EN** — A scheduled workflow runs `cargo audit` weekly and opens a PR for fixes.
Do not ship code with known high/critical vulnerabilities. Run locally with:

```bash
cargo audit
```

**ZH** — 定时工作流每周运行 `cargo audit` 并自动开 PR 修复。请勿提交存在已知
高危/严重漏洞的代码。本地可运行：

```bash
cargo audit
```

---

## 7. Opening a Pull Request / 发起 Pull Request

**EN**

1. Push your branch to your fork.
2. Open a PR against `main`.
3. Fill in the PR template and link any related issue.
4. Ensure required checks (fmt, clippy, test, commit-lint) pass.
5. A maintainer will review; please respond to review comments.

**ZH**

1. 将分支推送到你的 fork。
2. 向 `main` 发起 Pull Request。
3. 填写 PR 模板并关联相关 issue。
4. 确保必需的检查（fmt、clippy、test、commit-lint）通过。
5. 维护者会进行评审，请回复评审意见。

---

## 8. License / 许可证

**EN** — Contributions are licensed under [Apache-2.0](./LICENSE).

**ZH** — 本项目的贡献以 [Apache-2.0](./LICENSE) 协议授权。

---

Thank you for contributing! / 感谢你的贡献！
