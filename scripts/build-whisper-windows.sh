#!/usr/bin/env bash
# Developer build step. End users never run this script.
set -euo pipefail
tools=/workspace/.tools
compiler="$tools/llvm-mingw-20261006-ucrt-ubuntu-22.04-x86_64"
test "$(git -C "$tools/whisper.cpp" rev-parse HEAD)" = 48f628a84833905ee4a0658ee6d4a5c915ce1997
echo "5f9c6ed95b2d4bdb2869a488c5fd5857fbdabcf288a0aa3eb1da43f6a08d8ab4  $tools/llvm-mingw.tar.xz" | sha256sum --check --status
cat > "$tools/whisper-mingw-toolchain.cmake" <<EOF
set(CMAKE_SYSTEM_NAME Windows)
set(CMAKE_SYSTEM_PROCESSOR AMD64)
set(CMAKE_C_COMPILER $compiler/bin/x86_64-w64-mingw32-clang)
set(CMAKE_CXX_COMPILER $compiler/bin/x86_64-w64-mingw32-clang++)
set(CMAKE_RC_COMPILER $compiler/bin/x86_64-w64-mingw32-windres)
EOF
cmake_bin="$tools/cmake-python/cmake/data/bin/cmake"
"$cmake_bin" -S "$tools/whisper.cpp" -B "$tools/whisper-windows-portable" \
  -DCMAKE_TOOLCHAIN_FILE="$tools/whisper-mingw-toolchain.cmake" \
  -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF -DGGML_NATIVE=OFF \
  -DGGML_OPENMP=OFF -DGGML_AVX=OFF -DGGML_AVX2=OFF -DGGML_FMA=OFF -DGGML_F16C=OFF \
  -DWHISPER_BUILD_TESTS=OFF -DWHISPER_BUILD_EXAMPLES=ON -DWHISPER_BUILD_SERVER=OFF \
  -DWHISPER_CURL=OFF -DCMAKE_CXX_FLAGS='-include algorithm -include new' -DCMAKE_EXE_LINKER_FLAGS=-static
# Explicit standard-library includes compensate for upstream transitive-include assumptions.
"$cmake_bin" --build "$tools/whisper-windows-portable" --target whisper-cli -j 3
