FROM rust:bookworm AS builder
ARG DEBIAN_FRONTEND=noninteractive
RUN apt-get update
RUN apt-get install -y --no-install-recommends \
  build-essential \
  protobuf-compiler \
  llvm \
  clang \
  libclang-dev \
  libssl-dev \
  git-core \
  pkg-config \
  curl zip unzip tar \
  nasm \
  && apt-get clean \
  && rm -rf /tmp/* /var/tmp/*

WORKDIR /builder
COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry cargo install cargo-vcpkg
RUN --mount=type=cache,target=/usr/local/cargo/registry cargo vcpkg --verbose build
RUN --mount=type=cache,target=/usr/local/cargo/registry FFMPEG_PKG_CONFIG_PATH=/builder/target/vcpkg/installed/x64-linux/lib/pkgconfig cargo build --release --package wanna-cdn

FROM debian:bookworm-slim
ARG DEBIAN_FRONTEND=noninteractive
RUN apt-get update
RUN apt-get install -y --no-install-recommends \
  libssl-dev iproute2 \
  ca-certificates curl \
  && apt-get clean \
  && rm -rf /tmp/* /var/tmp/*

WORKDIR /app
COPY --from=builder /builder/target/release/wanna-cdn .

ENV RUST_LOG=info

LABEL org.opencontainers.image.source https://github.com/ClownpieceStripedAbyss/aya-dance-server

CMD /app/wanna-cdn
