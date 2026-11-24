#include "internal.h"
#include "lexe_v8_bridge.h"
#include "v8_bridge_helpers.h"

#include <atomic>
#include <cstdint>

#include <v8-persistent-handle.h>
#include <v8-weak-callback-info.h>

extern "C" LexeV8Status lexe_v8_value_to_float64(LexeV8ContextState* ctx,
                                                     uint64_t root,
                                                     double* out);

namespace {

struct NativeWrap {
  std::atomic<int>* destructor_count;
  ~NativeWrap() {
    if (destructor_count) {
      destructor_count->fetch_add(1, std::memory_order_relaxed);
    }
  }
};

struct FixtureCounters;

struct WeakClosure {
  FixtureCounters* owner;
  std::atomic<int>* weak_callback_count;
  v8::Persistent<v8::Object>* persistent;
  NativeWrap native;
};

struct FixtureCounters {
  std::atomic<int> cleanup_count{0};
  std::atomic<int> weak_callback_count{0};
  std::atomic<int> destructor_count{0};
  v8::Persistent<v8::Object>* strong_persistent = nullptr;
  WeakClosure* weak_closure = nullptr;
  uint64_t strong_root_id = 0;
  uint64_t weak_root_id = 0;
};

static void fixture_weak_callback(const v8::WeakCallbackInfo<void>& data) {
  auto* closure = static_cast<WeakClosure*>(data.GetParameter());
  if (!closure) {
    return;
  }
  if (closure->weak_callback_count) {
    closure->weak_callback_count->fetch_add(1, std::memory_order_relaxed);
  }
  if (closure->persistent != nullptr) {
    closure->persistent->Reset();
    delete closure->persistent;
    closure->persistent = nullptr;
  }
  if (closure->owner != nullptr) {
    closure->owner->weak_closure = nullptr;
  }
  delete closure;
}

}  // namespace

extern "C" void lexe_v8_test_objectwrap_fixture_cleanup_hook(void* arg) {
  auto* counters = static_cast<FixtureCounters*>(arg);
  counters->cleanup_count.fetch_add(1, std::memory_order_relaxed);
  if (counters->strong_persistent != nullptr && !counters->strong_persistent->IsEmpty()) {
    counters->strong_persistent->Reset();
    delete counters->strong_persistent;
    counters->strong_persistent = nullptr;
  }
}

extern "C" FixtureCounters* lexe_v8_test_objectwrap_fixture_setup(
    LexeV8ContextState* ctx_state) {
  const LexeV8BridgeV1* bridge = lexe_v8_bridge();
  auto* isolate = reinterpret_cast<v8::Isolate*>(lexe_v8_current_isolate());
  if (!bridge || !ctx_state || !isolate) {
    return nullptr;
  }

  auto* counters = new FixtureCounters();
  auto* closure = new WeakClosure{
      counters,
      &counters->weak_callback_count,
      nullptr,
      NativeWrap{&counters->destructor_count},
  };
  counters->weak_closure = closure;

  if (bridge->object_new(ctx_state, &counters->strong_root_id) != LEXE_V8_OK ||
      counters->strong_root_id == 0) {
    delete closure;
    delete counters;
    return nullptr;
  }
  v8::Local<v8::Object> strong_object = lexe_v8::local_from_root<v8::Object>(
      isolate, counters->strong_root_id, &lexe_v8::shim::Map::object_map());
  counters->strong_persistent = new v8::Persistent<v8::Object>();
  counters->strong_persistent->Reset(isolate, strong_object);

  if (bridge->object_new(ctx_state, &counters->weak_root_id) != LEXE_V8_OK ||
      counters->weak_root_id == 0) {
    counters->strong_persistent->Reset();
    delete counters->strong_persistent;
    delete closure;
    delete counters;
    return nullptr;
  }
  v8::Local<v8::Object> weak_object = lexe_v8::local_from_root<v8::Object>(
      isolate, counters->weak_root_id, &lexe_v8::shim::Map::object_map());
  closure->persistent = new v8::Persistent<v8::Object>();
  closure->persistent->Reset(isolate, weak_object);
  closure->persistent->SetWeak(static_cast<void*>(closure),
                               fixture_weak_callback,
                               v8::WeakCallbackType::kParameter);

  return counters;
}

