#include "registry_ownership.h"

#include "accessor_registry.h"
#include "function_registry.h"
#include "internal.h"
#include "template_registry.h"
#include "v8_bridge_helpers.h"

namespace lexe_v8 {

RegistryOwnership& RegistryOwnership::instance() {
  static RegistryOwnership ownership;
  return ownership;
}

uintptr_t current_quickjs_context_key() {
  auto* ctx = bridge_ctx();
  if (!ctx) {
    return 0;
  }
  return ctx_impl(ctx)->quickjs_context_key;
}

void RegistryOwnership::track(uintptr_t context_key, RegistryKind kind, uint32_t id) {
  if (context_key == 0 || id == 0) {
    return;
  }
  std::lock_guard<std::mutex> lock(mutex_);
  by_context_[context_key].emplace_back(kind, id);
}

void RegistryOwnership::clear_context(uintptr_t context_key) {
  std::vector<std::pair<RegistryKind, uint32_t>> entries;
  {
    std::lock_guard<std::mutex> lock(mutex_);
    auto it = by_context_.find(context_key);
    if (it == by_context_.end()) {
      return;
    }
    entries = std::move(it->second);
    by_context_.erase(it);
  }
  for (const auto& [kind, id] : entries) {
    switch (kind) {
      case RegistryKind::ObjectTemplate:
        TemplateRegistry::instance().erase_object_template(id);
        break;
      case RegistryKind::FunctionTemplate:
        TemplateRegistry::instance().erase_function_template(id);
        break;
      case RegistryKind::Function:
        FunctionRegistry::instance().erase_function(id);
        break;
      case RegistryKind::Accessor:
        AccessorRegistry::instance().erase_accessor(id);
        break;
    }
  }
}

size_t RegistryOwnership::function_template_ids_for_context(uintptr_t context_key,
                                                              uint32_t* out,
                                                              size_t capacity) {
  std::lock_guard<std::mutex> lock(mutex_);
  auto it = by_context_.find(context_key);
  if (it == by_context_.end()) {
    return 0;
  }
  size_t count = 0;
  for (const auto& [kind, id] : it->second) {
    if (kind != RegistryKind::FunctionTemplate) {
      continue;
    }
    if (count < capacity) {
      out[count] = id;
    }
    count++;
  }
  return count;
}

}  // namespace lexe_v8

extern "C" size_t lexe_v8_function_template_ids_for_context(uintptr_t context_key,
                                                              uint32_t* out,
                                                              size_t capacity) {
  return lexe_v8::RegistryOwnership::instance().function_template_ids_for_context(
      context_key, out, capacity);
}

extern "C" void lexe_v8_clear_registries_for_context(uintptr_t context_key) {
  lexe_v8::RegistryOwnership::instance().clear_context(context_key);
}
