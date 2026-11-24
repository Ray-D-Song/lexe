#include <node.h>
#include <node_buffer.h>
#include <v8.h>

#include <atomic>
#include <cstring>

#if defined(__unix__) || defined(__APPLE__)
#include <dlfcn.h>
#endif

using v8::Context;
using v8::EscapableHandleScope;
using v8::Function;
using v8::FunctionCallbackInfo;
using v8::HandleScope;
using v8::Isolate;
using v8::Local;
using v8::Object;
using v8::Persistent;
using v8::String;
using v8::Value;
using v8::WeakCallbackInfo;
using v8::WeakCallbackType;

namespace {

void Hello(const FunctionCallbackInfo<Value>& info) {
  Isolate* isolate = info.GetIsolate();
  Local<String> result = String::NewFromUtf8(isolate, "hello from v8").ToLocalChecked();
  info.GetReturnValue().Set(result);
}

// Regression: aligned Smi must not be treated as ObjectLayout* in root resolution.
void ReturnAlignedSmi(const FunctionCallbackInfo<Value>& info) {
  info.GetReturnValue().Set(4);
}

void BufferCopyTest(const FunctionCallbackInfo<Value>& info) {
  Isolate* isolate = info.GetIsolate();
  const char payload[] = "buffer-copy";
  Local<Object> buf =
      node::Buffer::Copy(isolate, payload, sizeof(payload) - 1).ToLocalChecked();
  if (node::Buffer::Length(buf) != sizeof(payload) - 1) {
    isolate->ThrowException(String::NewFromUtf8(isolate, "copy length mismatch").ToLocalChecked());
    return;
  }
  if (std::memcmp(node::Buffer::Data(buf), payload, sizeof(payload) - 1) != 0) {
    isolate->ThrowException(String::NewFromUtf8(isolate, "copy bytes mismatch").ToLocalChecked());
    return;
  }
  info.GetReturnValue().Set(String::NewFromUtf8(isolate, "buffer-copy-ok").ToLocalChecked());
}

static char g_shared_external[8] = "orig";
static std::atomic<int> g_external_finalize_count{0};

void CountedExternalFinalizer(char* data, void* hint) {
  (void)hint;
  g_external_finalize_count.fetch_add(1, std::memory_order_relaxed);
  delete[] data;
}

void NoopExternalFinalizer(char* data, void* hint) {
  (void)data;
  (void)hint;
}

void BufferExternalShared(const FunctionCallbackInfo<Value>& info) {
  Isolate* isolate = info.GetIsolate();
  Local<Object> buf = node::Buffer::New(isolate, g_shared_external, 4, NoopExternalFinalizer, nullptr)
                          .ToLocalChecked();
  info.GetReturnValue().Set(buf);
}

void BufferExternalVerifyJsWrite(const FunctionCallbackInfo<Value>& info) {
  Isolate* isolate = info.GetIsolate();
  if (std::memcmp(g_shared_external, "1234", 4) != 0) {
    isolate->ThrowException(
        String::NewFromUtf8(isolate, "shared buffer js write not visible to native")
            .ToLocalChecked());
    return;
  }
  std::memcpy(g_shared_external, "5678", 4);
  info.GetReturnValue().Set(
      String::NewFromUtf8(isolate, "buffer-shared-ok").ToLocalChecked());
}

void BufferExternalFinalizeOnce(const FunctionCallbackInfo<Value>& info) {
  Isolate* isolate = info.GetIsolate();
  g_external_finalize_count.store(0, std::memory_order_relaxed);
  char* data = new char[2];
  data[0] = 'x';
  data[1] = '\0';
  Local<Object> buf =
      node::Buffer::New(isolate, data, 1, CountedExternalFinalizer, nullptr).ToLocalChecked();
  info.GetReturnValue().Set(buf);
}

void BufferExternalFinalizeCount(const FunctionCallbackInfo<Value>& info) {
  Isolate* isolate = info.GetIsolate();
  info.GetReturnValue().Set(
      v8::Int32::New(isolate, g_external_finalize_count.load(std::memory_order_relaxed)));
}

void ExternalBufferFinalizer(char* data, void* hint) {
  (void)hint;
  delete[] data;
}

void BufferExternalTest(const FunctionCallbackInfo<Value>& info) {
  Isolate* isolate = info.GetIsolate();
  char* data = new char[4];
  std::memcpy(data, "ext", 4);
  Local<Object> buf =
      node::Buffer::New(isolate, data, 3, ExternalBufferFinalizer, nullptr).ToLocalChecked();
  if (node::Buffer::Length(buf) != 3) {
    isolate->ThrowException(String::NewFromUtf8(isolate, "external length mismatch").ToLocalChecked());
    return;
  }
  if (std::memcmp(node::Buffer::Data(buf), "ext", 3) != 0) {
    isolate->ThrowException(String::NewFromUtf8(isolate, "external bytes mismatch").ToLocalChecked());
    return;
  }
  info.GetReturnValue().Set(String::NewFromUtf8(isolate, "buffer-external-ok").ToLocalChecked());
}

bool g_weak_first_pass = false;
bool g_weak_second_pass = false;

struct WeakTestState {
  Persistent<Object> holder;

