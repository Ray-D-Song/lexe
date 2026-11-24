#pragma once

// Node 24.3.0 / ABI 137 layout constants (measured from Node 24 headers).
// Keep in sync with v8_compat/tools/layout_probe.cc.

#include <cstddef>
#include <cstdint>

namespace lexe_v8 {
namespace abi137 {

constexpr size_t kIsolateHandleScopeDataOffset = 560;
constexpr size_t kIsolateRootsOffset = 640;
constexpr size_t kHandleScopeDataSize = 24;
constexpr size_t kRootSlotCount = 10;

}  // namespace abi137
}  // namespace lexe_v8
