#pragma once

#include <cstdint>
#include <mutex>
#include <unordered_map>

#include <v8.h>

#include "template_registry.h"

namespace lexe_v8 {

struct FunctionRecord {
  uint32_t template_id = 0;
};

class FunctionRegistry {
 public:
  static FunctionRegistry& instance();

  uint32_t register_template(v8::FunctionCallback callback, uint64_t data_root_id);
  FunctionTemplateRecord* template_at(uint32_t id);

  uint32_t register_function(uint32_t template_id);
  FunctionRecord* function_at(uint32_t id);
  void erase_function(uint32_t id);

 private:
  std::mutex mutex_;
  uint32_t next_function_id_ = 1;
  std::unordered_map<uint32_t, FunctionRecord> functions_;
};

}  // namespace lexe_v8
