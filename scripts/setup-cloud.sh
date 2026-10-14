#!/usr/bin/env bash
# Cloud developer setup, never an end-user installer.
set -euo pipefail
export CARGO_HOME=/workspace/.tools/cargo RUSTUP_HOME=/workspace/.tools/rustup
export PATH="$CARGO_HOME/bin:$PATH"
mkdir -p /workspace/.tools
if ! command -v cargo >/dev/null; then
  curl --fail --silent --show-error --proto '=https' --tlsv1.2 https://sh.rustup.rs -o /tmp/openairecorder-rustup.sh
  sh /tmp/openairecorder-rustup.sh -y --profile minimal --default-toolchain stable --no-modify-path
fi
rustup component add rustfmt clippy
cd /workspace/openairecorder
cargo build --locked -j 3
cargo test --locked -j 3
