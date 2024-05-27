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
  && apt-get clean \
  && rm -rf /tmp/* /var/tmp/*

WORKDIR /builder
COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry cargo build --release --package pypy-cdn

FROM debian:bookworm-slim
ARG DEBIAN_FRONTEND=noninteractive
RUN apt-get update
RUN apt-get install -y --no-install-recommends \
  libssl-dev iproute2 \
  ca-certificates curl \
  && apt-get clean \
  && rm -rf /tmp/* /var/tmp/*

WORKDIR /app
COPY --from=builder /builder/target/release/pypy-cdn .

ENV RUST_LOG=info

LABEL org.opencontainers.image.source https://github.com/ClownpieceStripedAbyss/pypy-cdn

CMD /app/pypy-cdn
