# syntax=docker/dockerfile:1
#
# Image của Autoxide: build bằng toolchain Rust đã ghim, chạy trên Debian slim với một
# người dùng không phải root. Xem docs/DEPLOY.md.
#
#   docker build -t autoxide .
#   docker run --rm -p 8080:8080 -v autoxide-data:/var/lib/autoxide/data autoxide

# Cùng phiên bản với rust-toolchain.toml.
ARG RUST_VERSION=1.99.1

# --- build ---------------------------------------------------------------------
FROM rust:${RUST_VERSION}-slim-trixie AS build
# Dùng đúng toolchain của image; khỏi tải thêm thành phần chỉ dành cho máy dev
# (rust-analyzer…) mà rust-toolchain.toml liệt kê.
ENV RUSTUP_TOOLCHAIN=${RUST_VERSION} \
    CARGO_INCREMENTAL=0
WORKDIR /src
COPY . .
# Cache thư viện tải về và thư mục build giữa các lần build trên cùng máy.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p autoxide \
    && cp target/release/autoxide /usr/local/bin/autoxide

# --- chạy ----------------------------------------------------------------------
FROM debian:trixie-slim
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
