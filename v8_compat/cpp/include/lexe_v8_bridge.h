#pragma once

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define LEXE_V8_BRIDGE_VERSION 1

typedef enum LexeV8Status {
  LEXE_V8_OK = 0,
  LEXE_V8_EXCEPTION = 1,
  LEXE_V8_UNSUPPORTED = 2,
  LEXE_V8_WRONG_THREAD = 3,
  LEXE_V8_ERROR = 4,
} LexeV8Status;

typedef struct LexeV8ContextState LexeV8ContextState;
typedef struct LexeV8IsolateState LexeV8IsolateState;

typedef LexeV8Status (*LexeV8RootDupFn)(uint64_t root_id, uint64_t* out_root_id);
typedef LexeV8Status (*LexeV8RootDropFn)(uint64_t root_id);
typedef LexeV8Status (*LexeV8RootMakeWeakFn)(uint64_t root_id);
typedef LexeV8Status (*LexeV8RootFromJsFn)(
    LexeV8ContextState* ctx,
    uint64_t js_value_tag,
    uint64_t* out_root_id);
typedef LexeV8Status (*LexeV8RootToJsFn)(
    LexeV8ContextState* ctx,
    uint64_t root_id,
    uint64_t* out_js_value_tag);
typedef LexeV8Status (*LexeV8ThrowTypeErrorFn)(LexeV8ContextState* ctx, const char* message);
typedef void (*LexeV8FatalFn)(const char* api_name, const char* message);
typedef LexeV8Status (*LexeV8OddballRootFn)(
    LexeV8ContextState* ctx,
    int root_index,
    uint64_t* out_root_id);

typedef LexeV8Status (*LexeV8StringNewUtf8Fn)(
    LexeV8ContextState* ctx,
    const char* data,
    int length,
    uint64_t* out_root_id);
typedef LexeV8Status (*LexeV8ObjectNewFn)(LexeV8ContextState* ctx, uint64_t* out_root_id);
typedef LexeV8Status (*LexeV8ObjectSetFn)(
    LexeV8ContextState* ctx,
    uint64_t object_root_id,
    uint64_t key_root_id,
    uint64_t value_root_id);
typedef LexeV8Status (*LexeV8FunctionTemplateNewFn)(
    LexeV8ContextState* ctx,
    uint32_t template_id,
    void* callback,
    uint64_t data_root_id,
    uint64_t* out_template_id);
typedef LexeV8Status (*LexeV8FunctionTemplateGetFunctionFn)(
    LexeV8ContextState* ctx,
    uint32_t template_id,
    uint64_t* out_root_id);
typedef LexeV8Status (*LexeV8DispatchFunctionFn)(
    LexeV8ContextState* ctx,
    uint32_t function_id,
    uint64_t receiver_root_id,
    uint64_t new_target_root_id,
    const uint64_t* arg_root_ids,
    int argc,
    uint64_t* out_result_root_id);
typedef LexeV8Status (*LexeV8RunModuleInitFn)(
    LexeV8ContextState* ctx,
    void (*init_fn)(void*, void*, void*),
    uint64_t exports_root_id,
    uint64_t module_root_id,
    uint64_t* out_exports_root_id);

typedef LexeV8Status (*LexeV8ObjectGetFn)(
    LexeV8ContextState* ctx,
    uint64_t object_root_id,
    uint64_t key_root_id,
    uint64_t* out_root_id);
typedef LexeV8Status (*LexeV8ObjectGetIndexFn)(
    LexeV8ContextState* ctx,
    uint64_t object_root_id,
    uint32_t index,
    uint64_t* out_root_id);
typedef LexeV8Status (*LexeV8ObjectSetIndexFn)(
    LexeV8ContextState* ctx,
    uint64_t object_root_id,
    uint32_t index,
    uint64_t value_root_id);
typedef LexeV8Status (*LexeV8ObjectDefineOwnPropertyFn)(
    LexeV8ContextState* ctx,
    uint64_t object_root_id,
    uint64_t key_root_id,
    uint64_t value_root_id,
    int attr,
    bool* out_ok);
typedef LexeV8Status (*LexeV8ObjectHasOwnPropertyFn)(
    LexeV8ContextState* ctx,
    uint64_t object_root_id,
    uint64_t key_root_id,
    bool* out_ok);
typedef LexeV8Status (*LexeV8ObjectGetPrototypeFn)(
    LexeV8ContextState* ctx,
    uint64_t object_root_id,
    uint64_t* out_root_id);
