#!/usr/bin/env bash
# SQLite ASan gate: instrument lexe (Rust + bundled sqlite3.c) and the
# loadable SQLite extension (extension.c).
#
# Critical isolation (host == target triple on Linux CI):
# - Use --target + CARGO_TARGET_*_RUSTFLAGS so Cargo does not pass ASan rustflags
#   to host build scripts / proc-macros (Cargo's documented behavior).
# - Do NOT export global CFLAGS/CXXFLAGS during `cargo build` (would poison host
#   rquickjs-sys used by rquickjs_macro → E0463 / link failure).
# - Instrument SQLite C via package-scoped LEXE_SQLITE_SANITIZE=address
#   (lexe_sqlite/build.rs + build-extension.mjs only).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

TARGET="${TARGET:-x86_64-unknown-linux-gnu}"

if [[ "$TARGET" != "x86_64-unknown-linux-gnu" ]]; then
  echo "sqlite ASan currently supports x86_64-unknown-linux-gnu only" >&2
  exit 1
fi

unset RUSTFLAGS
unset CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS
unset CFLAGS
unset CXXFLAGS
unset RUSTC_WRAPPER

export CARGO_TARGET_DIR="$ROOT/target-sqlite-asan"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS="\
-Zsanitizer=address \
-Cdebuginfo=1"
# Package-scoped: only lexe_sqlite's cc::Build and the extension builder.
export LEXE_SQLITE_SANITIZE=address
# Intentionally no global CFLAGS/CXXFLAGS — see file header.
export CC="${CC:-clang}"
export CXX="${CXX:-clang++}"
export ASAN_OPTIONS="${ASAN_OPTIONS:-detect_leaks=1:abort_on_error=1}"

if ! rustup toolchain list | grep -q nightly; then
  rustup toolchain install nightly
fi

# Consume full stdin so pipefail + early-exit grep cannot yield SIGPIPE (141).
has_asan_syms() {
  local bin="$1"
  readelf -Ws "$bin" 2>/dev/null | grep '__asan' >/dev/null
}

dump_proc_macro_diagnostics() {
  echo "=== [sqlite-asan] rquickjs_macro artifact diagnostics ===" >&2
  local found=0
  local f
  while IFS= read -r -d '' f; do
    found=1
    echo "--- $f" >&2
    file "$f" >&2 || true
    if command -v ldd >/dev/null 2>&1; then
      ldd "$f" >&2 || true
    fi
    if command -v readelf >/dev/null 2>&1; then
      echo "readelf -d:" >&2
      readelf -d "$f" 2>/dev/null | head -40 >&2 || true
      if has_asan_syms "$f"; then
        echo "ASan symbols present in proc-macro (unexpected host pollution):" >&2
        readelf -Ws "$f" 2>/dev/null | grep '__asan' | head -20 >&2 || true
      else
        echo "no ASan symbols in proc-macro (expected)" >&2
      fi
    fi
  done < <(
    find "$CARGO_TARGET_DIR" \
      \( -name 'librquickjs_macro-*.so' -o -name 'librquickjs_macro.so' \) \
      -print0 2>/dev/null || true
  )
  if [[ "$found" -eq 0 ]]; then
    echo "no librquickjs_macro*.so under $CARGO_TARGET_DIR" >&2
    echo "deps dirs:" >&2
    find "$CARGO_TARGET_DIR" -type d -name deps 2>/dev/null | head -20 >&2 || true
  fi
}

# Host proc-macro must exist and must not carry ASan (would indicate flag leak).
assert_proc_macro_host_clean() {
  local found=0
  local f
  while IFS= read -r -d '' f; do
    found=1
    if has_asan_syms "$f"; then
      echo "proc-macro is ASan-instrumented (host isolation failed): $f" >&2
      dump_proc_macro_diagnostics
      exit 1
    fi
    echo "[sqlite-asan] proc-macro clean (no ASan): $f"
  done < <(
    find "$CARGO_TARGET_DIR" \
      \( -name 'librquickjs_macro-*.so' -o -name 'librquickjs_macro.so' \) \
      -print0 2>/dev/null || true
  )
  if [[ "$found" -eq 0 ]]; then
    echo "no rquickjs_macro .so found under $CARGO_TARGET_DIR — cannot assert host isolation" >&2
    dump_proc_macro_diagnostics
    exit 1
  fi
}

