#include "internal.h"
#include "v8_local_helpers.h"

#include <node.h>
#include <v8-context.h>
#include <v8-local-handle.h>
#include <v8-object.h>

namespace {

// NODE_MODULE_INIT() defines a 3-arg function; NODE_MODULE_CONTEXT_AWARE_X in
// Node headers intentionally casts it to 4-arg addon_context_register_func.
// The fourth priv argument is ignored by those bodies — Node's own ABI.
// Suppress only Clang's function-pointer type check at this erased call site.
#if defined(__clang__)
__attribute__((no_sanitize("function")))
#endif
void invoke_erased_context_register_func(
    node::addon_context_register_func callback,
    v8::Local<v8::Object> exports,
    v8::Local<v8::Value> module,
    v8::Local<v8::Context> context,
    void* priv) {
  callback(exports, module, context, priv);
}

}  // namespace

extern "C" LexeV8Status lexe_v8_run_module_init(
    LexeV8ContextState* ctx,
    void* module_opaque,
    uint64_t exports_root_id,
    uint64_t module_root_id,
    uint64_t* out_exports_root_id) {
  const LexeV8BridgeV1* bridge = lexe_v8_bridge();
  if (!bridge || !ctx || !module_opaque || !out_exports_root_id) {
    return LEXE_V8_ERROR;
  }

  auto* node_mod = reinterpret_cast<node::node_module*>(module_opaque);
  lexe_v8_set_current_context(ctx);
  lexe_v8_open_handle_scope(ctx);

  v8::Isolate* isolate = reinterpret_cast<v8::Isolate*>(lexe_v8_current_isolate());
  v8::HandleScope scope(isolate);

  auto* exports_slot = lexe_v8::alloc_handle_slot(ctx);
  exports_slot->object = lexe_v8::shim::ObjectLayout(
      const_cast<lexe_v8::shim::Map*>(&lexe_v8::shim::Map::object_map()), exports_root_id);
  auto* module_slot = lexe_v8::alloc_handle_slot(ctx);
  module_slot->object = lexe_v8::shim::ObjectLayout(
      const_cast<lexe_v8::shim::Map*>(&lexe_v8::shim::Map::object_map()), module_root_id);
  auto* context_slot = lexe_v8::alloc_handle_slot(ctx);
  context_slot->object = lexe_v8::shim::ObjectLayout(
      const_cast<lexe_v8::shim::Map*>(&lexe_v8::shim::Map::object_map()),
      lexe_v8::ctx_impl(ctx)->context_root_id);

  v8::Local<v8::Object> exports =
      v8::MakeLocalFromObject<v8::Object>(isolate, &exports_slot->object);
  v8::Local<v8::Value> module_obj =
      v8::MakeLocalFromObject<v8::Value>(isolate, &module_slot->object);
  v8::Local<v8::Context> context =
      v8::MakeLocalFromObject<v8::Context>(isolate, &context_slot->object);

  if (node_mod->nm_context_register_func) {
    invoke_erased_context_register_func(
        node_mod->nm_context_register_func,
        exports,
        module_obj,
        context,
        node_mod->nm_priv);
  } else if (node_mod->nm_register_func) {
    node_mod->nm_register_func(exports, module_obj, nullptr);
  } else {
    lexe_v8_close_handle_scope(ctx);
    return LEXE_V8_ERROR;
  }

  *out_exports_root_id = exports_slot->object.contents.root_id;
  lexe_v8_close_handle_scope(ctx);
  lexe_v8_dispatch_pending_weak_callbacks();
  return LEXE_V8_OK;
}
