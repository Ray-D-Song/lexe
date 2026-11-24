#pragma once

#include <cstdint>
#include <mutex>
#include <unordered_map>

#include <v8.h>

namespace lexe_v8 {

struct AccessorRecord {
  v8::AccessorNameGetterCallback getter = nullptr;
  uint64_t data_root_id = 0;
};

class AccessorRegistry {
 public:
  static AccessorRegistry& instance();

  uint32_t register_accessor(v8::AccessorNameGetterCallback getter, uint64_t data_root_id);
  AccessorRecord* accessor_at(uint32_t id);
  void erase_accessor(uint32_t id);

 private:
  std::mutex mutex_;
  uint32_t next_accessor_id_ = 1;
  std::unordered_map<uint32_t, AccessorRecord> accessors_;
};

}  // namespace lexe_v8
