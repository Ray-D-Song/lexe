#include "template_registry.h"
#include "accessor_registry.h"
#include "v8_bridge_helpers.h"

#include <v8.h>

namespace lexe_v8 {

static shim::ObjectLayout* alloc_template_object(uint32_t template_id, const shim::Map* map) {
  auto* ctx = bridge_ctx();
  if (!ctx) {
    return nullptr;
  }
  auto* slot = alloc_handle_slot(ctx);
  slot->object = shim::ObjectLayout(const_cast<shim::Map*>(map), template_id);
  slot->tagged_value = shim::TaggedPointer(&slot->object);
  slot->owns_root = false;
  return &slot->object;
}

}  // namespace lexe_v8

namespace v8 {

void Template::Set(Local<Name> name, Local<Data> value, PropertyAttribute attributes) {
  uint32_t template_id = static_cast<uint32_t>(lexe_v8::root_from_object(this));
  auto* rec = lexe_v8::TemplateRegistry::instance().object_template_at(template_id);
  if (!rec) {
    auto* fn_rec = lexe_v8::TemplateRegistry::instance().function_template_at(template_id);
    if (!fn_rec) {
      return;
    }
    rec = lexe_v8::TemplateRegistry::instance().object_template_at(fn_rec->instance_template_id);
  }
  if (!rec) {
    return;
  }
  rec->properties.emplace_back(
      lexe_v8::root_from_local(name),
      lexe_v8::root_from_local(Local<Value>::Cast(value)), attributes);
}

void Template::SetNativeDataProperty(
    Local<Name> name,
    AccessorNameGetterCallback getter,
    AccessorNameSetterCallback setter,
    Local<Value> data,
    PropertyAttribute attribute,
    SideEffectType getter_side_effect_type,
    SideEffectType setter_side_effect_type) {
  (void)getter_side_effect_type;
  (void)setter_side_effect_type;
  uint32_t template_id = static_cast<uint32_t>(lexe_v8::root_from_object(this));
  auto* fn_rec = lexe_v8::TemplateRegistry::instance().function_template_at(template_id);
  auto* rec = fn_rec ? lexe_v8::TemplateRegistry::instance().object_template_at(
                           fn_rec->prototype_template_id)
                     : lexe_v8::TemplateRegistry::instance().object_template_at(template_id);
  if (!rec) {
    return;
  }
  lexe_v8::NativeDataProperty prop{};
  prop.name_root_id = lexe_v8::root_from_local(name);
  prop.getter = getter;
  prop.setter = setter;
  prop.data_root_id = data.IsEmpty() ? 0 : lexe_v8::root_from_local(data);
  prop.attribute = attribute;
  prop.accessor_id = lexe_v8::AccessorRegistry::instance().register_accessor(
      getter, prop.data_root_id);
  rec->native_properties.push_back(prop);
}

Local<ObjectTemplate> FunctionTemplate::InstanceTemplate() {
  uint32_t template_id = static_cast<uint32_t>(lexe_v8::root_from_object(this));
  auto* rec = lexe_v8::TemplateRegistry::instance().function_template_at(template_id);
  if (!rec) {
    return Local<ObjectTemplate>();
  }
  auto* object = lexe_v8::alloc_template_object(rec->instance_template_id,
                                                  &lexe_v8::shim::Map::object_map());
  if (!object) {
    return Local<ObjectTemplate>();
  }
  return v8::MakeLocalFromObject<ObjectTemplate>(Isolate::GetCurrent(), object);
}

Local<ObjectTemplate> FunctionTemplate::PrototypeTemplate() {
  uint32_t template_id = static_cast<uint32_t>(lexe_v8::root_from_object(this));
  auto* rec = lexe_v8::TemplateRegistry::instance().function_template_at(template_id);
  if (!rec) {
    return Local<ObjectTemplate>();
  }
  auto* object = lexe_v8::alloc_template_object(rec->prototype_template_id,
                                                  &lexe_v8::shim::Map::object_map());
  if (!object) {
    return Local<ObjectTemplate>();
  }
  return v8::MakeLocalFromObject<ObjectTemplate>(Isolate::GetCurrent(), object);
}

void FunctionTemplate::SetClassName(Local<String> name) {
  uint32_t template_id = static_cast<uint32_t>(lexe_v8::root_from_object(this));
  auto* rec = lexe_v8::TemplateRegistry::instance().function_template_at(template_id);
  if (!rec) {
    return;
  }
  rec->class_name_root_id = lexe_v8::root_from_local(name);
}

}  // namespace v8
