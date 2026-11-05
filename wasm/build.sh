#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
if ! command -v emcc >/dev/null; then
    source "$HOME/emsdk/emsdk_env.sh"
fi
if [[ -z "${EM_CACHE:-}" ]]; then
    SDK_CACHE="$(em-config CACHE)"
    export EM_CACHE="$ROOT/.cache/emscripten"
    if [[ ! -d "$EM_CACHE/sysroot" ]]; then
        mkdir -p "$EM_CACHE"
        cp -R "$SDK_CACHE/" "$EM_CACHE/"
    fi
fi
export CARGO_TARGET_WASM32_UNKNOWN_EMSCRIPTEN_LINKER="$ROOT/wasm/emcc-link.py"
export CARGO_ENCODED_RUSTFLAGS="$(python3 - <<'PY'
print('\x1f'.join(['-C', 'panic=abort', '-C', 'link-args=-s MODULARIZE=1 -s EXPORT_ES6=1 -s ALLOW_MEMORY_GROWTH=1 -s INITIAL_MEMORY=268435456 -s STACK_SIZE=16777216 -s FILESYSTEM=0 -s ENVIRONMENT=web,worker,node -s EXPORTED_RUNTIME_METHODS=HEAPU8 -sDEFAULT_TO_CXX=1']))
PY
)"
cd "$ROOT/wasm"
nice -n 15 cargo build --lib --locked --offline --target wasm32-unknown-emscripten --release -j 4
