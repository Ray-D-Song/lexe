#include "abi_137_generated.h"
#include "function_registry.h"
#include "lexe_v8_bridge.h"
#include "template_registry.h"
#include "internal.h"

#include <cstdio>

#include <v8-function-callback.h>
#include <v8-internal.h>
#include <v8-local-handle.h>

namespace lexe_v8 {

thread_local uint32_t g_active_callback_template_id = 0;

struct FakeFunctionCallbackInfo {
  v8::internal::Address* implicit_args_;
  v8::internal::Address* values_;
  v8::internal::Address length_;
};

static_assert(sizeof(FakeFunctionCallbackInfo) == sizeof(v8::FunctionCallbackInfo<v8::Value>));

static const shim::Map* map_for_layout_kind(uint8_t kind) {
  switch (kind) {
    case LEXE_V8_LAYOUT_STRING:
      return &shim::Map::string_map();
    case LEXE_V8_LAYOUT_ODDBALL:
      return &shim::Map::oddball_map();
    case LEXE_V8_LAYOUT_HEAP_NUMBER:
      return &shim::Map::heap_number_map();
    case LEXE_V8_LAYOUT_OBJECT:
    default:
      return &shim::Map::object_map();
  }
}

static shim::ObjectLayout layout_for_value_root(LexeV8ContextState* ctx, uint64_t root_id) {
  uint8_t kind = LEXE_V8_LAYOUT_OBJECT;
  int32_t smi = 0;
  if (lexe_v8_value_layout_kind(ctx, root_id, &kind, &smi) != LEXE_V8_OK) {
    return shim::ObjectLayout(const_cast<shim::Map*>(&shim::Map::object_map()), root_id);
  }
  if (kind == LEXE_V8_LAYOUT_INT32_SMI) {
    shim::ObjectLayout layout;
    layout.tagged_map = shim::TaggedPointer(smi);
    layout.contents.root_id = root_id;
    return layout;
  }
  return shim::ObjectLayout(const_cast<shim::Map*>(map_for_layout_kind(kind)), root_id);
}

static shim::ObjectLayout* layout_for_root(LexeV8ContextState* ctx, uint64_t root_id) {
  auto* slot = alloc_handle_slot(ctx);
  slot->object = layout_for_value_root(ctx, root_id);
  slot->object.tagged_map = shim::TaggedPointer(&slot->object);
  slot->tagged_value = shim::TaggedPointer(&slot->object);
  slot->owns_root = false;
  register_handle_repr(ctx, reinterpret_cast<uintptr_t>(&slot->object), root_id);
  register_handle_repr(ctx, static_cast<uintptr_t>(slot->object.tagged_map.value), root_id);
  return &slot->object;
}

static uint64_t root_id_from_return_value(LexeV8ContextState* ctx,
                                          v8::internal::Address repr) {
  return resolve_return_value_repr(ctx, static_cast<uintptr_t>(repr));
}

LexeV8Status dispatch_v8_callback(uint32_t function_id,
                                    uint64_t receiver_root,
                                    uint64_t new_target_root,
                                    const uint64_t* arg_roots,
                                    int argc,
                                    uint64_t* out_result_root) {
  auto* fn_rec = FunctionRegistry::instance().function_at(function_id);
  if (!fn_rec) {
    return LEXE_V8_ERROR;
  }
  auto* tmpl_rec = FunctionRegistry::instance().template_at(fn_rec->template_id);
  if (!tmpl_rec || !tmpl_rec->callback) {
    return LEXE_V8_ERROR;
  }

  auto* ctx = lexe_v8_current_context();
  auto* isolate = lexe_v8_current_isolate();
  if (!ctx || !isolate) {
    return LEXE_V8_ERROR;
  }

  g_callback_handle_stack.emplace_back();
  auto& frame = current_callback_frame();

  auto store_layout = [&](uint64_t root_id, shim::ObjectLayout layout) -> v8::internal::Address {
    void* embedder = nullptr;
    if (lexe_v8_internal_field_get(ctx, root_id, 0, &embedder) != LEXE_V8_OK) {
      embedder = nullptr;
    }
    if (embedder != nullptr) {
      layout.embedder_slot_0 = embedder;
    }
    frame.layouts.push_back(std::move(layout));
    auto& stored = frame.layouts.back();
    frame.roots.push_back(root_id);
    return static_cast<v8::internal::Address>(shim::TaggedPointer(&stored).value);
  };

  frame.layouts.clear();
  frame.values.clear();
  frame.roots.clear();
  frame.borrowed_layouts.clear();
  frame.layouts.reserve(static_cast<size_t>(argc) + 1);
  frame.values.reserve(static_cast<size_t>(argc) + 2);
  frame.roots.reserve(static_cast<size_t>(argc) + 1);

  // Node 24 FunctionCallbackInfo: values_[-1] is the receiver, values_[0..] are args.
  frame.values.push_back(
      store_layout(receiver_root,
                   shim::ObjectLayout(const_cast<shim::Map*>(&shim::Map::object_map()),
                                      receiver_root)));
  for (int i = 0; i < argc; ++i) {
    frame.values.push_back(store_layout(arg_roots[i], layout_for_value_root(ctx, arg_roots[i])));
  }

  auto* context_layout = layout_for_root(ctx, ctx_impl(ctx)->context_root_id);
  frame.borrowed_layouts.push_back(context_layout);
  auto& undefined_layout =
      iso_impl(reinterpret_cast<LexeV8IsolateState*>(isolate))->undefined_value.layout;

  shim::ObjectLayout target_layout(
      const_cast<shim::Map*>(&shim::Map::object_map()), fn_rec->template_id);

  v8::internal::Address implicit_args[LEXE_V8_FUNCTION_CALLBACK_K_ARGS_LENGTH] = {};
  implicit_args[LEXE_V8_FUNCTION_CALLBACK_K_UNUSED_INDEX] = 0;
  implicit_args[LEXE_V8_FUNCTION_CALLBACK_K_ISOLATE_INDEX] =
      reinterpret_cast<v8::internal::Address>(isolate);
  implicit_args[LEXE_V8_FUNCTION_CALLBACK_K_CONTEXT_INDEX] =
      reinterpret_cast<v8::internal::Address>(context_layout);
  implicit_args[LEXE_V8_FUNCTION_CALLBACK_K_RETURN_VALUE_INDEX] =
      static_cast<v8::internal::Address>(shim::TaggedPointer(&undefined_layout).value);
  implicit_args[LEXE_V8_FUNCTION_CALLBACK_K_TARGET_INDEX] =
      reinterpret_cast<v8::internal::Address>(&target_layout);
  if (new_target_root != 0) {
    auto* new_target_layout = layout_for_root(ctx, new_target_root);
    frame.borrowed_layouts.push_back(new_target_layout);
    implicit_args[LEXE_V8_FUNCTION_CALLBACK_K_NEW_TARGET_INDEX] =
        reinterpret_cast<v8::internal::Address>(new_target_layout);
  } else {
    implicit_args[LEXE_V8_FUNCTION_CALLBACK_K_NEW_TARGET_INDEX] =
        static_cast<v8::internal::Address>(shim::TaggedPointer(&undefined_layout).value);
  }

  v8::internal::Address* values = frame.values.data() + 1;

  FakeFunctionCallbackInfo fake{implicit_args, values,
                                static_cast<v8::internal::Address>(argc)};
  auto& info = reinterpret_cast<v8::FunctionCallbackInfo<v8::Value>&>(fake);
  lexe_v8_open_handle_scope(ctx);
  g_active_callback_template_id = fn_rec->template_id;
  tmpl_rec->callback(info);
  g_active_callback_template_id = 0;
  *out_result_root = root_id_from_return_value(
      ctx, implicit_args[LEXE_V8_FUNCTION_CALLBACK_K_RETURN_VALUE_INDEX]);
  lexe_v8_close_handle_scope(ctx);
  lexe_v8_dispatch_pending_weak_callbacks();
  g_callback_handle_stack.pop_back();
  if (g_callback_handle_stack.empty()) {
    g_callback_handle_frame = {};
  }
  return LEXE_V8_OK;
}

}  // namespace lexe_v8

extern "C" LexeV8Status lexe_v8_dispatch_callback(uint32_t function_id,
                                                      uint64_t receiver_root,
                                                      uint64_t new_target_root,
                                                      const uint64_t* arg_roots,
                                                      int argc,
                                                      uint64_t* out_result_root) {
  return lexe_v8::dispatch_v8_callback(function_id, receiver_root, new_target_root, arg_roots,
                                         argc, out_result_root);
}
