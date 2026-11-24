#pragma once

#include "internal.h"
#include "lexe_v8_bridge.h"
#include "v8_local_helpers.h"

#include <cstddef>
#include <v8.h>

namespace lexe_v8 {

inline LexeV8ContextState* bridge_ctx() {
  return lexe_v8_current_context();
}

inline const LexeV8BridgeV1* bridge() {
  return lexe_v8_bridge();
}

inline shim::ObjectLayout* layout_from_slot(v8::internal::Address* slot) {
  if (!slot) {
    return nullptr;
  }
  const uintptr_t cell_word = static_cast<uintptr_t>(*slot);
  if (cell_word != 0 &&
      (cell_word & 0b11) == static_cast<uintptr_t>(shim::TaggedPointer::Tag::StrongPointer)) {
    if (auto* from_tag = shim::TaggedPointer::fromRaw(cell_word).getPtr<shim::ObjectLayout>()) {
      return from_tag;
    }
  }
  auto scan_frame = [&](const CallbackHandleFrame& frame) -> shim::ObjectLayout* {
    if (!frame.layouts.empty()) {
      const uintptr_t slot_addr = reinterpret_cast<uintptr_t>(slot);
      for (size_t i = 0; i < frame.layouts.size(); ++i) {
        auto* layout = const_cast<shim::ObjectLayout*>(&frame.layouts[i]);
        const uintptr_t begin = reinterpret_cast<uintptr_t>(layout);
        const uintptr_t end = begin + sizeof(*layout);
        if (slot_addr >= begin && slot_addr < end) {
          return layout;
        }
      }
    }
    if (!frame.values.empty()) {
      const auto* begin = frame.values.data();
      const auto* end = begin + frame.values.size();
      if (slot >= begin && slot < end) {
        const size_t index = static_cast<size_t>(slot - begin);
        if (index < frame.layouts.size()) {
          return const_cast<shim::ObjectLayout*>(&frame.layouts[index]);
        }
      }
    }
    return nullptr;
  };
  for (auto it = g_callback_handle_stack.rbegin(); it != g_callback_handle_stack.rend(); ++it) {
    if (auto* layout = scan_frame(*it)) {
      return layout;
    }
  }
  if (auto* layout = scan_frame(g_callback_handle_frame)) {
    return layout;
  }
  if (cell_word != 0 &&
      (cell_word & 0b11) == static_cast<uintptr_t>(shim::TaggedPointer::Tag::StrongPointer)) {
    return shim::TaggedPointer::fromRaw(cell_word).getPtr<shim::ObjectLayout>();
  }
  return reinterpret_cast<shim::ObjectLayout*>(cell_word);
}

template <typename T>
inline shim::ObjectLayout* layout_from_local(v8::Local<T> handle) {
  return layout_from_slot(reinterpret_cast<v8::internal::Address*>(handle.operator->()));
}

inline uint64_t materialize_oddball_root(shim::ObjectLayout* layout) {
  if (!layout || layout->contents.root_id != 0) {
    return layout ? layout->contents.root_id : 0;
  }
  auto* isolate = v8::Isolate::GetCurrent();
  auto* ctx = bridge_ctx();
  if (!isolate || !ctx) {
    return 0;
  }
  auto* impl = iso_impl(reinterpret_cast<LexeV8IsolateState*>(isolate));
  const int root_index = oddball_root_index(impl, layout);
  if (root_index < 0) {
    return 0;
  }
  uint64_t root_id = 0;
  if (lexe_v8_oddball_root(ctx, root_index, &root_id) != LEXE_V8_OK) {
    return 0;
  }
  layout->contents.root_id = root_id;
  return root_id;
}

inline uint64_t root_from_local(v8::Local<v8::Value> local) {
  auto* layout = layout_from_local(local);
  if (!layout) {
    return 0;
  }
  if (layout->contents.root_id != 0) {
    return layout->contents.root_id;
  }
  if (auto* ctx = bridge_ctx()) {
    if (uint64_t root_id = resolve_root_from_repr(ctx, reinterpret_cast<uintptr_t>(layout))) {
      layout->contents.root_id = root_id;
      return root_id;
    }
  }
  return materialize_oddball_root(layout);
}

inline uint64_t root_from_object(const void* self) {
  auto* slot = reinterpret_cast<v8::internal::Address*>(const_cast<void*>(self));
  auto* layout = layout_from_slot(slot);
  if (layout) {
    if (layout->contents.root_id != 0) {
      return layout->contents.root_id;
    }
    if (layout->tagged_map.tag() == shim::TaggedPointer::Tag::StrongPointer) {
      auto* target = layout->tagged_map.getPtr<shim::ObjectLayout>();
      if (target != nullptr && target != layout && target->contents.root_id != 0) {
        return target->contents.root_id;
      }
    }
    const uint64_t oddball = materialize_oddball_root(layout);
    if (oddball != 0) {
      return oddball;
    }
  }
  auto* ctx = bridge_ctx();
  if (!ctx) {
    return 0;
  }
  uint64_t root_id = 0;
  if (lexe_v8_root_id_for_js_object(ctx, const_cast<void*>(self), &root_id) == LEXE_V8_OK) {
    return root_id;
  }
  return 0;
}

template <typename T>
inline v8::Local<T> local_from_root(v8::Isolate* isolate, uint64_t root_id, const shim::Map* map) {
  auto* ctx = bridge_ctx();
  auto* slot = alloc_handle_slot(ctx);
  slot->object = shim::ObjectLayout(map, root_id);
  slot->tagged_value = shim::TaggedPointer(&slot->object);
  slot->owns_root = false;
  note_materialized_layout(&slot->object);
  return v8::MakeLocalFromObject<T>(isolate, &slot->object);
}

inline v8::Local<v8::Value> local_value_from_root(v8::Isolate* isolate, uint64_t root_id) {
  return local_from_root<v8::Value>(isolate, root_id, &shim::Map::object_map());
}

inline bool bridge_ok(LexeV8Status status) {
  return status == LEXE_V8_OK;
}

}  // namespace lexe_v8