extern "C" void lexe_v8_test_objectwrap_fixture_release_bridge_roots(
    LexeV8ContextState* ctx_state,
    FixtureCounters* counters) {
  const LexeV8BridgeV1* bridge = lexe_v8_bridge();
  if (!bridge || !ctx_state || !counters) {
    return;
  }
  if (counters->strong_root_id != 0 && bridge->root_drop) {
    bridge->root_drop(counters->strong_root_id);
    counters->strong_root_id = 0;
  }
  if (counters->weak_root_id != 0 && bridge->root_drop) {
    bridge->root_drop(counters->weak_root_id);
    counters->weak_root_id = 0;
  }
}

extern "C" void lexe_v8_test_objectwrap_fixture_read_counts(const FixtureCounters* counters,
                                                              int* cleanup_out,
                                                              int* weak_out,
                                                              int* destructor_out) {
  if (!counters) {
    return;
  }
  if (cleanup_out) {
    *cleanup_out = counters->cleanup_count.load(std::memory_order_relaxed);
  }
  if (weak_out) {
    *weak_out = counters->weak_callback_count.load(std::memory_order_relaxed);
  }
  if (destructor_out) {
    *destructor_out = counters->destructor_count.load(std::memory_order_relaxed);
  }
}

extern "C" void lexe_v8_test_objectwrap_fixture_destroy(FixtureCounters* counters) {
  if (!counters) {
    return;
  }
  if (counters->strong_persistent != nullptr) {
    counters->strong_persistent->Reset();
    delete counters->strong_persistent;
  }
  if (counters->weak_closure != nullptr) {
    if (counters->weak_closure->persistent != nullptr) {
      counters->weak_closure->persistent->Reset();
      delete counters->weak_closure->persistent;
    }
    delete counters->weak_closure;
  }
  delete counters;
}

extern "C" int lexe_v8_test_persistent_binds_to_requested_local(
    LexeV8ContextState* ctx_state) {
  const LexeV8BridgeV1* bridge = lexe_v8_bridge();
  auto* isolate = reinterpret_cast<v8::Isolate*>(lexe_v8_current_isolate());
  auto* isolate_impl = lexe_v8::iso_impl(
      reinterpret_cast<LexeV8IsolateState*>(lexe_v8_current_isolate()));
  if (!bridge || !ctx_state || !isolate || !isolate_impl) {
    return 0;
  }
  uint64_t first_root = 0;
  uint64_t second_root = 0;
  if (bridge->object_new(ctx_state, &first_root) != LEXE_V8_OK || first_root == 0 ||
      bridge->object_new(ctx_state, &second_root) != LEXE_V8_OK || second_root == 0) {
    return 0;
  }
  v8::Local<v8::Object> first = lexe_v8::local_from_root<v8::Object>(
      isolate, first_root, &lexe_v8::shim::Map::object_map());
  // Materialize second last so g_last_materialized_layout points at the wrong object.
  v8::Local<v8::Object> second = lexe_v8::local_from_root<v8::Object>(
      isolate, second_root, &lexe_v8::shim::Map::object_map());
  (void)second;

  v8::Persistent<v8::Object> persistent;
  persistent.Reset(isolate, first);

  bool bound_to_first = false;
  for (const auto& [cell, slot] : isolate_impl->persistents) {
    (void)cell;
    if (slot.root_id == first_root) {
      bound_to_first = true;
    }
  }
  persistent.Reset();
  return bound_to_first ? 1 : 0;
}

