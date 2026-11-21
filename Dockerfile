# syntax=docker/dockerfile:1
#
# Image của Autoxide: build bằng toolchain Rust đã ghim (thư viện ngoài ở lớp riêng nhờ cargo-chef),
# chạy trên Debian slim với một người dùng không phải root. Xem docs/DEPLOY.md.
#
#   docker build -t autoxide .
#   docker run --rm -p 8080:8080 -v autoxide-data:/var/lib/autoxide/data autoxide

# Cùng phiên bản với rust-toolchain.toml — và với thẻ của image `rust` ở `FROM … AS chef` dưới đây.
ARG RUST_VERSION=1.99.1

# --- công cụ ------------------------------------------------------------------
# cargo-chef tách "build thư viện ngoài" khỏi "build code của mình" thành hai lớp Docker riêng, để cache
# (cache gha của CI, hay lớp cục bộ) giữ được lớp thư viện: sửa code hay đổi số phiên bản phát hành không
# làm build lại phần thư viện ngoài (phần lớn thời gian build) (D-20261120-docker-chef).
# Image nền ghim theo digest (thẻ chỉ để người đọc và Dependabot): thẻ bị đẩy lại không đổi được build.
# Dependabot (`.github/dependabot.yml`, docker) đề xuất digest mới mỗi tuần.
FROM rust:1.99.1-slim-trixie@sha256:f6c4a35d21dbb3c89d7a8d056c193e94fc09037f2b58dd858b09c39a47dd02d7 AS chef
# ARG khai trước `FROM` chỉ dùng được trong `FROM`; muốn dùng trong stage phải khai lại.
ARG RUST_VERSION
ENV RUSTUP_TOOLCHAIN=${RUST_VERSION} \
    CARGO_INCREMENTAL=0
RUN cargo install cargo-chef --locked --version 0.1.78
WORKDIR /src

# --- công thức: chỉ danh sách thư viện ngoài -----------------------------------
# `recipe.json` chỉ chứa Cargo.toml / Cargo.lock đã gọt (không có mã nguồn, số phiên bản của crate trong
# workspace bị bỏ), nên nó chỉ đổi khi thư viện ngoài đổi.
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# --- build ---------------------------------------------------------------------
FROM chef AS build
COPY --from=planner /src/recipe.json recipe.json
# Lớp thư viện: chỉ build lại khi recipe.json đổi. Không dùng `--mount=type=cache` cho `target/`: thư mục
# đó nằm ngoài lớp nên cache lớp không giữ được nó (và cache mount không sống qua các máy chạy của GitHub).
RUN cargo chef cook --release --locked -p autoxide --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked -p autoxide \
    && cp target/release/autoxide /usr/local/bin/autoxide

# --- chạy ----------------------------------------------------------------------
FROM debian:trixie-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2
# Nhãn OCI: trang package trên GitHub (ghcr.io) và `docker inspect` đọc mô tả, mã nguồn, tài liệu từ đây.
# Workflow phát hành (docker/metadata-action) có thể ghi đè title / description / source bằng thông tin
# của repo; `documentation` thì chỉ có ở đây.
LABEL org.opencontainers.image.title="Autoxide" \
      org.opencontainers.image.description="Open-source Rust backend core on SQLite or PostgreSQL: REST + OpenAPI, MCP for AI agents, pluggable business modules. Quickstart: compose.quickstart.yml in the repository." \
      org.opencontainers.image.source="https://github.com/thuongtamduy/autoxide" \
      org.opencontainers.image.documentation="https://github.com/thuongtamduy/autoxide#readme" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0"
# Chứng chỉ gốc cho TLS ra ngoài (SMTP, S3, OTLP, PostgreSQL có TLS).
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --user-group --home-dir /var/lib/autoxide autoxide \
    && mkdir -p /var/lib/autoxide/data \
    && chown -R autoxide:autoxide /var/lib/autoxide
COPY --from=build /usr/local/bin/autoxide /usr/local/bin/autoxide
USER autoxide
WORKDIR /var/lib/autoxide
# Mọi dữ liệu ghi xuống đĩa (SQLite, file tải lên, mail ghi ra file) nằm trong `data/`.
VOLUME ["/var/lib/autoxide/data"]
ENV AUTOXIDE_HTTP_ADDRESS=0.0.0.0:8080 \
    AUTOXIDE_LOG_FORMAT=json
EXPOSE 8080
ENTRYPOINT ["autoxide"]
CMD ["serve"]
