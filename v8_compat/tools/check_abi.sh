#!/usr/bin/env bash
# Verify V8/Node ABI 137 constants against Node 24.3.0 headers.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TOOLS="$ROOT/v8_compat/tools"
GEN_HDR="$ROOT/v8_compat/cpp/include/abi_137_generated.h"
NODE_COMMIT="741975041995a272ff7e378bcbb6d6fa4b93f38f"

assert_node_version_header() {
  local header="$1"
  if ! grep -q '#define NODE_MAJOR_VERSION 24' "$header"; then
    echo "ABI check: expected Node 24.x in $header" >&2
    exit 1
  fi
  if ! grep -q '#define NODE_MINOR_VERSION 3' "$header"; then
    echo "ABI check: expected Node 24.3.x in $header" >&2
    exit 1
  fi
  if ! grep -q '#define NODE_PATCH_VERSION 0' "$header"; then
    echo "ABI check: expected Node 24.3.0 in $header" >&2
    exit 1
  fi
}

node_include_dirs() {
  if [[ -n "${LEXE_NODE24_INCLUDE:-}" ]]; then
    echo "${LEXE_NODE24_INCLUDE}"
    if [[ -f "${LEXE_NODE24_INCLUDE}/v8.h" ]]; then
      echo "${LEXE_NODE24_INCLUDE}"
    else
      echo "${LEXE_NODE24_INCLUDE}/../deps/v8/include"
    fi
    return
  fi
  if [[ -n "${HOME:-}" ]]; then
    local nvm="$HOME/.nvm/versions/node/v24.3.0/include/node"
    if [[ -f "$nvm/v8.h" ]]; then
      echo "$nvm"
      echo "$nvm"
      return
    fi
  fi
  local node_src="$ROOT/refs/node/src"
  local v8_include="$ROOT/refs/node/deps/v8/include"
  if [[ -f "$node_src/node_version.h" ]]; then
    echo "$node_src"
    echo "$v8_include"
    return
  fi
  echo "Node 24.3.0 headers not found. Install via nvm, set LEXE_NODE24_INCLUDE, or init refs/node." >&2
  exit 1
}

NODE_DIRS="$(node_include_dirs)"
NODE_INC="$(echo "$NODE_DIRS" | sed -n '1p')"
V8_INC="$(echo "$NODE_DIRS" | sed -n '2p')"

if [[ ! -f "$NODE_INC/node_version.h" && -f "$NODE_INC/src/node_version.h" ]]; then
  NODE_INC="$NODE_INC/src"
fi
assert_node_version_header "$NODE_INC/node_version.h"

[[ -f "$NODE_INC/node_version.h" ]] || {
  echo "ABI check: missing $NODE_INC/node_version.h" >&2
  exit 1
}
[[ -f "$V8_INC/v8.h" ]] || {
  echo "ABI check: missing $V8_INC/v8.h" >&2
  exit 1
}

CXX="${CXX:-clang++}"

build_probe() {
  local src="$1"
  local out="$2"
  shift 2
  "$CXX" -std=c++20 -DNODE_MODULE_VERSION=137 \
    -I"$NODE_INC" \
    -I"$V8_INC" \
    -I"$ROOT/v8_compat/cpp/include" \
    "$src" -o "$out" "$@"
}

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

expect_layout() {
  local name="$1"
  local want="$2"
  local got
  got="$(echo "$LAYOUT_OUT" | sed -n "s/^${name}=//p")"
  if [[ "$got" != "$want" ]]; then
    echo "ABI drift: $name expected $want, got $got" >&2
    exit 1
  fi
}

build_probe "$TOOLS/layout_probe.cc" "$TMP/layout_probe"
LAYOUT_OUT="$("$TMP/layout_probe")"

expect_layout kIsolateHandleScopeDataOffset 560
expect_layout kIsolateRootsOffset 640
expect_layout kHandleScopeDataSize 24

HANDLE_SCOPE_SIZE="$(echo "$LAYOUT_OUT" | sed -n 's/^sizeof(HandleScope)=//p')"
ESCAPABLE_SIZE="$(echo "$LAYOUT_OUT" | sed -n 's/^sizeof(EscapableHandleScope)=//p')"
if [[ -z "$HANDLE_SCOPE_SIZE" || -z "$ESCAPABLE_SIZE" || "$ESCAPABLE_SIZE" -lt "$HANDLE_SCOPE_SIZE" ]]; then
  echo "ABI drift: EscapableHandleScope=$ESCAPABLE_SIZE HandleScope=$HANDLE_SCOPE_SIZE" >&2
  exit 1
fi

build_probe "$TOOLS/abi-generator.cc" "$TMP/abi-generator"
ABI_JSON="$("$TMP/abi-generator")"

expect_macro() {
  local macro="$1"
  local want="$2"
  local got
  got="$(grep "#define $macro " "$GEN_HDR" | awk '{print $3}')"
  if [[ "$got" != "$want" ]]; then
    echo "ABI drift: $macro in abi_137_generated.h is $got, Node 24.3.0 has $want" >&2
    exit 1
  fi
}

json_int() {
  local key="$1"
  echo "$ABI_JSON" | sed -n "s/.*\"$key\": \\([0-9-]*\\).*/\\1/p" | head -1
}

expect_macro LEXE_V8_K_UNDEFINED_VALUE_ROOT_INDEX "$(json_int kUndefinedValueRootIndex)"
expect_macro LEXE_V8_K_THE_HOLE_VALUE_ROOT_INDEX "$(json_int kTheHoleValueRootIndex)"
expect_macro LEXE_V8_K_NULL_VALUE_ROOT_INDEX "$(json_int kNullValueRootIndex)"
expect_macro LEXE_V8_K_TRUE_VALUE_ROOT_INDEX "$(json_int kTrueValueRootIndex)"
expect_macro LEXE_V8_K_FALSE_VALUE_ROOT_INDEX "$(json_int kFalseValueRootIndex)"
expect_macro LEXE_V8_K_EMPTY_STRING_ROOT_INDEX "$(json_int kEmptyStringRootIndex)"
expect_macro LEXE_V8_FUNCTION_CALLBACK_K_RETURN_VALUE_INDEX "$(json_int function_callback_k_return_value_index)"
expect_macro LEXE_V8_PROPERTY_CALLBACK_K_RETURN_VALUE_INDEX "$(json_int property_callback_k_return_value_index)"
expect_macro LEXE_V8_PROPERTY_CALLBACK_K_THIS_INDEX "$(json_int property_callback_k_this_index)"

if git -C "$ROOT/refs/node" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  actual="$(git -C "$ROOT/refs/node" rev-parse HEAD)"
  if [[ "$actual" != "$NODE_COMMIT" ]]; then
    echo "ABI check: refs/node HEAD is $actual, expected $NODE_COMMIT" >&2
    exit 1
  fi
fi

echo "V8 ABI 137 check OK (Node 24.3.0 headers at $NODE_INC)"
