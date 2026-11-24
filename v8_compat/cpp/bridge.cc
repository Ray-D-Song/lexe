#include "internal.h"

#include <cstdio>

namespace {

const LexeV8BridgeV1* g_bridge = nullptr;
LexeV8OddballRootFn g_oddball_root_fn = nullptr;
thread_local LexeV8ContextState* g_current_ctx = nullptr;
thread_local LexeV8IsolateState* g_current_isolate = nullptr;

}  // namespace

extern "C" void lexe_v8_bind_bridge(const LexeV8BridgeV1* bridge) {
  g_bridge = bridge;
}

extern "C" void lexe_v8_set_oddball_root_fn(LexeV8OddballRootFn fn) {
  g_oddball_root_fn = fn;
}

extern "C" const LexeV8BridgeV1* lexe_v8_bridge(void) {
  return g_bridge;
}

extern "C" LexeV8ContextState* lexe_v8_current_context(void) {
  return g_current_ctx;
}

extern "C" LexeV8IsolateState* lexe_v8_current_isolate(void) {
  return g_current_isolate;
}

extern "C" void lexe_v8_set_current_context(LexeV8ContextState* ctx) {
  g_current_ctx = ctx;
}

extern "C" void lexe_v8_set_current_isolate(LexeV8IsolateState* isolate) {
  g_current_isolate = isolate;
}

extern "C" LexeV8IsolateState* lexe_v8_create_isolate(void) {
  auto* isolate = new lexe_v8::IsolateImpl();
  lexe_v8::init_isolate_roots(isolate);
  return reinterpret_cast<LexeV8IsolateState*>(isolate);
}

extern "C" void lexe_v8_destroy_isolate(LexeV8IsolateState* isolate) {
  if (g_current_isolate == isolate) {
    g_current_isolate = nullptr;
  }
  lexe_v8::dispose_isolate_persistents(lexe_v8::iso_impl(isolate));
  delete lexe_v8::iso_impl(isolate);
}

extern "C" LexeV8ContextState* lexe_v8_create_context(void) {
  return reinterpret_cast<LexeV8ContextState*>(new lexe_v8::ContextImpl());
}

extern "C" void lexe_v8_destroy_context(LexeV8ContextState* ctx) {
  if (g_current_ctx == ctx) {
    g_current_ctx = nullptr;
  }
  delete lexe_v8::ctx_impl(ctx);
}

extern "C" void lexe_v8_open_handle_scope(LexeV8ContextState* ctx) {
  auto* impl = lexe_v8::ctx_impl(ctx);
  impl->scopes.push_back(lexe_v8::HandleScopeFrame{impl->arena.watermark});
}

extern "C" void lexe_v8_close_handle_scope(LexeV8ContextState* ctx) {
  auto* impl = lexe_v8::ctx_impl(ctx);
  if (impl->scopes.empty()) {
    return;
  }
  lexe_v8::rewind_handle_arena(ctx, impl->scopes.back().watermark);
  impl->scopes.pop_back();
}

extern "C" void lexe_v8_set_context_root_id(LexeV8ContextState* ctx, uint64_t root_id) {
  lexe_v8::ctx_impl(ctx)->context_root_id = root_id;
}

extern "C" void lexe_v8_set_context_quickjs_key(LexeV8ContextState* ctx, uintptr_t key) {
  lexe_v8::ctx_impl(ctx)->quickjs_context_key = key;
}

extern "C" uint64_t lexe_v8_context_root_id(LexeV8ContextState* ctx) {
  return lexe_v8::ctx_impl(ctx)->context_root_id;
}

extern "C" LexeV8Status lexe_v8_oddball_root(LexeV8ContextState* ctx,
                                                 int root_index,
                                                 uint64_t* out_root_id) {
  if (!ctx || !out_root_id || root_index < 0 ||
      static_cast<size_t>(root_index) >= lexe_v8::abi137::kRootSlotCount) {
    return LEXE_V8_ERROR;
  }
  auto* impl = lexe_v8::ctx_impl(ctx);
  if (impl->oddball_roots[root_index] != 0) {
    *out_root_id = impl->oddball_roots[root_index];
    return LEXE_V8_OK;
  }
  if (!g_oddball_root_fn) {
    return LEXE_V8_UNSUPPORTED;
  }
  uint64_t root_id = 0;
  LexeV8Status status = g_oddball_root_fn(ctx, root_index, &root_id);
  if (status != LEXE_V8_OK) {
    return status;
  }
  impl->oddball_roots[root_index] = root_id;
  *out_root_id = root_id;
  return LEXE_V8_OK;
}

extern "C" size_t lexe_v8_pending_modules_count(void) {
  return lexe_v8::pending_v8_modules_count();
}

extern "C" void* lexe_v8_take_pending_module(size_t index) {
  return lexe_v8::take_pending_v8_module(index);
}