typedef LexeV8Status (*LexeV8ArrayNewFn)(LexeV8ContextState* ctx, int length, uint64_t* out_root_id);
typedef LexeV8Status (*LexeV8NumberNewFn)(LexeV8ContextState* ctx, double value, uint64_t* out_root_id);
typedef LexeV8Status (*LexeV8BigIntNewFn)(LexeV8ContextState* ctx, int64_t value, uint64_t* out_root_id);
typedef LexeV8Status (*LexeV8IntegerNewFn)(LexeV8ContextState* ctx, int value, uint64_t* out_root_id);
typedef LexeV8Status (*LexeV8StringNewLatin1Fn)(
    LexeV8ContextState* ctx,
    const uint8_t* data,
    int length,
    uint64_t* out_root_id);
typedef LexeV8Status (*LexeV8StringToUtf8Fn)(
    LexeV8ContextState* ctx,
    uint64_t value_root_id,
    char** out_ptr,
    size_t* out_len);
typedef LexeV8Status (*LexeV8StringFreeUtf8Fn)(LexeV8ContextState* ctx, char* ptr);
typedef LexeV8Status (*LexeV8FunctionCallFn)(
    LexeV8ContextState* ctx,
    uint64_t func_root_id,
    uint64_t recv_root_id,
    int argc,
    const uint64_t* arg_root_ids,
    uint64_t* out_root_id);
typedef LexeV8Status (*LexeV8ThrowValueFn)(LexeV8ContextState* ctx, uint64_t value_root_id);
typedef LexeV8Status (*LexeV8NewExceptionFn)(
    LexeV8ContextState* ctx,
    uint64_t msg_root_id,
    int kind,
    uint64_t* out_root_id);
typedef LexeV8Status (*LexeV8ExternalNewFn)(LexeV8ContextState* ctx, void* ptr, uint64_t* out_root_id);
typedef LexeV8Status (*LexeV8InternalFieldSetFn)(
    LexeV8ContextState* ctx,
    uint64_t object_root_id,
    int index,
    void* ptr);
typedef LexeV8Status (*LexeV8InternalFieldGetFn)(
    LexeV8ContextState* ctx,
    uint64_t object_root_id,
    int index,
    void** out_ptr);
typedef LexeV8Status (*LexeV8SymbolIteratorFn)(LexeV8ContextState* ctx, uint64_t* out_root_id);
typedef LexeV8Status (*LexeV8GetCreationContextFn)(
    LexeV8ContextState* ctx,
    uint64_t object_root_id,
    uint64_t* out_root_id);
typedef LexeV8Status (*LexeV8RegisterWeakCallbackFn)(
    LexeV8ContextState* ctx,
    uint64_t object_root_id,
    void* parameter,
    void* callback);
typedef LexeV8Status (*LexeV8GetContextRootFn)(LexeV8ContextState* ctx, uint64_t* out_root_id);

typedef struct LexeV8BridgeV1 {
  uint32_t version;
  uint32_t node_module_version;
  LexeV8RootDupFn root_dup;
  LexeV8RootDropFn root_drop;
  LexeV8RootMakeWeakFn root_make_weak;
  LexeV8RootFromJsFn root_from_js;
  LexeV8RootToJsFn root_to_js;
  LexeV8ThrowTypeErrorFn throw_type_error;
  LexeV8FatalFn fatal;
  LexeV8StringNewUtf8Fn string_new_utf8;
  LexeV8ObjectNewFn object_new;
  LexeV8ObjectSetFn object_set;
  LexeV8FunctionTemplateNewFn function_template_new;
  LexeV8FunctionTemplateGetFunctionFn function_template_get_function;
    LexeV8DispatchFunctionFn dispatch_function;
    LexeV8RunModuleInitFn run_module_init;
    LexeV8ObjectGetFn object_get;
    LexeV8ObjectGetIndexFn object_get_index;
    LexeV8ObjectSetIndexFn object_set_index;
    LexeV8ObjectDefineOwnPropertyFn object_define_own_property;
    LexeV8ObjectHasOwnPropertyFn object_has_own_property;
    LexeV8ObjectGetPrototypeFn object_get_prototype;
    LexeV8ArrayNewFn array_new;
    LexeV8NumberNewFn number_new;
    LexeV8BigIntNewFn bigint_new;
    LexeV8IntegerNewFn integer_new;
    LexeV8StringNewLatin1Fn string_new_latin1;
    LexeV8StringToUtf8Fn string_to_utf8;
    LexeV8StringFreeUtf8Fn string_free_utf8;
    LexeV8FunctionCallFn function_call;
    LexeV8ThrowValueFn throw_value;
    LexeV8NewExceptionFn new_exception;
    LexeV8ExternalNewFn external_new;
    LexeV8InternalFieldSetFn internal_field_set;
    LexeV8InternalFieldGetFn internal_field_get;
    LexeV8SymbolIteratorFn symbol_iterator;
    LexeV8GetCreationContextFn get_creation_context;
    LexeV8RegisterWeakCallbackFn register_weak_callback;
    LexeV8GetContextRootFn get_context_root;
} LexeV8BridgeV1;

