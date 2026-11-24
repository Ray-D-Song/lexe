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

### 3. Historical note

An earlier Lexe approach forced a wrapper Promise on every `await`. That was
removed in favor of reaction-resource hooks, which also cover plain `.then()`
used by React/Next.

Wired via workspace `[patch.crates-io]` in the root `Cargo.toml`.
