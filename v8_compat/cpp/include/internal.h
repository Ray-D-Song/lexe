#pragma once

#include "abi_137_generated.h"
#include "lexe_v8_bridge.h"
#include "shim_types.h"
#include "v8_abi_layout.h"

#include <node.h>
#include <unordered_map>
#include <v8-internal.h>
#include <vector>

namespace lexe_v8 {

struct CallbackHandleFrame {
  std::vector<shim::ObjectLayout> layouts;
  std::vector<v8::internal::Address> values;
  std::vector<uint64_t> roots;
  std::vector<shim::ObjectLayout*> borrowed_layouts;
};

inline thread_local CallbackHandleFrame g_callback_handle_frame;
inline thread_local std::vector<CallbackHandleFrame> g_callback_handle_stack;
inline thread_local shim::ObjectLayout* g_last_materialized_layout = nullptr;

inline CallbackHandleFrame& current_callback_frame() {
  if (g_callback_handle_stack.empty()) {
    return g_callback_handle_frame;
  }
  return g_callback_handle_stack.back();
}

inline void note_materialized_layout(shim::ObjectLayout* layout) {
  g_last_materialized_layout = layout;
}

inline bool is_callback_frame_layout(const shim::ObjectLayout* candidate) {
  if (candidate == nullptr ||
      (reinterpret_cast<uintptr_t>(candidate) % alignof(shim::ObjectLayout)) != 0) {
    return false;
  }
  auto in_frame = [&](const CallbackHandleFrame& frame) {
    for (const auto& layout : frame.layouts) {
      if (&layout == candidate) {
        return true;
      }
    }
    for (const auto* borrowed : frame.borrowed_layouts) {
      if (borrowed == candidate) {
        return true;
      }
    }
    return false;
  };
  for (auto it = g_callback_handle_stack.rbegin(); it != g_callback_handle_stack.rend(); ++it) {
    if (in_frame(*it)) {
      return true;
    }
  }
  return in_frame(g_callback_handle_frame);
}

struct HandleScopeData {
  uintptr_t* next;
  uintptr_t* limit;
  int level;
  int sealed_level;

  void initialize() {
    next = nullptr;
    limit = nullptr;
    level = 0;
    sealed_level = 0;
  }
};

static_assert(sizeof(HandleScopeData) == abi137::kHandleScopeDataSize);

struct OddballValue {
  shim::ObjectLayout layout;
};

struct PersistentSlot {
  uint64_t root_id = 0;
  bool is_weak = false;
  uintptr_t weak_object_ptr = 0;
  uintptr_t context_key = 0;
};

// Layout-compatible fake v8::Isolate for Node 24 / ABI 137.
struct IsolateImpl {
  void* hook0 = nullptr;
  void* hook1 = nullptr;
  uint8_t padding_before_hsd[abi137::kIsolateHandleScopeDataOffset - 16] {};
  HandleScopeData handle_scope_data {};
  uint8_t padding_after_hsd[abi137::kIsolateRootsOffset - abi137::kIsolateHandleScopeDataOffset -
                            abi137::kHandleScopeDataSize] {};
  uintptr_t roots[abi137::kRootSlotCount] {};
  OddballValue undefined_value;
  OddballValue the_hole_value;
  OddballValue null_value;
  OddballValue true_value;
  OddballValue false_value;
  OddballValue empty_string;
  std::unordered_map<uintptr_t*, PersistentSlot> persistents;
  std::unordered_map<uintptr_t, uint64_t> layout_to_root;
  std::unordered_map<uintptr_t, uint32_t> layout_to_function_id;
};

static_assert(offsetof(IsolateImpl, handle_scope_data) == abi137::kIsolateHandleScopeDataOffset);
static_assert(offsetof(IsolateImpl, roots) == abi137::kIsolateRootsOffset);

struct HandleArena {
  static constexpr size_t kBlockSize = 256;
  std::vector<std::vector<shim::HandleSlot>> blocks;
  size_t watermark = 0;
};

struct HandleScopeFrame {
  size_t watermark = 0;
  uintptr_t escape_slot = 0;
};

struct ContextImpl {
  HandleArena arena;
  std::vector<HandleScopeFrame> scopes;
  uint64_t context_root_id = 0;
  uint64_t oddball_roots[abi137::kRootSlotCount] {};
  std::unordered_map<uintptr_t, uint64_t> repr_to_root;
  uintptr_t quickjs_context_key = 0;
};

std::vector<node::node_module*>& pending_v8_modules();
void push_pending_v8_module(node::node_module* module);
void clear_pending_v8_modules();
size_t pending_v8_modules_count();
node::node_module* take_pending_v8_module(size_t index);

ContextImpl* ctx_impl(LexeV8ContextState* ctx);
IsolateImpl* iso_impl(LexeV8IsolateState* isolate);

shim::HandleSlot* alloc_handle_slot(LexeV8ContextState* ctx);
void rewind_handle_arena(LexeV8ContextState* ctx, size_t watermark);

uintptr_t local_from_root(LexeV8ContextState* ctx, uint64_t root_id, const shim::Map* map);
uint64_t root_from_local(uintptr_t tagged);

void register_handle_repr(LexeV8ContextState* ctx, uintptr_t repr, uint64_t root_id);
void unregister_handle_repr(LexeV8ContextState* ctx, uintptr_t repr);
uint64_t resolve_root_from_repr(LexeV8ContextState* ctx, uintptr_t repr);
/// Callback/accessor return-value slot: handles Smi including Set(0) (raw 0).
uint64_t resolve_return_value_repr(LexeV8ContextState* ctx, uintptr_t repr);

void init_isolate_roots(IsolateImpl* isolate);
int oddball_root_index(IsolateImpl* isolate, const shim::ObjectLayout* layout);
HandleScopeData* handle_scope_data(IsolateImpl* isolate);
void dispose_isolate_persistents(IsolateImpl* isolate);
void dispose_context_persistents(IsolateImpl* isolate, uintptr_t context_key);
void persistent_counts_for_context(IsolateImpl* isolate,
                                   uintptr_t context_key,
                                   size_t* strong_out,
                                   size_t* weak_out);
/// Drops only strong (non-weak, root_id != 0) persistents for `context_key`.
/// Returns the number of weak persistents still registered for that context.
size_t dispose_strong_context_persistents(IsolateImpl* isolate, uintptr_t context_key);

/// Removes weak persistent cells for `context_key` after shutdown-time weak dispatch.
size_t dispose_weak_context_persistents(IsolateImpl* isolate, uintptr_t context_key);

LexeV8Status dispatch_v8_callback(uint32_t function_id,
                                    uint64_t receiver_root,
                                    uint64_t new_target_root,
                                    const uint64_t* arg_roots,
                                    int argc,
                                    uint64_t* out_result_root);

// Set for the duration of a V8 callback dispatch; used by GetFunctionTemplateData.
extern thread_local uint32_t g_active_callback_template_id;

}  // namespace lexe_v8