LexeV8Status lexe_v8_run_module_init(
    LexeV8ContextState* ctx,
    void* module,
    uint64_t exports_root_id,
    uint64_t module_root_id,
    uint64_t* out_exports_root_id);

void lexe_v8_bind_bridge(const LexeV8BridgeV1* bridge);
void lexe_v8_set_oddball_root_fn(LexeV8OddballRootFn fn);
const LexeV8BridgeV1* lexe_v8_bridge(void);

LexeV8ContextState* lexe_v8_current_context(void);
LexeV8IsolateState* lexe_v8_current_isolate(void);
void lexe_v8_set_current_context(LexeV8ContextState* ctx);
void lexe_v8_set_current_isolate(LexeV8IsolateState* isolate);

LexeV8IsolateState* lexe_v8_create_isolate(void);
void lexe_v8_destroy_isolate(LexeV8IsolateState* isolate);
LexeV8ContextState* lexe_v8_create_context(void);
void lexe_v8_destroy_context(LexeV8ContextState* ctx);
void lexe_v8_open_handle_scope(LexeV8ContextState* ctx);
void lexe_v8_close_handle_scope(LexeV8ContextState* ctx);
void lexe_v8_set_context_root_id(LexeV8ContextState* ctx, uint64_t root_id);
void lexe_v8_set_context_quickjs_key(LexeV8ContextState* ctx, uintptr_t key);
uint64_t lexe_v8_context_root_id(LexeV8ContextState* ctx);
LexeV8Status lexe_v8_oddball_root(LexeV8ContextState* ctx,
                                      int root_index,
                                      uint64_t* out_root_id);
LexeV8Status lexe_v8_dispatch_callback(uint32_t function_id,
                                           uint64_t receiver_root,
                                           uint64_t new_target_root,
                                           const uint64_t* arg_roots,
                                           int argc,
                                           uint64_t* out_result_root);
uint32_t lexe_v8_function_template_id(uint32_t function_id);
int32_t lexe_v8_instance_internal_field_count(uint32_t template_id);
uint32_t lexe_v8_function_prototype_template_id(uint32_t template_id);
void lexe_v8_set_function_template_prototype_root(uint32_t template_id, uint64_t root_id);
uint64_t lexe_v8_function_template_prototype_root(uint32_t template_id);
size_t lexe_v8_object_template_property_count(uint32_t object_template_id);
int lexe_v8_object_template_property_at(uint32_t object_template_id,
                                          size_t index,
                                          uint64_t* key_root,
                                          uint32_t* value_template_id);
uint32_t lexe_v8_register_function_for_template(uint32_t template_id);
size_t lexe_v8_object_template_native_property_count(uint32_t object_template_id);
int lexe_v8_object_template_native_property_at(uint32_t object_template_id,
                                               size_t index,
                                               uint64_t* name_root,
                                               uint32_t* accessor_id);
LexeV8Status lexe_v8_dispatch_accessor(uint32_t accessor_id,
                                           uint64_t receiver_root,
                                           void* embedder_override,
                                           uint64_t* out_result_root);
LexeV8Status lexe_v8_object_internal_field_count(LexeV8ContextState* ctx,
                                                     uint64_t object_root,
                                                     int* out);
LexeV8Status lexe_v8_object_reserve_internal_fields(LexeV8ContextState* ctx,
                                                      uint64_t object_root,
                                                      int count);
size_t lexe_v8_pending_modules_count(void);
void* lexe_v8_take_pending_module(size_t index);

void lexe_v8_force_link(void);

enum LexeV8ValueLayoutKind : uint8_t {
  LEXE_V8_LAYOUT_OBJECT = 0,
  LEXE_V8_LAYOUT_STRING = 1,
  LEXE_V8_LAYOUT_ODDBALL = 2,
  LEXE_V8_LAYOUT_HEAP_NUMBER = 3,
  LEXE_V8_LAYOUT_INT32_SMI = 4,
};

LexeV8Status lexe_v8_value_layout_kind(LexeV8ContextState* ctx, uint64_t root,
                                           uint8_t* out_kind, int32_t* out_smi);
