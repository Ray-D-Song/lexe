# Vendored rquickjs-sys (Lexe patch)

This is rquickjs-sys 0.12.1 with QuickJS patches so `AsyncLocalStorage` can
propagate across `await` / `.then()` and so `ArrayBuffer.transfer` / detach
does not double-free.

Vendored QuickJS baseline: `04e27345bd12e1d9b1eb68d76805126313998`
(document as tracked by this Lexe tree).

## Patch summary

### 1. Promise reaction BEFORE/AFTER identity

Upstream QuickJS only fired `JS_PROMISE_HOOK_BEFORE` / `AFTER` for thenable
resolution jobs. Lexe also wraps normal promise reaction jobs (including
`await` and `.then()`).

**Critical:** BEFORE/AFTER use the **reaction async resource**, not the source
Promise:

- Ordinary `promise.then()`: the resource is the **result Promise** (from the
  capability resolve function).
- `await` / async generator (undefined capability): a **hidden hook Promise**
  is created at schedule time with `parent_promise = source`, so INIT can
  inherit ALS store from the awaiting context; the user-visible Promise identity
  from `Promise.resolve` is unchanged (no forced await wrapper).

`JSPromiseReactionData` holds `hook_promise` (GC-marked/freed with the
reaction). Fulfill and reject reactions share the same resource (separate
refs).

### 2. ArrayBuffer detach (quickjs-ng style)

In `JS_DetachArrayBuffer`, after calling `free_func` on the backing store,
clear `free_func` and `opaque` to `NULL` so the ArrayBuffer finalizer does not
release the same backing store again after `transfer` / detach.

### 3. `JS_SetGCHook` prologue/epilogue notifications

`JS_SetGCHook(rt, gc_hook, opaque)` exposes phase `0` (before) / `1` (after)
notifications around `JS_RunGC(rt)` (distinguishing allocation-triggered GCs
from forced calls via `flags`), used by `node:perf_hooks` GC observers.

### 4. `node:vm` context, module, microtask, and dynamic import hooks

- `JS_SetCodegenFromStrings` / `JS_EvalObject`: per-context toggle for `codeGeneration: { strings: false }` (via static `js_throw_eval_error`).
- `JS_SetGlobalObject` plus exotic `global_obj` handling in `JS_CheckDefineGlobalVar`, `JS_DefineGlobalVar`, and `JS_DefineGlobalFunction` for live contextified `Proxy` globals (using `JS_PROP_CONFIGURABLE` on exotic globals to match V8/Node descriptors), plus fixing the `js_proxy_define_property` ES2020 writable invariant check (`!(flags & JS_PROP_WRITABLE)`).
- `build.rs` emits `cargo:rerun-if-changed=quickjs` and `cargo:rerun-if-changed=quickjs.bind.h`.
- `JS_ExecutePendingJobForContext`: drains microtasks strictly belonging to one `JSContext` (`microtaskMode: "afterEvaluate"`).
- `JS_SetDynamicImportHook`: intercepts `js_dynamic_import` for `importModuleDynamically` callbacks (`JS_UNINITIALIZED` falls back to default loader).
- `JS_EVAL_FLAG_NO_RESOLVE`, `JS_READ_OBJ_NO_RESOLVE`, `JS_ValueToModule`, `JS_GetModuleReqCount`, `JS_GetModuleReqName`, `JS_GetModuleReqAttributes`, `JS_SetModuleReqModule`, `JS_SetModuleResolved`, `JS_VmLinkModule`, `JS_VmEvaluateModule`, `JS_GetModuleStatus`, `JS_GetModuleEvalException`, and `JS_ModuleHasTLA` for `vm.SourceTextModule` and `vm.SyntheticModule`.

### 5. Historical note

An earlier Lexe approach forced a wrapper Promise on every `await`. That was
removed in favor of reaction-resource hooks, which also cover plain `.then()`
used by React/Next.

Wired via workspace `[patch.crates-io]` in the root `Cargo.toml`.
