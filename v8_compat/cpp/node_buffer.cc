#include "v8_bridge_helpers.h"

#include <node.h>
#include <v8.h>

namespace node {

void AddEnvironmentCleanupHook(v8::Isolate* isolate, void (*fun)(void*), void* arg) {
  lexe_v8_add_env_cleanup_hook(reinterpret_cast<LexeV8IsolateState*>(isolate),
                                 reinterpret_cast<void (*)(void*)>(fun), arg);
}

void RemoveEnvironmentCleanupHook(v8::Isolate* isolate, void (*fun)(void*), void* arg) {
  lexe_v8_remove_env_cleanup_hook(reinterpret_cast<LexeV8IsolateState*>(isolate),
                                    reinterpret_cast<void (*)(void*)>(fun), arg);
}

namespace Buffer {

bool HasInstance(v8::Local<v8::Value> val) {
  auto* ctx = lexe_v8::bridge_ctx();
  if (!ctx) {
    return false;
  }
  bool out = false;
  if (lexe_v8_buffer_has_instance(ctx, lexe_v8::root_from_local(val), &out) != LEXE_V8_OK) {
    return false;
  }
  return out;
}

bool HasInstance(v8::Local<v8::Object> obj) {
  return HasInstance(v8::Local<v8::Value>(obj));
}

v8::MaybeLocal<v8::Object> New(v8::Isolate* isolate,
                                 char* data,
                                 size_t length,
                                 void (*callback)(char*, void*),
                                 void* hint) {
  auto* ctx = lexe_v8::bridge_ctx();
  if (!ctx) {
    return v8::MaybeLocal<v8::Object>();
  }
  uint64_t root = 0;
  if (callback != nullptr) {
    auto* external_cb = reinterpret_cast<void (*)(uint8_t*, void*)>(callback);
    if (lexe_v8_buffer_new_external(ctx, reinterpret_cast<uint8_t*>(data), length, external_cb,
                                      hint, &root) != LEXE_V8_OK) {
      return v8::MaybeLocal<v8::Object>();
    }
  } else if (lexe_v8_buffer_new_copy(ctx, reinterpret_cast<const uint8_t*>(data), length,
                                        &root) != LEXE_V8_OK) {
    return v8::MaybeLocal<v8::Object>();
  }
  return lexe_v8::local_from_root<v8::Object>(isolate, root, &lexe_v8::shim::Map::object_map());
}

v8::MaybeLocal<v8::Object> Copy(v8::Isolate* isolate, const char* data, size_t length) {
  auto* ctx = lexe_v8::bridge_ctx();
  if (!ctx) {
    return v8::MaybeLocal<v8::Object>();
  }
  uint64_t root = 0;
  if (lexe_v8_buffer_new_copy(ctx, reinterpret_cast<const uint8_t*>(data), length, &root) !=
      LEXE_V8_OK) {
    return v8::MaybeLocal<v8::Object>();
  }
  return lexe_v8::local_from_root<v8::Object>(isolate, root, &lexe_v8::shim::Map::object_map());
}

char* Data(v8::Local<v8::Value> val) {
  auto* ctx = lexe_v8::bridge_ctx();
  if (!ctx) {
    return nullptr;
  }
  uint8_t* ptr = nullptr;
  size_t len = 0;
  if (lexe_v8_buffer_data(ctx, lexe_v8::root_from_local(val), &ptr, &len) != LEXE_V8_OK) {
    return nullptr;
  }
  return reinterpret_cast<char*>(ptr);
}

char* Data(v8::Local<v8::Object> obj) {
  return Data(v8::Local<v8::Value>(obj));
}

size_t Length(v8::Local<v8::Value> val) {
  auto* ctx = lexe_v8::bridge_ctx();
  if (!ctx) {
    return 0;
  }
  uint8_t* ptr = nullptr;
  size_t len = 0;
  if (lexe_v8_buffer_data(ctx, lexe_v8::root_from_local(val), &ptr, &len) != LEXE_V8_OK) {
    return 0;
  }
  return len;
}

size_t Length(v8::Local<v8::Object> obj) {
  return Length(v8::Local<v8::Value>(obj));
}

}  // namespace Buffer
}  // namespace node