LexeV8Status lexe_v8_value_is_object(LexeV8ContextState* ctx, uint64_t root, bool* out);
LexeV8Status lexe_v8_value_is_array(LexeV8ContextState* ctx, uint64_t root, bool* out);
LexeV8Status lexe_v8_value_is_function(LexeV8ContextState* ctx, uint64_t root, bool* out);
LexeV8Status lexe_v8_value_is_number(LexeV8ContextState* ctx, uint64_t root, bool* out);
LexeV8Status lexe_v8_value_is_int32(LexeV8ContextState* ctx, uint64_t root, bool* out);
LexeV8Status lexe_v8_value_is_bigint(LexeV8ContextState* ctx, uint64_t root, bool* out);
LexeV8Status lexe_v8_value_is_boolean(LexeV8ContextState* ctx, uint64_t root, bool* out);
LexeV8Status lexe_v8_value_to_boolean(LexeV8ContextState* ctx, uint64_t root, bool* out);
LexeV8Status lexe_v8_value_strict_equals(LexeV8ContextState* ctx, uint64_t a, uint64_t b, bool* out);
LexeV8Status lexe_v8_value_to_float64(LexeV8ContextState* ctx, uint64_t root, double* out);
LexeV8Status lexe_v8_value_to_int32(LexeV8ContextState* ctx, uint64_t root, int32_t* out);
LexeV8Status lexe_v8_value_to_int64(LexeV8ContextState* ctx, uint64_t root, int64_t* out, bool* lossless);
LexeV8Status lexe_v8_array_length(LexeV8ContextState* ctx, uint64_t root, uint32_t* out);
LexeV8Status lexe_v8_function_new_instance(LexeV8ContextState* ctx, uint64_t func_root, int argc,
                                               const uint64_t* args, uint64_t* out);
LexeV8Status lexe_v8_buffer_new_copy(LexeV8ContextState* ctx, const uint8_t* data, size_t len,
                                        uint64_t* out);
LexeV8Status lexe_v8_buffer_new_external(LexeV8ContextState* ctx, uint8_t* data, size_t len,
                                             void (*callback)(uint8_t*, void*), void* hint,
                                             uint64_t* out);
LexeV8Status lexe_v8_buffer_data(LexeV8ContextState* ctx, uint64_t root, uint8_t** out_ptr,
                                     size_t* out_len);
LexeV8Status lexe_v8_internal_field_get(LexeV8ContextState* ctx, uint64_t object_root, int index,
                                            void** out_ptr);
LexeV8Status lexe_v8_internal_field_set(LexeV8ContextState* ctx, uint64_t object_root, int index,
                                            void* ptr);
LexeV8Status lexe_v8_root_id_for_js_object(LexeV8ContextState* ctx, void* object_ptr,
                                               uint64_t* out_root_id);
LexeV8Status lexe_v8_object_ptr_for_root(LexeV8ContextState* ctx, uint64_t root,
                                             void** out_object_ptr);
LexeV8Status lexe_v8_root_restrong_from_object_ptr(LexeV8ContextState* ctx, void* object_ptr,
                                                     uint64_t* out_root_id);
void lexe_v8_unregister_weak_for_object_ptr(LexeV8ContextState* ctx, void* object_ptr);
LexeV8Status lexe_v8_root_id_for_persistent_layout(void* layout, uint64_t* out_root_id);
void lexe_v8_register_layout_root(void* layout, uint64_t root_id);
void lexe_v8_register_layout_function_id(void* layout, uint32_t function_id);
LexeV8Status lexe_v8_function_id_for_layout(void* layout, uint32_t* out_function_id);
LexeV8Status lexe_v8_function_id_for_root(LexeV8ContextState* ctx, uint64_t root_id,
                                              uint32_t* out_function_id);
LexeV8Status lexe_v8_function_root_for_id(LexeV8ContextState* ctx, uint32_t function_id,
                                             uint64_t* out_root_id);
LexeV8Status lexe_v8_buffer_has_instance(LexeV8ContextState* ctx, uint64_t root, bool* out);
void lexe_v8_add_env_cleanup_hook(LexeV8IsolateState* isolate, void (*cb)(void*), void* arg);
void lexe_v8_remove_env_cleanup_hook(LexeV8IsolateState* isolate, void (*cb)(void*), void* arg);
void lexe_v8_invoke_weak_callback(void* callback, void* parameter);
int lexe_v8_invoke_weak_callback_first_pass(void* callback, void* parameter, void** out_second_pass);
void lexe_v8_invoke_weak_callback_second_pass(void* callback, void* parameter);
void lexe_v8_dispatch_pending_weak_callbacks(void);
void lexe_v8_run_gc(void);
void lexe_v8_dispose_all_persistents(void);
void lexe_v8_dispose_context_persistents(void* isolate, uintptr_t context_key);
void lexe_v8_persistent_counts_for_context(void* isolate,
                                            uintptr_t context_key,
                                            size_t* strong_out,
                                            size_t* weak_out);
size_t lexe_v8_dispose_strong_context_persistents(void* isolate, uintptr_t context_key);
size_t lexe_v8_dispose_weak_context_persistents(void* isolate, uintptr_t context_key);
void lexe_v8_clear_registries_for_context(uintptr_t context_key);
size_t lexe_v8_function_template_ids_for_context(uintptr_t context_key,
                                                 uint32_t* out,
                                                 size_t capacity);

#ifdef __cplusplus
}
#endif
