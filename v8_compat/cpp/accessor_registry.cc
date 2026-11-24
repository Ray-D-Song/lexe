#include "accessor_registry.h"

#include "registry_ownership.h"

namespace lexe_v8 {

AccessorRegistry& AccessorRegistry::instance() {
  static AccessorRegistry registry;
  return registry;
}

uint32_t AccessorRegistry::register_accessor(v8::AccessorNameGetterCallback getter,
                                             uint64_t data_root_id) {
  std::lock_guard<std::mutex> lock(mutex_);
  uint32_t id = next_accessor_id_++;
  accessors_[id] = AccessorRecord{getter, data_root_id};
  RegistryOwnership::instance().track(
      current_quickjs_context_key(), RegistryKind::Accessor, id);
  return id;
}

AccessorRecord* AccessorRegistry::accessor_at(uint32_t id) {
  std::lock_guard<std::mutex> lock(mutex_);
  auto it = accessors_.find(id);
  if (it == accessors_.end()) {
    return nullptr;
  }
  return &it->second;
}

void AccessorRegistry::erase_accessor(uint32_t id) {
  std::lock_guard<std::mutex> lock(mutex_);
  accessors_.erase(id);
}

}  // namespace lexe_v8