sqlite_extension_path() {
  if [[ "$(uname -s)" == "Darwin" ]]; then
    echo "compat/node-sqlite/build/sqlite_extension.dylib"
  elif [[ "$(uname -s)" == "MINGW"* || "$(uname -s)" == "MSYS"* || "$(uname -s)" == "CYGWIN"* ]]; then
    echo "compat/node-sqlite/build/sqlite_extension.dll"
  else
    echo "compat/node-sqlite/build/sqlite_extension.so"
  fi
}

make js JS_MINIFY=0

build_log="$(mktemp)"
trap 'rm -f "$build_log"' EXIT

set +e
cargo +nightly build \
  -p lexe \
  --target "$TARGET" 2>&1 | tee "$build_log"
build_status=${PIPESTATUS[0]}
set -e

if [[ "$build_status" -ne 0 ]]; then
  echo "[sqlite-asan] cargo build failed; dumping proc-macro artifacts" >&2
  dump_proc_macro_diagnostics
  if grep -Eq 'rquickjs_macro|E0463|rquickjs-sys|linking with' "$build_log"; then
    echo "[sqlite-asan] re-running failed build with -vv (macro / E0463 / link)" >&2
    echo "[sqlite-asan] inspect: rquickjs_macro rustc line, artifact path, --extern, ldd" >&2
    cargo +nightly build \
      -p lexe \
      --target "$TARGET" \
      -vv || true
  fi
  exit "$build_status"
fi

assert_proc_macro_host_clean

# Loadable extension: flags come from LEXE_SQLITE_SANITIZE via build-extension.mjs
# (not ambient CFLAGS — spawnSync does not pass env CFLAGS to the compiler).
node compat/node-sqlite/build-extension.mjs

EXTENSION="$(sqlite_extension_path)"
if [[ ! -f "$EXTENSION" ]]; then
  echo "missing SQLite extension: $EXTENSION" >&2
  exit 1
fi
if ! has_asan_syms "$EXTENSION"; then
  echo "SQLite extension is not ASan-instrumented: $EXTENSION" >&2
  exit 1
fi
echo "[sqlite-asan] SQLite extension is ASan-instrumented: $EXTENSION"

LEXE_BIN="$CARGO_TARGET_DIR/$TARGET/debug/lexe"
if ! has_asan_syms "$LEXE_BIN"; then
  echo "binary is not ASan-instrumented: $LEXE_BIN" >&2
  exit 1
fi
echo "[sqlite-asan] lexe is ASan-instrumented: $LEXE_BIN"

SANITIZER_FAIL_RE='ERROR: AddressSanitizer|LeakSanitizer|runtime error:|SIGABRT|assert'

run_and_scan() {
  local label="$1"
  shift
  local output
  output="$("$@" 2>&1)" || {
    echo "$output"
    echo "[$label] command failed" >&2
    exit 1
  }
  echo "$output"
  if grep -Eiq "$SANITIZER_FAIL_RE" <<<"$output"; then
    echo "[$label] sanitizer or abort output detected" >&2
    exit 1
  fi
}

run_and_scan "node-sqlite parity" \
  env COMPAT_SKIP_NODE_BASELINE=1 \
  COMPAT_SQLITE_LIFECYCLE_LOOPS="${COMPAT_SQLITE_LIFECYCLE_LOOPS:-20}" \
  COMPAT_SQLITE_BACKUP_LOOPS="${COMPAT_SQLITE_BACKUP_LOOPS:-5}" \
  node compat/run.mjs node-sqlite "$LEXE_BIN"

run_and_scan "sqlite unit tests" \
  "$LEXE_BIN" test -d bundle/js/__tests__/unit --filter sqlite
