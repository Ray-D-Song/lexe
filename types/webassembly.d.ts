export {};

/**
 * Core `WebAssembly` JavaScript API, backed by `wasmi` 1.1.0.
 *
 * Scope of this implementation:
 * - `compile`, `instantiate`, `validate`, `compileStreaming`, `instantiateStreaming`.
 * - `Module`, `Instance`, `Memory`, `Table`, `Global`.
 * - `CompileError`, `LinkError`, `RuntimeError`.
 * - `Module.imports`/`exports`/`customSections`.
 * - JS function imports, Wasm function exports, multi-value results,
 *   `i64` <-> `BigInt`, `externref`, `funcref`, and SIMD (executed internally
 *   inside Wasm only -- `v128` values cannot cross the JS/Wasm boundary).
 * - A synchronously-mirrored, non-shared `wasm32` linear `Memory`.
 *
 * Explicitly NOT implemented (do not add ambient declarations for these):
 * - `WebAssembly.Function`, `WebAssembly.Tag`, `WebAssembly.Exception`.
 * - JSPI (`Suspending`, `promising`).
 * - The GC, exception-handling, and function-references proposals.
 * - Shared memory, threads, and `Memory64`.
 * - WASI and the `.wasm` ESM loader.
 *
 * @see [source](https://webassembly.github.io/spec/js-api/)
 */
