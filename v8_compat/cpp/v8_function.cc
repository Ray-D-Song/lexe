#include "v8_bridge_helpers.h"

#include <vector>

#include <v8.h>

namespace v8 {

MaybeLocal<Value> Function::Call(Local<Context> context,
                                 Local<Value> recv,
                                 int argc,
                                 Local<Value>* argv) {
  (void)context;
  const LexeV8BridgeV1* b = lexe_v8::bridge();
  auto* ctx = lexe_v8::bridge_ctx();
  if (!b || !ctx || !b->function_call) {
    return MaybeLocal<Value>();
  }
  std::vector<uint64_t> args;
  args.reserve(static_cast<size_t>(argc));
  for (int i = 0; i < argc; ++i) {
    args.push_back(lexe_v8::root_from_local(argv[i]));
  }
  uint64_t out = 0;
  if (b->function_call(ctx, lexe_v8::root_from_object(this), lexe_v8::root_from_local(recv),
                       argc, args.empty() ? nullptr : args.data(), &out) != LEXE_V8_OK) {
    return MaybeLocal<Value>();
  }
  return lexe_v8::local_value_from_root(Isolate::GetCurrent(), out);
}

}  // namespace v8
