#include "v8_bridge_helpers.h"

#include <v8-external.h>

namespace v8 {

Local<External> External::New(Isolate* isolate, void* value) {
  const LexeV8BridgeV1* b = lexe_v8::bridge();
  auto* ctx = lexe_v8::bridge_ctx();
  if (!b || !ctx || !b->external_new) {
    return Local<External>();
  }
  uint64_t root_id = 0;
  if (b->external_new(ctx, value, &root_id) != LEXE_V8_OK) {
    return Local<External>();
  }
  return lexe_v8::local_from_root<External>(isolate, root_id, &lexe_v8::shim::Map::object_map());
}

}  // namespace v8
