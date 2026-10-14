#!/usr/bin/env bash
set -euo pipefail
export CARGO_HOME=/workspace/.tools/cargo RUSTUP_HOME=/workspace/.tools/rustup
export PATH="$CARGO_HOME/bin:$PATH"
tools=/workspace/.tools
compiler_name=llvm-mingw-20261006-ucrt-ubuntu-22.04-x86_64
compiler="$tools/$compiler_name"
if [ ! -f "$tools/llvm-mingw.tar.xz" ]; then
  curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 "https://github.com/mstorsjo/llvm-mingw/releases/download/20261006/$compiler_name.tar.xz" -o "$tools/llvm-mingw.tar.xz"
fi
echo "5f9c6ed95b2d4bdb2869a488c5fd5857fbdabcf288a0aa3eb1da43f6a08d8ab4  $tools/llvm-mingw.tar.xz" | sha256sum --check --status
if [ ! -x "$compiler/bin/x86_64-w64-mingw32-clang" ]; then tar -xf "$tools/llvm-mingw.tar.xz" -C "$tools"; fi
curl --fail --silent --show-error --proto '=https' --tlsv1.2 https://raw.githubusercontent.com/llvm/llvm-project/main/llvm/LICENSE.TXT -o "$tools/llvm-LICENSE.txt"
rustup target add x86_64-pc-windows-gnullvm
export CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER="$compiler/bin/x86_64-w64-mingw32-clang"
cd /workspace/openairecorder
cargo build --locked --release --target x86_64-pc-windows-gnullvm -j 3
bash scripts/build-whisper-windows.sh
python scripts/package-windows.py
