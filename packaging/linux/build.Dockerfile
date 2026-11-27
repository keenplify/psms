# Linux build environment matching the CI Linux jobs, for scripts/build-linux.sh.
FROM ubuntu:24.04
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential clang curl ca-certificates pkg-config cmake git python3 \
      rpm desktop-file-utils file nodejs \
      libfontconfig-dev libwayland-dev libwebkit2gtk-4.1-dev libxkbcommon-x11-dev \
      libx11-xcb-dev libssl-dev libzstd-dev libasound2-dev libvulkan-dev \
    && rm -rf /var/lib/apt/lists/*
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable \
    && chmod -R a+w /usr/local/rustup /usr/local/cargo
