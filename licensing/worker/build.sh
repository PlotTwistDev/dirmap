#!/bin/sh
# Build the Worker. Called by `wrangler deploy` (see [build] in wrangler.toml).
#
# ring compiles C code to WebAssembly. The system `ar` on macOS (and GNU ar) can't index wasm
# objects, which leaves "undefined symbol: ring_core_..." link errors, so point the C build at
# the llvm-ar that ships with the Rust toolchain (`rustup component add llvm-tools`).
set -e
if [ -z "$AR_wasm32_unknown_unknown" ]; then
  host=$(rustc -vV | sed -n 's/^host: //p')
  llvm_ar="$(rustc --print sysroot)/lib/rustlib/$host/bin/llvm-ar"
  if [ -x "$llvm_ar" ]; then
    export AR_wasm32_unknown_unknown="$llvm_ar"
  elif command -v llvm-ar >/dev/null 2>&1; then
    export AR_wasm32_unknown_unknown=llvm-ar
  else
    echo "llvm-ar not found. Run: rustup component add llvm-tools" >&2
    exit 1
  fi
fi
command -v worker-build >/dev/null 2>&1 || cargo install worker-build
exec worker-build --release
