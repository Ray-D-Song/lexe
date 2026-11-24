#include "lexe_v8_bridge.h"

#include <node.h>

extern "C" void node_module_register(void*);

extern "C" void lexe_v8_force_link(void) {
  void* keep[] = {
      reinterpret_cast<void*>(&node_module_register),
      reinterpret_cast<void*>(&lexe_v8_bind_bridge),
      reinterpret_cast<void*>(&lexe_v8_create_isolate),
      reinterpret_cast<void*>(&lexe_v8_destroy_isolate),
      reinterpret_cast<void*>(&lexe_v8_create_context),
      reinterpret_cast<void*>(&lexe_v8_destroy_context),
      reinterpret_cast<void*>(&lexe_v8_open_handle_scope),
      reinterpret_cast<void*>(&lexe_v8_close_handle_scope),
      reinterpret_cast<void*>(&lexe_v8_pending_modules_count),
      reinterpret_cast<void*>(&lexe_v8_take_pending_module),
  };
  (void)keep;
}