  static void WeakCallback(const WeakCallbackInfo<WeakTestState>& info) {
    g_weak_first_pass = true;
    info.GetParameter()->holder.Reset();
  }
};

struct TwoPassWeakState {
  Persistent<Object> holder;

  static void SecondPass(const WeakCallbackInfo<TwoPassWeakState>& info) {
    g_weak_second_pass = true;
    info.GetParameter()->holder.Reset();
  }

  static void FirstPass(const WeakCallbackInfo<TwoPassWeakState>& info) {
    g_weak_first_pass = true;
    info.SetSecondPassCallback(SecondPass);
  }
};

void RunGcIfAvailable(const FunctionCallbackInfo<Value>& info) {
  bool ran = false;
#if defined(__unix__) || defined(__APPLE__)
  using RunGcFn = void (*)();
  auto* run_gc = reinterpret_cast<RunGcFn>(dlsym(RTLD_DEFAULT, "lexe_v8_run_gc"));
  if (run_gc != nullptr) {
    run_gc();
    ran = true;
  }
#endif
  info.GetReturnValue().Set(ran);
}

void WeakTwoPassProbe(const FunctionCallbackInfo<Value>& info) {
  Isolate* isolate = info.GetIsolate();
  g_weak_first_pass = false;
  g_weak_second_pass = false;

  auto first_pass = +[](const WeakCallbackInfo<void>& cb_info) {
    g_weak_first_pass = true;
    cb_info.SetSecondPassCallback(+[](const WeakCallbackInfo<void>& second_info) {
      (void)second_info;
      g_weak_second_pass = true;
    });
  };

  void* embedder_fields[v8::kEmbedderFieldsInWeakCallback] = {nullptr, nullptr};
  WeakCallbackInfo<void>::Callback second_pass = nullptr;
  WeakCallbackInfo<void> cb_info(isolate, nullptr, embedder_fields, &second_pass);
  first_pass(cb_info);
  if (second_pass != nullptr) {
    WeakCallbackInfo<void>::Callback unused = nullptr;
    WeakCallbackInfo<void> second_info(isolate, nullptr, embedder_fields, &unused);
    second_pass(second_info);
  }

  if (!g_weak_first_pass || !g_weak_second_pass) {
    isolate->ThrowException(
        String::NewFromUtf8(isolate, "weak two-pass probe failed").ToLocalChecked());
    return;
  }
  info.GetReturnValue().Set(String::NewFromUtf8(isolate, "weak-two-pass-probe-ok").ToLocalChecked());
}

void WeakTwoPassGc(const FunctionCallbackInfo<Value>& info) {
  Isolate* isolate = info.GetIsolate();
  g_weak_first_pass = false;
  g_weak_second_pass = false;

  TwoPassWeakState state;
  {
    HandleScope scope(isolate);
    Local<Object> obj = Object::New(isolate);
    state.holder.Reset(isolate, obj);
    state.holder.SetWeak(&state, TwoPassWeakState::FirstPass, WeakCallbackType::kParameter);
  }

#if defined(__unix__) || defined(__APPLE__)
  using RunGcFn = void (*)();
  auto* run_gc = reinterpret_cast<RunGcFn>(dlsym(RTLD_DEFAULT, "lexe_v8_run_gc"));
  if (run_gc != nullptr) {
    run_gc();
  }
#endif

  if (!g_weak_first_pass || !g_weak_second_pass || !state.holder.IsEmpty()) {
    isolate->ThrowException(
        String::NewFromUtf8(isolate, "weak two-pass gc failed").ToLocalChecked());
    return;
  }
  info.GetReturnValue().Set(String::NewFromUtf8(isolate, "weak-two-pass-gc-ok").ToLocalChecked());
}

void WeakShutdownOnly(const FunctionCallbackInfo<Value>& info) {
  Isolate* isolate = info.GetIsolate();
  static WeakTestState state;
  Local<Object> obj = Object::New(isolate);
  state.holder.Reset(isolate, obj);
  state.holder.SetWeak(&state, WeakTestState::WeakCallback, WeakCallbackType::kParameter);
  info.GetReturnValue().Set(String::NewFromUtf8(isolate, "weak-shutdown-only-ok").ToLocalChecked());
}

void PersistentLifecycleTest(const FunctionCallbackInfo<Value>& info) {
  Isolate* isolate = info.GetIsolate();
  Persistent<Object> holder;
  if (!holder.IsEmpty()) {
    isolate->ThrowException(
        String::NewFromUtf8(isolate, "persistent should start empty").ToLocalChecked());
    return;
  }

  Local<Object> obj = Object::New(isolate);
  holder.Reset(isolate, obj);
  if (holder.IsEmpty()) {
    isolate->ThrowException(
        String::NewFromUtf8(isolate, "persistent reset failed").ToLocalChecked());
    return;
  }

  holder.Reset();
  if (!holder.IsEmpty()) {
    isolate->ThrowException(
        String::NewFromUtf8(isolate, "persistent clear failed").ToLocalChecked());
    return;
  }

  holder.Reset(isolate, obj);
  {
    struct LifecycleState {
      static void NopWeak(const WeakCallbackInfo<LifecycleState>&) {}
    };
    LifecycleState lifecycle;
    holder.SetWeak(&lifecycle, LifecycleState::NopWeak, WeakCallbackType::kParameter);
  }
  holder.ClearWeak();
  holder.Reset();
  if (!holder.IsEmpty()) {
    isolate->ThrowException(
        String::NewFromUtf8(isolate, "persistent weak lifecycle failed").ToLocalChecked());
    return;
  }

  info.GetReturnValue().Set(
      String::NewFromUtf8(isolate, "persistent-lifecycle-ok").ToLocalChecked());
}

void BufferProbeTest(const FunctionCallbackInfo<Value>& info) {
  Isolate* isolate = info.GetIsolate();
  if (info.Length() < 1 || !info[0]->IsObject()) {
    isolate->ThrowException(String::NewFromUtf8(isolate, "expected Buffer argument").ToLocalChecked());
    return;
  }
  Local<Object> buf = info[0].As<Object>();
  if (!node::Buffer::HasInstance(buf)) {
    isolate->ThrowException(String::NewFromUtf8(isolate, "argument is not a Buffer").ToLocalChecked());
    return;
  }
  if (node::Buffer::Length(buf) != 5) {
    isolate->ThrowException(String::NewFromUtf8(isolate, "buffer length mismatch").ToLocalChecked());
    return;
  }
  if (std::memcmp(node::Buffer::Data(buf), "probe", 5) != 0) {
    isolate->ThrowException(String::NewFromUtf8(isolate, "buffer bytes mismatch").ToLocalChecked());
    return;
  }
  info.GetReturnValue().Set(String::NewFromUtf8(isolate, "buffer-probe-ok").ToLocalChecked());
}

void EscapableHandleScopeOnce(const FunctionCallbackInfo<Value>& info) {
  Isolate* isolate = info.GetIsolate();
  Local<String> escaped;
  {
    EscapableHandleScope scope(isolate);
    Local<String> inner =
        String::NewFromUtf8(isolate, "escaped-value").ToLocalChecked();
    escaped = scope.Escape(inner);
  }
  if (escaped.IsEmpty()) {
    isolate->ThrowException(
        String::NewFromUtf8(isolate, "escaped handle empty").ToLocalChecked());
    return;
  }
  Local<String> expected =
      String::NewFromUtf8(isolate, "escaped-value").ToLocalChecked();
  if (!escaped->StrictEquals(expected)) {
    isolate->ThrowException(
        String::NewFromUtf8(isolate, "escaped value mismatch").ToLocalChecked());
    return;
  }
  info.GetReturnValue().Set(
      String::NewFromUtf8(isolate, "escapable-once-ok").ToLocalChecked());
}

void EscapableHandleScopeTwice(const FunctionCallbackInfo<Value>& info) {
  Isolate* isolate = info.GetIsolate();
  EscapableHandleScope scope(isolate);
  Local<String> a = String::NewFromUtf8(isolate, "a").ToLocalChecked();
  Local<String> b = String::NewFromUtf8(isolate, "b").ToLocalChecked();
  Local<String> first = scope.Escape(a);
  if (first.IsEmpty()) {
    isolate->ThrowException(
        String::NewFromUtf8(isolate, "first escape failed").ToLocalChecked());
    return;
  }
  Local<String> second = scope.Escape(b);
  if (second.IsEmpty()) {
    info.GetReturnValue().Set(
        String::NewFromUtf8(isolate, "escapable-twice-ok").ToLocalChecked());
    return;
  }
  // Stock V8 may return the inner handle on a second Escape.
  info.GetReturnValue().Set(
      String::NewFromUtf8(isolate, "escapable-twice-node-ok").ToLocalChecked());
}

void WeakClearTest(const FunctionCallbackInfo<Value>& info) {
  Isolate* isolate = info.GetIsolate();
  g_weak_first_pass = false;
  g_weak_second_pass = false;
  Local<Object> obj = Object::New(isolate);
  WeakTestState state;
  state.holder.Reset(isolate, obj);
  state.holder.SetWeak(&state, WeakTestState::WeakCallback, WeakCallbackType::kParameter);
  state.holder.ClearWeak();
  state.holder.Reset();
  if (!g_weak_first_pass && !g_weak_second_pass) {
    info.GetReturnValue().Set(String::NewFromUtf8(isolate, "weak-clear-ok").ToLocalChecked());
    return;
  }
  isolate->ThrowException(String::NewFromUtf8(isolate, "weak callback fired after ClearWeak")
                              .ToLocalChecked());
}

// NODE_MODULE_CONTEXT_AWARE expects addon_context_register_func (4 args).
// A 3-arg Init is only accepted via cast and trips UBSan on the indirect call.
void Init(Local<Object> exports,
          Local<Value> module,
          Local<Context> context,
          void* priv) {
  (void)module;
  (void)context;
  (void)priv;
  NODE_SET_METHOD(exports, "hello", Hello);
  NODE_SET_METHOD(exports, "returnAlignedSmi", ReturnAlignedSmi);
  NODE_SET_METHOD(exports, "bufferCopy", BufferCopyTest);
  NODE_SET_METHOD(exports, "bufferExternal", BufferExternalTest);
  NODE_SET_METHOD(exports, "bufferExternalShared", BufferExternalShared);
  NODE_SET_METHOD(exports, "bufferExternalVerifyJsWrite", BufferExternalVerifyJsWrite);
  NODE_SET_METHOD(exports, "bufferExternalFinalizeOnce", BufferExternalFinalizeOnce);
  NODE_SET_METHOD(exports, "bufferExternalFinalizeCount", BufferExternalFinalizeCount);
  NODE_SET_METHOD(exports, "bufferProbe", BufferProbeTest);
  NODE_SET_METHOD(exports, "escapableOnce", EscapableHandleScopeOnce);
  NODE_SET_METHOD(exports, "escapableTwice", EscapableHandleScopeTwice);
  NODE_SET_METHOD(exports, "weakClear", WeakClearTest);
  NODE_SET_METHOD(exports, "weakTwoPassProbe", WeakTwoPassProbe);
  NODE_SET_METHOD(exports, "weakTwoPassGc", WeakTwoPassGc);
  NODE_SET_METHOD(exports, "runGc", RunGcIfAvailable);
  NODE_SET_METHOD(exports, "persistentLifecycle", PersistentLifecycleTest);
  NODE_SET_METHOD(exports, "weakShutdownOnly", WeakShutdownOnly);
}

}  // namespace

NODE_MODULE_CONTEXT_AWARE(v8_hello, Init)
