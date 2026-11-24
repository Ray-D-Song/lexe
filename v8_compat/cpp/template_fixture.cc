#include "function_registry.h"
#include "lexe_v8_bridge.h"
#include "template_registry.h"
#include "v8_bridge_helpers.h"

#include <cstdint>

#include <node_object_wrap.h>
#include <v8.h>

extern "C" LexeV8Status lexe_v8_object_reserve_internal_fields(
    LexeV8ContextState* ctx,
    uint64_t object_root,
    int count);

namespace {

static void noop_callback(const v8::FunctionCallbackInfo<v8::Value>& info) {
  (void)info;
}

struct ObjectWrapShutdownCounters {
  int constructed = 0;
  int destroyed = 0;
};

// Real node::ObjectWrap so teardown exercises WeakCallback + Persistent::Reset
// (not a synthetic weak callback).
class ShutdownObjectWrap final : public node::ObjectWrap {
 public:
  explicit ShutdownObjectWrap(ObjectWrapShutdownCounters* counters) : counters_(counters) {
    ++counters_->constructed;
  }

  ~ShutdownObjectWrap() override {
    ++counters_->destroyed;
  }

  void Attach(v8::Local<v8::Object> object) {
    Wrap(object);
  }

 private:
  ObjectWrapShutdownCounters* counters_;
};

ObjectWrapShutdownCounters* g_ctor_counters = nullptr;

void objectwrap_ctor_callback(const v8::FunctionCallbackInfo<v8::Value>& info) {
  if (g_ctor_counters == nullptr) {
    return;
  }
  // Mirror better-sqlite3: a native ctor materializes locals before Wrap, so
  // g_last_materialized_layout points at an unrelated object by then.
  v8::Local<v8::Object> decoy = v8::Object::New(info.GetIsolate());
  (void)decoy;
  // Local<T>::New(info.This()) copies the frame layout into the arena; the copy
  // must stay marked borrowed so globalizing it dups the receiver root.
  v8::Local<v8::Object> copied = v8::Local<v8::Object>::New(info.GetIsolate(), info.This());
  auto* wrap = new ShutdownObjectWrap(g_ctor_counters);
  wrap->Attach(copied);
  info.GetReturnValue().Set(info.This());
}

}  // namespace

// Counters live outside the wrap so they survive delete.
extern "C" ObjectWrapShutdownCounters* lexe_v8_test_objectwrap_shutdown_counters_new() {
  return new ObjectWrapShutdownCounters();
}

extern "C" void lexe_v8_test_objectwrap_shutdown_counters_read(
    const ObjectWrapShutdownCounters* counters,
    int* constructed_out,
    int* destroyed_out) {
  if (!counters) {
    return;
  }
  if (constructed_out) {
    *constructed_out = counters->constructed;
  }
  if (destroyed_out) {
    *destroyed_out = counters->destroyed;
  }
}

extern "C" void lexe_v8_test_objectwrap_shutdown_counters_destroy(
    ObjectWrapShutdownCounters* counters) {
  delete counters;
}

// Create + Wrap only. Do not manually release roots or invoke weak callbacks —
// production teardown must drive ObjectWrap::WeakCallback → delete → Reset.
extern "C" int lexe_v8_test_setup_shutdown_object_wrap(
    LexeV8ContextState* ctx_state,
    ObjectWrapShutdownCounters* counters) {
  const LexeV8BridgeV1* bridge = lexe_v8_bridge();
  auto* isolate = reinterpret_cast<v8::Isolate*>(lexe_v8_current_isolate());
  if (!bridge || !ctx_state || !counters || !isolate || !bridge->object_new) {
    return 0;
  }

  v8::HandleScope scope(isolate);

  uint64_t root_id = 0;
  if (bridge->object_new(ctx_state, &root_id) != LEXE_V8_OK || root_id == 0) {
    return 0;
  }
  if (lexe_v8_object_reserve_internal_fields(ctx_state, root_id, 1) != LEXE_V8_OK) {
    if (bridge->root_drop) {
      bridge->root_drop(root_id);
    }
    return 0;
  }

  v8::Local<v8::Object> object = lexe_v8::local_from_root<v8::Object>(
      isolate, root_id, &lexe_v8::shim::Map::object_map());
  if (object.IsEmpty()) {
    if (bridge->root_drop) {
      bridge->root_drop(root_id);
    }
    return 0;
  }

  auto* wrap = new ShutdownObjectWrap(counters);
  wrap->Attach(object);
  // Local leaves scope; ObjectWrap weak persistent is the remaining owner.
  // MakeWeak already dropped the bridge strong root.
  return 1;
}

extern "C" uint32_t lexe_v8_test_register_function_template() {
  uint32_t template_id =
      lexe_v8::FunctionRegistry::instance().register_template(noop_callback, 0);
  return lexe_v8::FunctionRegistry::instance().register_function(template_id);
}

extern "C" uint32_t lexe_v8_test_register_objectwrap_ctor_template(
    ObjectWrapShutdownCounters* counters) {
  g_ctor_counters = counters;
  uint32_t template_id =
      lexe_v8::FunctionRegistry::instance().register_template(objectwrap_ctor_callback, 0);
  if (auto* fn = lexe_v8::TemplateRegistry::instance().function_template_at(template_id)) {
    if (auto* inst = lexe_v8::TemplateRegistry::instance().object_template_at(
            fn->instance_template_id)) {
      inst->internal_field_count = 1;
    }
  }
  return lexe_v8::FunctionRegistry::instance().register_function(template_id);
}

extern "C" void lexe_v8_test_register_template_property_with_missing_key(uint32_t function_id) {
  auto* fn = lexe_v8::FunctionRegistry::instance().function_at(function_id);
  if (!fn) {
    return;
  }
  auto* fn_rec =
      lexe_v8::TemplateRegistry::instance().function_template_at(fn->template_id);
  if (!fn_rec) {
    return;
  }
  auto* rec = lexe_v8::TemplateRegistry::instance().object_template_at(
      fn_rec->prototype_template_id);
  if (!rec) {
    return;
  }
  uint32_t child_template_id =
      lexe_v8::FunctionRegistry::instance().register_template(noop_callback, 0);
  rec->properties.emplace_back(
      UINT64_C(0xDEADBEEF),
      static_cast<uint64_t>(child_template_id),
      v8::None);
}

extern "C" LexeV8Status lexe_v8_test_function_template_get_function(
    LexeV8ContextState* ctx_state,
    uint32_t function_id,
    uint64_t* out_root) {
  const LexeV8BridgeV1* bridge = lexe_v8_bridge();
  if (!bridge || !ctx_state || !out_root || !bridge->function_template_get_function) {
    return LEXE_V8_ERROR;
  }
  return bridge->function_template_get_function(ctx_state, function_id, out_root);
}

extern "C" LexeV8Status lexe_v8_test_function_root_for_id(
    LexeV8ContextState* ctx_state,
    uint32_t function_id,
    uint64_t* out_root) {
  return lexe_v8_function_root_for_id(ctx_state, function_id, out_root);
}