extern "C" int lexe_v8_test_objectwrap_strong_reset_scrubs_layout_maps(
    LexeV8ContextState* ctx_state) {
  const LexeV8BridgeV1* bridge = lexe_v8_bridge();
  auto* isolate = reinterpret_cast<v8::Isolate*>(lexe_v8_current_isolate());
  auto* isolate_impl = lexe_v8::iso_impl(
      reinterpret_cast<LexeV8IsolateState*>(lexe_v8_current_isolate()));
  if (!bridge || !ctx_state || !isolate || !isolate_impl) {
    return 0;
  }

  uint64_t root_id = 0;
  if (bridge->object_new(ctx_state, &root_id) != LEXE_V8_OK || root_id == 0) {
    return 0;
  }
  v8::Local<v8::Object> object = lexe_v8::local_from_root<v8::Object>(
      isolate, root_id, &lexe_v8::shim::Map::object_map());

  v8::Persistent<v8::Object> persistent;
  persistent.Reset(isolate, object);

  uintptr_t layout_addr = 0;
  for (const auto& [cell, slot] : isolate_impl->persistents) {
    if (slot.root_id == root_id) {
      layout_addr = *cell;
      break;
    }
  }
  if (layout_addr == 0) {
    return 0;
  }

  lexe_v8_register_layout_root(reinterpret_cast<void*>(layout_addr), root_id);
  lexe_v8_register_layout_function_id(reinterpret_cast<void*>(layout_addr), 1);

  persistent.Reset();

  const bool maps_clean =
      isolate_impl->layout_to_root.find(layout_addr) == isolate_impl->layout_to_root.end() &&
      isolate_impl->layout_to_function_id.find(layout_addr) ==
          isolate_impl->layout_to_function_id.end();

  uint64_t second_root_id = 0;
  if (bridge->object_new(ctx_state, &second_root_id) == LEXE_V8_OK && second_root_id != 0) {
    v8::Local<v8::Object> second_object = lexe_v8::local_from_root<v8::Object>(
        isolate, second_root_id, &lexe_v8::shim::Map::object_map());
    v8::Persistent<v8::Object> second_persistent;
    second_persistent.Reset(isolate, second_object);
    second_persistent.Reset();
  }

  return maps_clean ? 1 : 0;
}

