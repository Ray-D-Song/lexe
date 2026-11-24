#pragma once

#include <cstdint>
#include <mutex>
#include <unordered_map>
#include <vector>

namespace lexe_v8 {

enum class RegistryKind : uint8_t {
  ObjectTemplate = 1,
  FunctionTemplate = 2,
  Function = 3,
  Accessor = 4,
};

class RegistryOwnership {
 public:
  static RegistryOwnership& instance();

  void track(uintptr_t context_key, RegistryKind kind, uint32_t id);
  void clear_context(uintptr_t context_key);
  size_t function_template_ids_for_context(uintptr_t context_key,
                                           uint32_t* out,
                                           size_t capacity);

 private:
  std::mutex mutex_;
  std::unordered_map<uintptr_t, std::vector<std::pair<RegistryKind, uint32_t>>> by_context_;
};

uintptr_t current_quickjs_context_key();

}  // namespace lexe_v8
