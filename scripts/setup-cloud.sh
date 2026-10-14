#!/usr/bin/env bash
set -euo pipefail
# Each task is already isolated. Use the existing checkout; do not create worktrees.
export CARGO_HOME=/workspace/.tools/cargo
export RUSTUP_HOME=/workspace/.tools/rustup
export PATH="$CARGO_HOME/bin:$PATH"
mkdir -p /workspace/.tools
if ! command -v cargo >/dev/null; then
  curl --fail --silent --show-error --proto '=https' --tlsv1.2 https://sh.rustup.rs -o /tmp/openairecorder-rustup.sh
  sh /tmp/openairecorder-rustup.sh -y --profile minimal --default-toolchain stable --no-modify-path
fi
rustup component add rustfmt clippy
command -v ffmpeg >/dev/null || { echo 'ffmpeg must be installed by the base environment'; exit 1; }
if [ ! -x /workspace/.tools/cmake-python/cmake/data/bin/cmake ]; then
  python -m pip install --target /workspace/.tools/cmake-python cmake==3.31.10
fi
if [ ! -d /workspace/.tools/whisper.cpp ]; then
  git clone --depth 1 --branch v1.8.7 https://github.com/ggml-org/whisper.cpp.git /workspace/.tools/whisper.cpp
fi
test "$(git -C /workspace/.tools/whisper.cpp rev-parse HEAD)" = 48f628a84833905ee4a0658ee6d4a5c915ce1997
cmake_bin=/workspace/.tools/cmake-python/cmake/data/bin/cmake
"$cmake_bin" -S /workspace/.tools/whisper.cpp -B /workspace/.tools/whisper.cpp/build -DCMAKE_BUILD_TYPE=Release -DGGML_NATIVE=OFF -DWHISPER_BUILD_TESTS=OFF -DWHISPER_BUILD_EXAMPLES=ON -DWHISPER_CURL=OFF -DGGML_OPENMP=OFF
"$cmake_bin" --build /workspace/.tools/whisper.cpp/build --target whisper-cli -j 3
cd /workspace/openairecorder
cargo build --locked
cargo test --locked
model=/workspace/.tools/whisper.cpp/models/ggml-tiny.bin
if [ ! -s "$model" ]; then
  curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin?download=true' -o "$model.download"
  echo "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21  $model.download" | sha256sum --check --status
  mv "$model.download" "$model"
fi
echo "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21  $model" | sha256sum --check --status
