#include "function_registry.h"
#include "registry_ownership.h"
#include "template_registry.h"

namespace lexe_v8 {

FunctionRegistry& FunctionRegistry::instance() {
  static FunctionRegistry registry;
  return registry;
}

uint32_t FunctionRegistry::register_template(v8::FunctionCallback callback, uint64_t data_root_id) {
  return TemplateRegistry::instance().register_function_template(
      callback, data_root_id, v8::ConstructorBehavior::kAllow);
}

FunctionTemplateRecord* FunctionRegistry::template_at(uint32_t id) {
  return TemplateRegistry::instance().function_template_at(id);
}

uint32_t FunctionRegistry::register_function(uint32_t template_id) {
  std::lock_guard<std::mutex> lock(mutex_);
  uint32_t id = next_function_id_++;
  functions_[id] = FunctionRecord{template_id};
  RegistryOwnership::instance().track(
      current_quickjs_context_key(), RegistryKind::Function, id);
  return id;
}

FunctionRecord* FunctionRegistry::function_at(uint32_t id) {
  std::lock_guard<std::mutex> lock(mutex_);
  auto it = functions_.find(id);
  if (it == functions_.end()) {
    return nullptr;
  }
  return &it->second;
}

void FunctionRegistry::erase_function(uint32_t id) {
  std::lock_guard<std::mutex> lock(mutex_);
  functions_.erase(id);
}

}  // namespace lexe_v8

extern "C" uint32_t lexe_v8_function_template_id(uint32_t function_id) {
  auto* rec = lexe_v8::FunctionRegistry::instance().function_at(function_id);
  return rec ? rec->template_id : 0;
}

extern "C" int32_t lexe_v8_instance_internal_field_count(uint32_t template_id) {
  auto* fn = lexe_v8::TemplateRegistry::instance().function_template_at(template_id);
  if (!fn) {
    return 0;
  }
  auto* inst = lexe_v8::TemplateRegistry::instance().object_template_at(fn->instance_template_id);
  return inst ? inst->internal_field_count : 0;
}

extern "C" uint32_t lexe_v8_function_prototype_template_id(uint32_t template_id) {
  auto* rec = lexe_v8::TemplateRegistry::instance().function_template_at(template_id);
  return rec ? rec->prototype_template_id : 0;
}

extern "C" void lexe_v8_set_function_template_prototype_root(uint32_t template_id,
                                                               uint64_t root_id) {
  lexe_v8::TemplateRegistry::instance().set_function_template_prototype_root(template_id,
                                                                                root_id);
}

extern "C" uint64_t lexe_v8_function_template_prototype_root(uint32_t template_id) {
  return lexe_v8::TemplateRegistry::instance().function_template_prototype_root(template_id);
}

extern "C" size_t lexe_v8_object_template_property_count(uint32_t object_template_id) {
  auto* rec = lexe_v8::TemplateRegistry::instance().object_template_at(object_template_id);
  return rec ? rec->properties.size() : 0;
}

extern "C" int lexe_v8_object_template_property_at(uint32_t object_template_id,
                                                     size_t index,
                                                     uint64_t* key_root,
                                                     uint32_t* value_template_id) {
  if (!key_root || !value_template_id) {
    return 0;
  }
  auto* rec = lexe_v8::TemplateRegistry::instance().object_template_at(object_template_id);
  if (!rec || index >= rec->properties.size()) {
    return 0;
  }
  const auto& prop = rec->properties[index];
  *key_root = std::get<0>(prop);
  *value_template_id = static_cast<uint32_t>(std::get<1>(prop));
  return 1;
}

extern "C" uint32_t lexe_v8_register_function_for_template(uint32_t template_id) {
  return lexe_v8::FunctionRegistry::instance().register_function(template_id);
}

extern "C" size_t lexe_v8_object_template_native_property_count(uint32_t object_template_id) {
  auto* rec = lexe_v8::TemplateRegistry::instance().object_template_at(object_template_id);
  return rec ? rec->native_properties.size() : 0;
}

extern "C" int lexe_v8_object_template_native_property_at(uint32_t object_template_id,
                                                             size_t index,
                                                             uint64_t* name_root,
                                                             uint32_t* accessor_id) {
  if (!name_root || !accessor_id) {
    return 0;
  }
  auto* rec = lexe_v8::TemplateRegistry::instance().object_template_at(object_template_id);
  if (!rec || index >= rec->native_properties.size()) {
    return 0;
  }
  const auto& prop = rec->native_properties[index];
  *name_root = prop.name_root_id;
  *accessor_id = prop.accessor_id;
  return 1;
}
