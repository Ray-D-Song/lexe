#pragma once

#include <cstdint>
#include <mutex>
#include <unordered_map>
#include <vector>

#include <v8.h>

namespace lexe_v8 {

struct NativeDataProperty {
  uint64_t name_root_id = 0;
  uint32_t accessor_id = 0;
  v8::AccessorNameGetterCallback getter = nullptr;
  v8::AccessorNameSetterCallback setter = nullptr;
  uint64_t data_root_id = 0;
  v8::PropertyAttribute attribute = v8::None;
};

struct ObjectTemplateRecord {
  int internal_field_count = 0;
  std::vector<std::tuple<uint64_t, uint64_t, v8::PropertyAttribute>> properties;
  std::vector<NativeDataProperty> native_properties;
};

struct FunctionTemplateRecord {
  v8::FunctionCallback callback = nullptr;
  uint64_t data_root_id = 0;
  uint64_t class_name_root_id = 0;
  uint32_t instance_template_id = 0;
  uint32_t prototype_template_id = 0;
  uint64_t installed_prototype_root_id = 0;
  v8::ConstructorBehavior behavior = v8::ConstructorBehavior::kAllow;
};

class TemplateRegistry {
 public:
  static TemplateRegistry& instance();

  uint32_t register_object_template();
  ObjectTemplateRecord* object_template_at(uint32_t id);

  uint32_t register_function_template(v8::FunctionCallback callback,
                                      uint64_t data_root_id,
                                      v8::ConstructorBehavior behavior);
  FunctionTemplateRecord* function_template_at(uint32_t id);
  void set_function_template_prototype_root(uint32_t template_id, uint64_t root_id);
  uint64_t function_template_prototype_root(uint32_t template_id);
  void erase_object_template(uint32_t id);
  void erase_function_template(uint32_t id);

 private:
  std::mutex mutex_;
  uint32_t next_object_template_id_ = 1;
  uint32_t next_function_template_id_ = 1;
  std::unordered_map<uint32_t, ObjectTemplateRecord> object_templates_;
  std::unordered_map<uint32_t, FunctionTemplateRecord> function_templates_;
};

}  // namespace lexe_v8
