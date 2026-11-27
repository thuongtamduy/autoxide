# syntax=docker/dockerfile:1
#
# Image của Autoxide: build bằng toolchain Rust đã ghim (thư viện ngoài ở lớp riêng nhờ cargo-chef),
# chạy trên image distroless (không shell) với một người dùng không phải root. Xem docs/DEPLOY.md.
#
#   docker build -t autoxide .
#   docker run --rm -p 8080:8080 -v autoxide-data:/var/lib/autoxide/data autoxide

# Cùng phiên bản với rust-toolchain.toml — và với thẻ của image `rust` ở `FROM … AS chef` dưới đây.
ARG RUST_VERSION=1.100.0

# --- công cụ ------------------------------------------------------------------
# cargo-chef tách "build thư viện ngoài" khỏi "build code của mình" thành hai lớp Docker riêng, để cache
# (cache gha của CI, hay lớp cục bộ) giữ được lớp thư viện: sửa code hay đổi số phiên bản phát hành không
# làm build lại phần thư viện ngoài (phần lớn thời gian build) (D-20261120-docker-chef).
# Image nền ghim theo digest (thẻ chỉ để người đọc và Dependabot): thẻ bị đẩy lại không đổi được build.
# Dependabot (`.github/dependabot.yml`, docker) đề xuất digest mới mỗi tuần.
FROM rust:1.100.0-slim-trixie@sha256:6f14b2b4b278ec0d50aa1d69541c304ce0f95e0f6268487aa9924af1c2e9aebe AS chef
# ARG khai trước `FROM` chỉ dùng được trong `FROM`; muốn dùng trong stage phải khai lại.
ARG RUST_VERSION
# Build tái lập (docs/COMPATIBILITY.md, "Reproducible builds"): luôn build trong /src và xoá đường dẫn build, đường dẫn
# registry khỏi binary (`--remap-path-prefix`), nên binary không phụ thuộc máy build. `SOURCE_DATE_EPOCH` (thời điểm
# commit của bản phát hành, do workflow phát hành truyền vào) là mốc thời gian duy nhất mà build được dùng; khai ở stage
# `build` dưới đây, sau lớp thư viện, để mỗi bản phát hành không làm mất cache lớp đó.
ENV RUSTUP_TOOLCHAIN=${RUST_VERSION} \
    CARGO_INCREMENTAL=0 \
    RUSTFLAGS="--remap-path-prefix=/src=/build --remap-path-prefix=/usr/local/cargo/registry/src=/cargo-registry"
# cargo-auditable nhúng danh sách crate Rust đúng như lúc build vào binary (đoạn `.dep-v0`), để máy quét lỗ hổng
# (cargo audit bin, Syft, Trivy) thấy crate trong binary đã strip (SBOM, docs/COMPATIBILITY.md).
RUN cargo install cargo-chef --locked --version 0.1.78 \
    && cargo install cargo-auditable --locked --version 0.7.7 \
    && cargo install cargo-about --locked --version 0.9.3 --features cli
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
RUN cargo auditable chef cook --release --locked -p autoxide --recipe-path recipe.json
COPY . .
ARG SOURCE_DATE_EPOCH=0
ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}
RUN cargo auditable build --release --locked -p autoxide \
    && cp target/release/autoxide /usr/local/bin/autoxide \
    && mkdir -p /rootfs/var/lib/autoxide/data
# Văn bản giấy phép của mọi crate trong binary (D-20261126-third-party-licenses): nghĩa vụ giữ thông báo bản quyền khi
# phân phối binary. Đi cùng image, và workflow phát hành lấy từ đây đưa vào tarball và trang Release.
RUN cargo about generate --manifest-path crates/app/Cargo.toml --locked about.hbs \
    -o /usr/local/share/THIRD-PARTY-LICENSES.html

# --- chạy ----------------------------------------------------------------------
# Image distroless (G7): chỉ glibc, libgcc, chứng chỉ gốc và múi giờ — không shell, không trình quản lý gói, không curl — nên bề mặt
# tấn công và số thành phần cần vá nhỏ hơn nhiều so với một bản Debian đầy đủ. Binary build trên Debian 13 và dùng glibc nên chọn
# biến thể `cc` của Debian 13 (cùng phiên bản glibc), không phải `static` (dành cho binary tĩnh). Kiểm sức khoẻ bằng chính binary
# (`autoxide healthcheck`) vì không có shell hay curl. Ghim theo digest như mọi image nền; Dependabot đề xuất digest mới mỗi tuần.
FROM gcr.io/distroless/cc-debian13:nonroot@sha256:29ee70762214fe8190cea04ef1c0a84a8365879e047ce8f911ae5a5ab2355fa2
# Nhãn OCI: trang package trên GitHub (ghcr.io) và `docker inspect` đọc mô tả, mã nguồn, tài liệu từ đây.
# Workflow phát hành (docker/metadata-action) có thể ghi đè title / description / source bằng thông tin
# của repo; `documentation` thì chỉ có ở đây.
LABEL org.opencontainers.image.title="Autoxide" \
      org.opencontainers.image.description="Open-source Rust backend core on SQLite or PostgreSQL: REST + OpenAPI, MCP for AI agents, pluggable business modules. Quickstart: compose.quickstart.yml in the repository." \
      org.opencontainers.image.source="https://github.com/thuongtamduy/autoxide" \
      org.opencontainers.image.documentation="https://github.com/thuongtamduy/autoxide#readme" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0"
COPY --from=build /usr/local/bin/autoxide /usr/local/bin/autoxide
COPY --from=build /usr/local/share/THIRD-PARTY-LICENSES.html /usr/share/doc/autoxide/THIRD-PARTY-LICENSES.html
COPY LICENSE-MIT LICENSE-APACHE /usr/share/doc/autoxide/
# Thư mục dữ liệu thuộc user 10001 (cùng uid với các bản trước, để volume cũ vẫn ghi được): distroless không có `useradd`, nên
# dựng sẵn ở stage build rồi chép sang với chủ sở hữu đúng. UID số là đủ, không cần dòng trong /etc/passwd.
COPY --from=build --chown=10001:10001 /rootfs/var/lib/autoxide /var/lib/autoxide
USER 10001:10001
WORKDIR /var/lib/autoxide
# Mọi dữ liệu ghi xuống đĩa (SQLite, file tải lên, mail ghi ra file) nằm trong `data/`.
VOLUME ["/var/lib/autoxide/data"]
ENV AUTOXIDE_HTTP_ADDRESS=0.0.0.0:8080 \
    AUTOXIDE_LOG_FORMAT=json
EXPOSE 8080
# `/health/ready`: máy chủ nhận được request (database trả lời). Khoảng bắt đầu rộng cho lần migrate đầu; container chỉ chạy lệnh
# một lần (`migrate up`, `doctor`) không bị ảnh hưởng — compose tắt kiểm cho chúng.
HEALTHCHECK --interval=30s --timeout=5s --start-period=60s --retries=3 CMD ["/usr/local/bin/autoxide", "healthcheck"]
ENTRYPOINT ["autoxide"]
CMD ["serve"]
