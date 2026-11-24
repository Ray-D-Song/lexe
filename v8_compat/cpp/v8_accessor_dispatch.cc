#include "abi_137_generated.h"
#include "accessor_registry.h"
#include "internal.h"
#include "lexe_v8_bridge.h"

#include <v8-function-callback.h>

namespace lexe_v8 {

struct FakePropertyCallbackInfo {
  v8::internal::Address args_[LEXE_V8_PROPERTY_CALLBACK_K_ARGS_LENGTH];
};

static_assert(sizeof(FakePropertyCallbackInfo) ==
              sizeof(v8::PropertyCallbackInfo<v8::Value>));

static v8::internal::Address encode_smi(int32_t value) {
  return static_cast<v8::internal::Address>(
      (static_cast<uintptr_t>(value) << 32) |
      static_cast<uintptr_t>(shim::TaggedPointer::Tag::Smi));
}

static uint64_t root_id_from_accessor_return(LexeV8ContextState* ctx,
                                             uintptr_t repr) {
  // Same Smi / Set(0) handling as function callback return values.
  uint64_t root = resolve_return_value_repr(ctx, repr);
  if (root != 0) {
    return root;
  }
  auto* isolate = lexe_v8_current_isolate();
  if (!isolate) {
    return 0;
  }
  auto* impl = iso_impl(isolate);
  shim::ObjectLayout* layout = nullptr;
  if (repr == reinterpret_cast<uintptr_t>(&impl->true_value.layout) ||
      repr == reinterpret_cast<uintptr_t>(&impl->false_value.layout) ||
      repr == reinterpret_cast<uintptr_t>(&impl->undefined_value.layout) ||
      repr == reinterpret_cast<uintptr_t>(&impl->null_value.layout) ||
      repr == reinterpret_cast<uintptr_t>(&impl->the_hole_value.layout) ||
      repr == reinterpret_cast<uintptr_t>(&impl->empty_string.layout)) {
    layout = reinterpret_cast<shim::ObjectLayout*>(repr);
  } else if ((repr & 0b11) ==
             static_cast<uintptr_t>(shim::TaggedPointer::Tag::StrongPointer)) {
    layout = shim::TaggedPointer::fromRaw(repr).getPtr<shim::ObjectLayout>();
  }
  if (!layout) {
    return 0;
  }
  const int root_index = oddball_root_index(impl, layout);
  if (root_index < 0) {
    return 0;
  }
  uint64_t root_id = 0;
  if (lexe_v8_oddball_root(ctx, root_index, &root_id) != LEXE_V8_OK) {
    return 0;
  }
  return root_id;
}

static v8::internal::Address layout_slot(shim::ObjectLayout* layout) {
  return static_cast<v8::internal::Address>(shim::TaggedPointer(layout).value);
}

static shim::ObjectLayout* layout_for_root(LexeV8ContextState* ctx, uint64_t root_id) {
  auto* slot = alloc_handle_slot(ctx);
  slot->object =
      shim::ObjectLayout(const_cast<shim::Map*>(&shim::Map::object_map()), root_id);
  slot->object.tagged_map = shim::TaggedPointer(&slot->object);
  slot->tagged_value = shim::TaggedPointer(&slot->object);
  slot->owns_root = false;
  register_handle_repr(ctx, reinterpret_cast<uintptr_t>(&slot->object), root_id);
  register_handle_repr(ctx, static_cast<uintptr_t>(slot->object.tagged_map.value), root_id);
  return &slot->object;
}

LexeV8Status dispatch_v8_accessor(uint32_t accessor_id,
                                    uint64_t receiver_root,
                                    void* embedder_override,
                                    uint64_t* out_result_root) {
  auto* acc_rec = AccessorRegistry::instance().accessor_at(accessor_id);
  if (!acc_rec || !acc_rec->getter) {
    return LEXE_V8_ERROR;
  }

  auto* ctx = lexe_v8_current_context();
  auto* isolate = lexe_v8_current_isolate();
  if (!ctx || !isolate) {
    return LEXE_V8_ERROR;
  }

  g_callback_handle_stack.emplace_back();
  auto& callback_frame = current_callback_frame();

  shim::ObjectLayout* this_layout = layout_for_root(ctx, receiver_root);
  shim::ObjectLayout* holder_layout = layout_for_root(ctx, receiver_root);
  callback_frame.borrowed_layouts.push_back(this_layout);
  callback_frame.borrowed_layouts.push_back(holder_layout);
  shim::ObjectLayout data_layout(const_cast<shim::Map*>(&shim::Map::object_map()), acc_rec->data_root_id);
  auto& undefined_layout =
      iso_impl(reinterpret_cast<LexeV8IsolateState*>(isolate))->undefined_value.layout;

  void* embedder = embedder_override;
  if (!embedder) {
    lexe_v8_internal_field_get(ctx, receiver_root, 0, &embedder);
  }
  if (embedder != nullptr) {
    this_layout->embedder_slot_0 = embedder;
    holder_layout->embedder_slot_0 = embedder;
  }

  FakePropertyCallbackInfo frame{};
  frame.args_[LEXE_V8_PROPERTY_CALLBACK_K_SHOULD_THROW_ON_ERROR_INDEX] = encode_smi(0);
  frame.args_[LEXE_V8_PROPERTY_CALLBACK_K_HOLDER_INDEX] = layout_slot(holder_layout);
  frame.args_[LEXE_V8_PROPERTY_CALLBACK_K_ISOLATE_INDEX] =
      reinterpret_cast<v8::internal::Address>(isolate);
  frame.args_[LEXE_V8_PROPERTY_CALLBACK_K_HOLDER_V2_INDEX] = layout_slot(holder_layout);
  frame.args_[LEXE_V8_PROPERTY_CALLBACK_K_RETURN_VALUE_INDEX] =
      layout_slot(&undefined_layout);
  frame.args_[LEXE_V8_PROPERTY_CALLBACK_K_DATA_INDEX] = layout_slot(&data_layout);
  frame.args_[LEXE_V8_PROPERTY_CALLBACK_K_THIS_INDEX] = layout_slot(this_layout);

  auto& info = reinterpret_cast<v8::PropertyCallbackInfo<v8::Value>&>(frame);

  lexe_v8_open_handle_scope(ctx);
  acc_rec->getter(v8::Local<v8::Name>(), info);
  const uintptr_t repr = static_cast<uintptr_t>(
      frame.args_[LEXE_V8_PROPERTY_CALLBACK_K_RETURN_VALUE_INDEX]);
  *out_result_root = root_id_from_accessor_return(ctx, repr);
  lexe_v8_close_handle_scope(ctx);
  lexe_v8_dispatch_pending_weak_callbacks();
  g_callback_handle_stack.pop_back();
  if (g_callback_handle_stack.empty()) {
    g_callback_handle_frame = {};
  }
  return LEXE_V8_OK;
}

}  // namespace lexe_v8

extern "C" LexeV8Status lexe_v8_dispatch_accessor(uint32_t accessor_id,
                                                      uint64_t receiver_root,
                                                      void* embedder_override,
                                                      uint64_t* out_result_root) {
  return lexe_v8::dispatch_v8_accessor(accessor_id, receiver_root, embedder_override,
                                        out_result_root);
}
