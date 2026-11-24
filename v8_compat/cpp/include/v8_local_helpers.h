#pragma once

#include "internal.h"

#include <v8-local-handle.h>

namespace v8 {

class HandleScopeHelper : public HandleScope {
 public:
  static internal::Address* MakeHandle(Isolate* isolate, internal::Address value) {
    return CreateHandle(reinterpret_cast<internal::Isolate*>(isolate), value);
  }
};

template <typename T>
inline Local<T> MakeLocalFromObject(Isolate* isolate, lexe_v8::shim::ObjectLayout* object) {
  internal::Address* slot =
      HandleScopeHelper::MakeHandle(isolate, reinterpret_cast<internal::Address>(object));
  return *reinterpret_cast<Local<T>*>(&slot);
}

}  // namespace v8
