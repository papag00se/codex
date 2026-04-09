#!/bin/bash
# Build environment setup for machines without libssl-dev / libcap-dev
# Run: source codex-rs/routing/build-env.sh
# Then: cargo build -p codex-cli

export OPENSSL_STATIC=1
export OPENSSL_INCLUDE_DIR=/home/jesse/.local/ssl/usr/include
export OPENSSL_LIB_DIR=/home/jesse/.local/ssl/usr/lib/x86_64-linux-gnu
export OPENSSL_NO_PKG_CONFIG=1
export PKG_CONFIG_PATH="/home/jesse/.local/cap/usr/lib/x86_64-linux-gnu/pkgconfig:${PKG_CONFIG_PATH:-}"
export C_INCLUDE_PATH="/home/jesse/.local/cap/usr/include:${C_INCLUDE_PATH:-}"
export LIBRARY_PATH="/home/jesse/.local/cap/usr/lib/x86_64-linux-gnu:${LIBRARY_PATH:-}"

echo "Build environment configured for codex-rs"
