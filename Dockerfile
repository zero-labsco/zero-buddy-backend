# ---- 构建阶段 ----
FROM rust:1.82-slim AS builder

# 安装编译所需的基础工具（curl、ssl、pkg-config 等）
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev ca-certificates && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

# 先拷贝依赖清单，利用 Docker 层缓存加速构建
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && \
    cargo build --release && rm -rf src

# 再拷贝真实源码并构建二进制
COPY src ./src
COPY data ./data
RUN touch data/answers_cache.json && \
    cargo build --release && \
    cp target/release/zero-buddy-backend /app/zero-buddy-backend

# ---- 运行阶段 ----
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 && \
    rm -rf /var/lib/apt/lists/* && \
    groupadd -r app && useradd -r -g app app

WORKDIR /app

COPY --from=builder /app/zero-buddy-backend /app/zero-buddy-backend
COPY --from=builder /app/data /app/data
COPY banner.txt /app/banner.txt

# 运行时以非 root 用户启动
RUN chown -R app:app /app
USER app

# 通过 BIND_ADDR 控制监听地址与端口（1Panel 部署务必设为 0.0.0.0:<端口>）
ENV BIND_ADDR=0.0.0.0:3030
EXPOSE 3030

CMD ["/app/zero-buddy-backend"]