declare global {
  namespace WebAssembly {
    /** An `ArrayBuffer` or a view over one (e.g. a typed array or `DataView`). */
    type BufferSource = QuickJS.ArrayBufferView | ArrayBuffer;

    /** Element kind for a {@link Table}. `anyfunc` is an alias of `funcref`. */
    type TableKind = "anyfunc" | "funcref" | "externref";

    /** Value type accepted by a {@link Global}. */
    type ValueType = "i32" | "i64" | "f32" | "f64" | "anyfunc" | "funcref" | "externref";

    /** The kind of an entry returned by {@link Module.imports} / {@link Module.exports}. */
    type ImportExportKind = "function" | "table" | "memory" | "global";

    interface ModuleImportDescriptor {
      readonly module: string;
      readonly name: string;
      readonly kind: ImportExportKind;
    }

    interface ModuleExportDescriptor {
      readonly name: string;
      readonly kind: ImportExportKind;
    }

    interface MemoryDescriptor {
      readonly initial: number;
      readonly maximum?: number;
      /** Shared memory is out of scope; passing `true` throws a `TypeError`. */
      readonly shared?: false;
    }

    interface TableDescriptor {
      readonly element: TableKind;
      readonly initial: number;
      readonly maximum?: number;
    }

    interface GlobalDescriptor {
      readonly value: ValueType;
      readonly mutable?: boolean;
    }

    /**
     * A value accepted or produced by a {@link Global}. Its runtime type is
     * determined by `GlobalDescriptor.value`: numeric globals use `number`,
     * `i64` uses `bigint`, `funcref` uses `null` or a Wasm function, and
     * `externref` accepts and can return every JavaScript value, including
     * `undefined`, strings, booleans, symbols, and objects.
     */
    type GlobalValue = unknown;

    /**
     * A value usable as a Wasm import. The module's declared import type
     * determines the accepted runtime value; an `externref` value can be any
     * JavaScript value.
     */
    type ImportValue = unknown;

    type ModuleImports = Record<string, Record<string, ImportValue>>;

    /**
     * A validated, compiled WebAssembly module. Independent of any
     * particular {@link Instance} and safe to instantiate multiple times,
     * including across realms (each {@link Instance} is still bound to a
     * single realm).
     */
    class Module {
      constructor(bytes: BufferSource);

      /**
       * Returns the imports declared by `moduleObject`, in the module's
       * binary declaration order. A new array is returned on every call.
       */
      static imports(moduleObject: Module): ModuleImportDescriptor[];

      /**
       * Returns the exports declared by `moduleObject`, in the module's
       * binary declaration order. A new array is returned on every call.
       */
      static exports(moduleObject: Module): ModuleExportDescriptor[];

      /**
       * Returns copies (as new `ArrayBuffer`s) of the contents of every
       * custom section named `sectionName` in `moduleObject`.
       */
      static customSections(moduleObject: Module, sectionName: string): ArrayBuffer[];

      readonly [Symbol.toStringTag]: "WebAssembly.Module";
    }

    /**
     * An instantiated {@link Module}, with its imports linked and its
     * `exports` object populated. Only usable from the realm that created
     * it; passing it (or values derived from it) to another realm's
     * WebAssembly APIs throws a {@link LinkError}.
     */
    class Instance {
      constructor(module: Module, importObject?: ModuleImports);

      /**
       * A null-prototype, non-extensible object whose own properties are
       * the module's exports (enumerable, non-writable, non-configurable).
       */
      readonly exports: Record<string, unknown>;

      readonly [Symbol.toStringTag]: "WebAssembly.Instance";
    }

    /**
     * A resizable `ArrayBuffer`-backed linear memory, synchronously
     * mirrored between JS and the underlying `wasmi` store. Only
     * non-shared `wasm32` memories are supported.
     */
    class Memory {
      constructor(descriptor: MemoryDescriptor);

      /**
       * The current memory contents as an `ArrayBuffer`. Stable identity
       * across repeated reads as long as the memory has not grown; a
       * `grow()` (including `grow(0)`) detaches the previous buffer and
       * returns a freshly created one.
       */
      readonly buffer: ArrayBuffer;

      /**
       * Grows the memory by `delta` pages (64 KiB each) and returns the
       * previous size in pages. Throws `RangeError` if the new size would
       * exceed the configured maximum (or the `wasm32` hard limit).
       */
      grow(delta: number): number;

      readonly [Symbol.toStringTag]: "WebAssembly.Memory";
    }

    /**
     * A resizable, typed array of `funcref`/`anyfunc` or `externref`
     * values.
     */
    class Table {
      constructor(descriptor: TableDescriptor, initialValue?: unknown);

      /** The current number of elements in the table. */
      readonly length: number;

      /**
       * Returns the element at `index`: a callable Wasm-function wrapper
       * or `null` for `funcref`/`anyfunc` tables, or the original JS value
       * (by identity) for `externref` tables. Throws `RangeError` if
       * `index` is out of bounds.
       */
      get(index: number): unknown;

      /**
       * Sets the element at `index`. `funcref`/`anyfunc` tables only
       * accept `null` or a Lexe-wrapped Wasm function (an arbitrary JS
       * function is rejected with `TypeError`); `externref` tables accept
       * any JS value. Throws `RangeError` if `index` is out of bounds.
       */
      set(index: number, value: unknown): void;

      /**
       * Grows the table by `delta` elements, optionally filling new slots
       * with `value`, and returns the previous length. Throws `RangeError`
       * if the new length would exceed the configured maximum.
       */
      grow(delta: number, value?: unknown): number;

      readonly [Symbol.toStringTag]: "WebAssembly.Table";
    }

    /** A mutable or immutable Wasm global value, boxed for JS access. */
    class Global {
      constructor(descriptor: GlobalDescriptor, value?: GlobalValue);

      /**
       * The current value of the global, converted per {@link GlobalDescriptor.value}
       * (`i64` globals read/write as `BigInt`). Setting the value of an
       * immutable global throws `TypeError`.
       */
      value: GlobalValue;

      /** Equivalent to reading {@link Global.value}. */
      valueOf(): GlobalValue;

      readonly [Symbol.toStringTag]: "WebAssembly.Global";
    }

    /** Thrown for invalid or unsupported Wasm bytes (malformed modules, disabled proposals, etc.). */
    class CompileError extends Error {
      constructor(message?: string);
    }

    /**
     * Thrown when instantiation fails to resolve an import, or an import's
     * type, realm, or signature does not match the module's declaration.
     */
    class LinkError extends Error {
      constructor(message?: string);
    }

    /** Thrown for Wasm traps, including a trapping `start` function. */
    class RuntimeError extends Error {
      constructor(message?: string);
    }

    /**
     * Synchronously compiles `bytes` (an `ArrayBuffer` or
     * `ArrayBufferView`) into a {@link Module} and returns `true` if
     * compilation and validation succeed, or `false` otherwise. Throws
     * `TypeError` if `bytes` is not a valid `BufferSource`.
     */
    function validate(bytes: BufferSource): boolean;

    /**
     * Asynchronously compiles `bytes` into a {@link Module}. The returned
     * promise resolves with the `Module`, or rejects with a
     * {@link CompileError}.
     */
    function compile(bytes: BufferSource): Promise<Module>;

    /**
     * Asynchronously compiles and instantiates a WebAssembly module from
     * `bytes`. Resolves with `{ module, instance }`.
     */
    function instantiate(
      bytes: BufferSource,
      importObject?: ModuleImports
    ): Promise<{ module: Module; instance: Instance }>;

    /**
     * Asynchronously instantiates an already-compiled `module`. Resolves
     * with the {@link Instance}.
     */
    function instantiate(module: Module, importObject?: ModuleImports): Promise<Instance>;

    /**
     * Streams, compiles `source` (awaiting it if it is a promise, and
     * requiring a same-origin-style `Response` with `ok === true` and an
     * `application/wasm` `Content-Type`). Resolves with the {@link Module}.
     */
    function compileStreaming(source: Response | PromiseLike<Response>): Promise<Module>;

    /**
     * Streams, compiles, and instantiates `source` (same `Response`
     * validation as {@link compileStreaming}). Resolves with
     * `{ module, instance }`.
     */
    function instantiateStreaming(
      source: Response | PromiseLike<Response>,
      importObject?: ModuleImports
    ): Promise<{ module: Module; instance: Instance }>;
  }
}
