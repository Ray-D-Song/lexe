#include "function_registry.h"
#include "template_registry.h"
#include "internal.h"
#include "v8_local_helpers.h"

#include <node.h>
#include <v8-function-callback.h>
#include <v8-internal.h>
#include <v8-local-handle.h>
#include <v8-object.h>
#include <v8-primitive.h>
#include <v8-template.h>

namespace lexe_v8 {

static LexeV8ContextState* bridge_ctx() {
  return lexe_v8_current_context();
}

static shim::ObjectLayout* alloc_template_object(uint32_t template_id) {
  auto* ctx = bridge_ctx();
  if (!ctx) {
    return nullptr;
  }
  auto* slot = alloc_handle_slot(ctx);
  slot->object =
      shim::ObjectLayout(const_cast<shim::Map*>(&shim::Map::object_map()), template_id);
  slot->tagged_value = shim::TaggedPointer(&slot->object);
  slot->owns_root = false;
  return &slot->object;
}

template <typename T>
static lexe_v8::shim::ObjectLayout* layout_from_local(v8::Local<T> handle) {
  return reinterpret_cast<lexe_v8::shim::ObjectLayout*>(handle.operator->());
}

}  // namespace lexe_v8

namespace v8 {
namespace api_internal {

void ToLocalEmpty() {
  if (const LexeV8BridgeV1* b = lexe_v8_bridge()) {
    b->fatal("ToLocalChecked", "empty MaybeLocal");
  }
}

void FromJustIsNothing() {
  if (const LexeV8BridgeV1* b = lexe_v8_bridge()) {
    b->fatal("FromJust", "empty Maybe");
  }
}

Local<Value> GetFunctionTemplateData(Isolate* isolate, Local<Data> target) {
  (void)isolate;
  uint32_t template_id = lexe_v8::g_active_callback_template_id;
  if (template_id == 0) {
    auto* layout = lexe_v8::layout_from_local(target);
    template_id = static_cast<uint32_t>(layout->contents.root_id);
  }
  auto* rec = lexe_v8::FunctionRegistry::instance().template_at(template_id);
  if (!rec || rec->data_root_id == 0) {
    return Local<Value>();
  }
  auto* ctx = lexe_v8::bridge_ctx();
  auto* object_slot = lexe_v8::alloc_handle_slot(ctx);
  object_slot->object = lexe_v8::shim::ObjectLayout(
      const_cast<lexe_v8::shim::Map*>(&lexe_v8::shim::Map::object_map()), rec->data_root_id);
  return v8::MakeLocalFromObject<Value>(isolate, &object_slot->object);
}

}  // namespace api_internal

Isolate* Isolate::GetCurrent() {
  return reinterpret_cast<Isolate*>(lexe_v8_current_isolate());
}

Local<Context> Isolate::GetCurrentContext() {
  auto* ctx = lexe_v8::bridge_ctx();
  auto* impl = lexe_v8::ctx_impl(ctx);
  auto* object_slot = lexe_v8::alloc_handle_slot(ctx);
  object_slot->object = lexe_v8::shim::ObjectLayout(
      const_cast<lexe_v8::shim::Map*>(&lexe_v8::shim::Map::object_map()), impl->context_root_id);
  return v8::MakeLocalFromObject<Context>(this, &object_slot->object);
}

MaybeLocal<String> String::NewFromUtf8(Isolate* isolate,
                                       const char* data,
                                       NewStringType type,
                                       int length) {
  (void)type;
  const LexeV8BridgeV1* b = lexe_v8_bridge();
  auto* ctx = lexe_v8::bridge_ctx();
  if (!b || !ctx || !data) {
    return MaybeLocal<String>();
  }
  uint64_t root_id = 0;
  if (b->string_new_utf8(ctx, data, length, &root_id) != LEXE_V8_OK) {
    return MaybeLocal<String>();
  }
  auto* slot = lexe_v8::alloc_handle_slot(ctx);
  slot->object = lexe_v8::shim::ObjectLayout(
      const_cast<lexe_v8::shim::Map*>(&lexe_v8::shim::Map::string_map()), root_id);
  return MaybeLocal<String>(v8::MakeLocalFromObject<String>(isolate, &slot->object));
}

Maybe<bool> Object::Set(Local<Context> context,
                        Local<Value> key,
                        Local<Value> value) {
  (void)context;
  const LexeV8BridgeV1* b = lexe_v8_bridge();
  auto* ctx = lexe_v8::bridge_ctx();
  if (!b || !ctx) {
    return Nothing<bool>();
  }
  auto* layout = reinterpret_cast<lexe_v8::shim::ObjectLayout*>(this);
  uint64_t object_root = layout->contents.root_id;
  auto root_from_local = [](v8::Local<v8::Value> local) -> uint64_t {
    return lexe_v8::layout_from_local(local)->contents.root_id;
  };
  uint64_t key_root = root_from_local(key);
  uint64_t value_root = root_from_local(value);
  if (b->object_set(ctx, object_root, key_root, value_root) != LEXE_V8_OK) {
    return Nothing<bool>();
  }
  return Just(true);
}

void Function::SetName(Local<String> name) {
  (void)name;
}

Local<FunctionTemplate> FunctionTemplate::New(
    Isolate* isolate,
    FunctionCallback callback,
    Local<Value> data,
    Local<Signature> signature,
    int length,
    ConstructorBehavior behavior,
    SideEffectType side_effect_type,
    const CFunction* c_function,
    uint16_t instance_type,
    uint16_t allowed_receiver_instance_type_range_start,
    uint16_t allowed_receiver_instance_type_range_end) {
  (void)signature;
  (void)length;
  (void)behavior;
  (void)side_effect_type;
  (void)c_function;
  (void)instance_type;
  (void)allowed_receiver_instance_type_range_start;
  (void)allowed_receiver_instance_type_range_end;
  uint64_t data_root = 0;
  if (!data.IsEmpty()) {
    data_root = lexe_v8::layout_from_local(data)->contents.root_id;
  }
  uint32_t template_id =
      lexe_v8::FunctionRegistry::instance().register_template(callback, data_root);
  auto* object = lexe_v8::alloc_template_object(template_id);
  if (!object) {
    return Local<FunctionTemplate>();
  }
  return v8::MakeLocalFromObject<FunctionTemplate>(isolate, object);
}

MaybeLocal<Function> FunctionTemplate::GetFunction(Local<Context> context) {
  (void)context;
  const LexeV8BridgeV1* b = lexe_v8_bridge();
  auto* ctx = lexe_v8::bridge_ctx();
  if (!b || !ctx) {
    return MaybeLocal<Function>();
  }
  uint32_t template_id =
      static_cast<uint32_t>(reinterpret_cast<lexe_v8::shim::ObjectLayout*>(this)->contents.root_id);
  if (template_id == 0) {
    return MaybeLocal<Function>();
  }
  uint32_t function_id = lexe_v8::FunctionRegistry::instance().register_function(template_id);
  uint64_t root_id = 0;
  if (b->function_template_get_function(ctx, function_id, &root_id) != LEXE_V8_OK) {
    return MaybeLocal<Function>();
  }
  auto* slot = lexe_v8::alloc_handle_slot(ctx);
  slot->object = lexe_v8::shim::ObjectLayout(
      const_cast<lexe_v8::shim::Map*>(&lexe_v8::shim::Map::object_map()), root_id);
  slot->object.function_id = function_id;
  slot->owns_root = false;
  return MaybeLocal<Function>(v8::MakeLocalFromObject<Function>(
      reinterpret_cast<Isolate*>(lexe_v8_current_isolate()), &slot->object));
}

}  // namespace v8
