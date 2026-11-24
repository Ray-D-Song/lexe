#include "template_registry.h"

#include "registry_ownership.h"

namespace lexe_v8 {

TemplateRegistry& TemplateRegistry::instance() {
  static TemplateRegistry registry;
  return registry;
}

uint32_t TemplateRegistry::register_object_template() {
  std::lock_guard<std::mutex> lock(mutex_);
  uint32_t id = next_object_template_id_++;
  object_templates_[id] = ObjectTemplateRecord{};
  RegistryOwnership::instance().track(
      current_quickjs_context_key(), RegistryKind::ObjectTemplate, id);
  return id;
}

ObjectTemplateRecord* TemplateRegistry::object_template_at(uint32_t id) {
  std::lock_guard<std::mutex> lock(mutex_);
  auto it = object_templates_.find(id);
  if (it == object_templates_.end()) {
    return nullptr;
  }
  return &it->second;
}

uint32_t TemplateRegistry::register_function_template(v8::FunctionCallback callback,
                                                      uint64_t data_root_id,
                                                      v8::ConstructorBehavior behavior) {
  std::lock_guard<std::mutex> lock(mutex_);
  uint32_t instance_template_id = next_object_template_id_++;
  object_templates_[instance_template_id] = ObjectTemplateRecord{};
  uint32_t prototype_template_id = next_object_template_id_++;
  object_templates_[prototype_template_id] = ObjectTemplateRecord{};
  uint32_t id = next_function_template_id_++;
  FunctionTemplateRecord rec{};
  rec.callback = callback;
  rec.data_root_id = data_root_id;
  rec.behavior = behavior;
  rec.instance_template_id = instance_template_id;
  rec.prototype_template_id = prototype_template_id;
  function_templates_[id] = rec;
  RegistryOwnership::instance().track(
      current_quickjs_context_key(), RegistryKind::FunctionTemplate, id);
  return id;
}

FunctionTemplateRecord* TemplateRegistry::function_template_at(uint32_t id) {
  std::lock_guard<std::mutex> lock(mutex_);
  auto it = function_templates_.find(id);
  if (it == function_templates_.end()) {
    return nullptr;
  }
  return &it->second;
}

void TemplateRegistry::set_function_template_prototype_root(uint32_t template_id, uint64_t root_id) {
  std::lock_guard<std::mutex> lock(mutex_);
  auto it = function_templates_.find(template_id);
  if (it != function_templates_.end()) {
    it->second.installed_prototype_root_id = root_id;
  }
}

uint64_t TemplateRegistry::function_template_prototype_root(uint32_t template_id) {
  std::lock_guard<std::mutex> lock(mutex_);
  auto it = function_templates_.find(template_id);
  if (it == function_templates_.end()) {
    return 0;
  }
  return it->second.installed_prototype_root_id;
}

void TemplateRegistry::erase_object_template(uint32_t id) {
  std::lock_guard<std::mutex> lock(mutex_);
  object_templates_.erase(id);
}

void TemplateRegistry::erase_function_template(uint32_t id) {
  std::lock_guard<std::mutex> lock(mutex_);
  auto it = function_templates_.find(id);
  if (it == function_templates_.end()) {
    return;
  }
  object_templates_.erase(it->second.instance_template_id);
  object_templates_.erase(it->second.prototype_template_id);
  function_templates_.erase(it);
}

}  // namespace lexe_v8