/// Bypasses repr_to_root fast path: rejects Smi, materializes isolate oddball
/// direct layouts, and safely handles unmaterialized (root_id==0) arena slots.
extern "C" int lexe_v8_test_resolve_root_repr_smi_and_tagged(
    LexeV8ContextState* ctx_state) {
  if (!ctx_state) {
    return 0;
  }
  auto* impl = lexe_v8::ctx_impl(ctx_state);

  const LexeV8BridgeV1* bridge = lexe_v8_bridge();
  if (!bridge || !bridge->root_drop) {
    return 0;
  }

  // General resolver must reject pure Smi (no provenance → not a layout).
  const uintptr_t smi_four =
      (static_cast<uintptr_t>(4) << 32) |
      static_cast<uintptr_t>(lexe_v8::shim::TaggedPointer::Tag::Smi);
  if (lexe_v8::resolve_root_from_repr(ctx_state, smi_four) != 0) {
    return 0;
  }
  // Return-value entry materializes Smi 4 → number root for 4.
  const uint64_t smi_root =
      lexe_v8::resolve_return_value_repr(ctx_state, smi_four);
  if (smi_root == 0) {
    return 0;
  }
  double smi_number = 0.0;
  if (lexe_v8_value_to_float64(ctx_state, smi_root, &smi_number) != LEXE_V8_OK ||
      smi_number != 4.0) {
    bridge->root_drop(smi_root);
    return 0;
  }
  bridge->root_drop(smi_root);

  // Set(0) return-value slot uses raw 0 — must become number root 0.
  const uint64_t zero_root = lexe_v8::resolve_return_value_repr(ctx_state, 0);
  if (zero_root == 0) {
    return 0;
  }
  double zero_number = 1.0;
  if (lexe_v8_value_to_float64(ctx_state, zero_root, &zero_number) != LEXE_V8_OK ||
      zero_number != 0.0) {
    bridge->root_drop(zero_root);
    return 0;
  }
  bridge->root_drop(zero_root);

  // Unknown aligned direct / strong-tagged addresses must not become numbers.
  lexe_v8::shim::ObjectLayout stack_layout;
  const uintptr_t unknown_direct = reinterpret_cast<uintptr_t>(&stack_layout);
  if (lexe_v8::resolve_root_from_repr(ctx_state, unknown_direct) != 0) {
    return 0;
  }
  const uintptr_t unknown_tagged =
      static_cast<uintptr_t>(lexe_v8::shim::TaggedPointer(&stack_layout).value);
  if (lexe_v8::resolve_root_from_repr(ctx_state, unknown_tagged) != 0) {
    return 0;
  }
  if (lexe_v8::resolve_return_value_repr(ctx_state, unknown_direct) != 0) {
    return 0;
  }
  if (lexe_v8::resolve_return_value_repr(ctx_state, unknown_tagged) != 0) {
    return 0;
  }

  // Isolate oddball direct address (root_id starts at 0; not in repr_to_root).
  auto* isolate_state = lexe_v8_current_isolate();
  if (!isolate_state) {
    return 0;
  }
  auto* iso = lexe_v8::iso_impl(isolate_state);
  const uintptr_t undef_addr =
      reinterpret_cast<uintptr_t>(&iso->undefined_value.layout);
  impl->repr_to_root.erase(undef_addr);
  iso->undefined_value.layout.contents.root_id = 0;
  const uint64_t oddball_root =
      lexe_v8::resolve_root_from_repr(ctx_state, undef_addr);
  if (oddball_root == 0) {
    return 0;
  }

  // Unmaterialized arena object: clear root + erase maps so resolve cannot use
  // the registered fast path; direct address must not crash (root may stay 0).
  uint64_t root_id = 0;
  if (bridge->object_new(ctx_state, &root_id) != LEXE_V8_OK || root_id == 0) {
    return 0;
  }
  auto* isolate = reinterpret_cast<v8::Isolate*>(isolate_state);
  v8::Local<v8::Object> object = lexe_v8::local_from_root<v8::Object>(
      isolate, root_id, &lexe_v8::shim::Map::object_map());
  (void)object;

  lexe_v8::shim::ObjectLayout* arena_layout = nullptr;
  uintptr_t tagged_repr = 0;
  uintptr_t direct_addr = 0;
  for (auto& block : impl->arena.blocks) {
    for (auto& slot : block) {
      if (slot.object.contents.root_id == root_id) {
        arena_layout = &slot.object;
        tagged_repr = static_cast<uintptr_t>(slot.object.tagged_map.value);
        direct_addr = reinterpret_cast<uintptr_t>(&slot.object);
        break;
      }
    }
    if (arena_layout) {
      break;
    }
  }
  if (!arena_layout || tagged_repr == 0 || direct_addr == 0) {
    return 0;
  }

  // Scrub provenance so resolve_root_from_repr must re-identify by address.
  impl->repr_to_root.erase(direct_addr);
  impl->repr_to_root.erase(tagged_repr);
  arena_layout->contents.root_id = 0;

  // Unmaterialized direct/tagged: must not UBSan/crash (root stays 0 for plain objects).
  if (lexe_v8::resolve_root_from_repr(ctx_state, direct_addr) != 0) {
    return 0;
  }
  if (lexe_v8::resolve_root_from_repr(ctx_state, tagged_repr) != 0) {
    return 0;
  }

  // Restore root and require known-layout path to recover it without map entry.
  arena_layout->contents.root_id = root_id;
  impl->repr_to_root.erase(direct_addr);
  impl->repr_to_root.erase(tagged_repr);
  if (lexe_v8::resolve_root_from_repr(ctx_state, direct_addr) != root_id) {
    return 0;
  }
  impl->repr_to_root.erase(direct_addr);
  if (lexe_v8::resolve_root_from_repr(ctx_state, tagged_repr) != root_id) {
    return 0;
  }

  return 1;
}
