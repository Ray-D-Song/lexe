#include "internal.h"

#include <utility>

#include <v8-internal.h>
#include <v8-local-handle.h>

namespace lexe_v8 {

static HandleScopeData* isolate_handle_scope_data(v8::Isolate* isolate) {
  return handle_scope_data(reinterpret_cast<IsolateImpl*>(isolate));
}

}  // namespace lexe_v8

namespace v8 {

void HandleScope::Initialize(Isolate* isolate) {
  auto* data = lexe_v8::isolate_handle_scope_data(isolate);
  i_isolate_ = reinterpret_cast<internal::Isolate*>(isolate);
  prev_next_ = reinterpret_cast<internal::Address*>(data->next);
  prev_limit_ = reinterpret_cast<internal::Address*>(data->limit);
  data->level++;
  if (auto* ctx = lexe_v8_current_context()) {
    auto* impl = lexe_v8::ctx_impl(ctx);
    impl->scopes.push_back(lexe_v8::HandleScopeFrame{impl->arena.watermark});
  }
}

HandleScope::HandleScope(Isolate* isolate) {
  Initialize(isolate);
}

HandleScope::~HandleScope() {
  if (!i_isolate_) {
    return;
  }
  auto* isolate = reinterpret_cast<Isolate*>(i_isolate_);
  auto* data = lexe_v8::isolate_handle_scope_data(isolate);
  std::swap(data->next, reinterpret_cast<uintptr_t*&>(prev_next_));
  data->level--;
  if (reinterpret_cast<uintptr_t*>(data->limit) != reinterpret_cast<uintptr_t*>(prev_limit_)) {
    data->limit = reinterpret_cast<uintptr_t*>(prev_limit_);
  }
  if (auto* ctx = lexe_v8_current_context()) {
    auto* impl = lexe_v8::ctx_impl(ctx);
    if (!impl->scopes.empty()) {
      lexe_v8::rewind_handle_arena(ctx, impl->scopes.back().watermark);
      impl->scopes.pop_back();
    }
  }
}

internal::Address* HandleScope::CreateHandle(internal::Isolate* i_isolate,
                                             internal::Address value) {
  (void)i_isolate;
  auto* ctx = lexe_v8_current_context();
  if (!ctx) {
    return nullptr;
  }
  const uintptr_t raw = static_cast<uintptr_t>(value);
  // Local<T>::New and ReturnValue::Get hand us the tagged first word, not the layout.
  auto* src =
      (raw & 0b11) == static_cast<uintptr_t>(lexe_v8::shim::TaggedPointer::Tag::StrongPointer)
          ? lexe_v8::shim::TaggedPointer::fromRaw(raw).getPtr<lexe_v8::shim::ObjectLayout>()
          : reinterpret_cast<lexe_v8::shim::ObjectLayout*>(raw);
  if (src == nullptr) {
    return nullptr;
  }
  auto* slot = lexe_v8::alloc_handle_slot(ctx);
  slot->object = *src;
  slot->object.tagged_map = lexe_v8::shim::TaggedPointer(&slot->object);
  slot->tagged_value = lexe_v8::shim::TaggedPointer(&slot->object);
  lexe_v8::note_materialized_layout(&slot->object);
  lexe_v8::register_handle_repr(ctx, reinterpret_cast<uintptr_t>(&slot->object),
                                  slot->object.contents.root_id);
  lexe_v8::register_handle_repr(ctx, static_cast<uintptr_t>(slot->object.tagged_map.value),
                                  slot->object.contents.root_id);
  if (lexe_v8::is_callback_frame_layout(src)) {
    lexe_v8::current_callback_frame().borrowed_layouts.push_back(&slot->object);
  }
  // Indirect Local::ptr() reads the first word of the in-place object layout.
  return reinterpret_cast<internal::Address*>(&slot->object);
}

}  // namespace v8
