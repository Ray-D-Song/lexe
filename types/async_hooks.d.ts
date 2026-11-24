/**
 * We strongly discourage the use of the `async_hooks` API.
 * Other APIs that can cover most of its use cases include:
 *
 * * [`AsyncLocalStorage`](https://nodejs.org/docs/latest-v22.x/api/async_context.html#class-asynclocalstorage) tracks async context
 * * [`process.getActiveResourcesInfo()`](https://nodejs.org/docs/latest-v22.x/api/process.html#processgetactiveresourcesinfo) tracks active resources
 *
 * The `async_hooks` module provides an API to track asynchronous resources.
 * It can be accessed using:
 *
 * ```js
 * import async_hooks from 'async_hooks';
 * ```
 * @experimental
 * @see [source](https://github.com/nodejs/node/blob/v22.x/lib/async_hooks.js)
 */
declare module "async_hooks" {
  /**
   * ```js
   * import { executionAsyncId } from 'async_hooks';
   * import fs from 'fs';
   *
   * console.log(executionAsyncId());  // 1 - bootstrap
   * const path = '.';
   * fs.open(path, 'r', (err, fd) => {
   *   console.log(executionAsyncId());  // 6 - open()
   * });
   * ```
   *
   * The ID returned from `executionAsyncId()` is related to execution timing, not
   * causality (which is covered by `triggerAsyncId()`):
   *
   * ```js
   * const server = net.createServer((conn) => {
   *   // Returns the ID of the server, not of the new connection, because the
   *   // callback runs in the execution scope of the server's MakeCallback().
   *   async_hooks.executionAsyncId();
   *
   * }).listen(port, () => {
   *   // Returns the ID of a TickObject (process.nextTick()) because all
   *   // callbacks passed to .listen() are wrapped in a nextTick().
   *   async_hooks.executionAsyncId();
   * });
   * ```
   *
   * Promise contexts may not get precise `executionAsyncIds` by default.
   * See the section on [promise execution tracking](https://nodejs.org/docs/latest-v22.x/api/async_hooks.html#promise-execution-tracking).
   * @return The `asyncId` of the current execution context. Useful to track when something calls.
   */
  function executionAsyncId(): number;
  /**
   * Resource objects returned by `executionAsyncResource()` are most often internal
   * lexe handle objects with undocumented APIs. Using any functions or properties
   * on the object is likely to crash your application and should be avoided.
   *
   * Using `executionAsyncResource()` in the top-level execution context will
   * return an empty object as there is no handle or request object to use,
   * but having an object representing the top-level can be helpful.
   *
   * ```js
   * import { open } from 'fs';
   * import { executionAsyncId, executionAsyncResource } from 'async_hooks';
   *
   * console.log(executionAsyncId(), executionAsyncResource());  // 1 {}
   * open(new URL(import.meta.url), 'r', (err, fd) => {
   *   console.log(executionAsyncId(), executionAsyncResource());  // 7 FSReqWrap
   * });
   * ```
   *
   * This can be used to implement continuation local storage without the
   * use of a tracking `Map` to store the metadata:
   *
   * ```js
   * import { createServer } from 'http';
   * import {
   *   executionAsyncId,
   *   executionAsyncResource,
   *   createHook,
   * } from 'async_hooks';
   * const sym = Symbol('state'); // Private symbol to avoid pollution
   *
   * createHook({
   *   init(asyncId, type, triggerAsyncId, resource) {
   *     const cr = executionAsyncResource();
   *     if (cr) {
   *       resource[sym] = cr[sym];
   *     }
   *   },
   * }).enable();
   *
   * const server = createServer((req, res) => {
   *   executionAsyncResource()[sym] = { state: req.url };
   *   setTimeout(function() {
   *     res.end(JSON.stringify(executionAsyncResource()[sym]));
   *   }, 100);
   * }).listen(3000);
   * ```
   * @return The resource representing the current execution. Useful to store data within the resource.
   */
  function executionAsyncResource(): object;
  /**
   * ```js
   * const server = net.createServer((conn) => {
   *   // The resource that caused (or triggered) this callback to be called
   *   // was that of the new connection. Thus the return value of triggerAsyncId()
   *   // is the asyncId of "conn".
   *   async_hooks.triggerAsyncId();
   *
   * }).listen(port, () => {
   *   // Even though all callbacks passed to .listen() are wrapped in a nextTick()
   *   // the callback itself exists because the call to the server's .listen()
   *   // was made. So the return value would be the ID of the server.
   *   async_hooks.triggerAsyncId();
   * });
   * ```
   *
   * Promise contexts may not get valid `triggerAsyncId`s by default. See
   * the section on [promise execution tracking](https://nodejs.org/docs/latest-v22.x/api/async_hooks.html#promise-execution-tracking).
   * @return The ID of the resource responsible for calling the callback that is currently being executed.
   */
  function triggerAsyncId(): number;
  interface HookCallbacks {
    /**
     * Called when a class is constructed that has the possibility to emit an asynchronous event.
     * @param asyncId A unique ID for the async resource
     * @param type The type of the async resource
     * @param triggerAsyncId The unique ID of the async resource in whose execution context this async resource was created
     * @param resource Reference to the resource representing the async operation, needs to be released during destroy
     */
    init?(
      asyncId: number,
      type: string,
      triggerAsyncId: number,
      resource: object
    ): void;
    /**
     * When an asynchronous operation is initiated or completes a callback is called to notify the user.
     * The before callback is called just before said callback is executed.
     * @param asyncId the unique identifier assigned to the resource about to execute the callback.
     */
    before?(asyncId: number): void;
    /**
     * Called immediately after the callback specified in `before` is completed.
     *
     * If an uncaught exception occurs during execution of the callback, then `after` will run after the `'uncaughtException'` event is emitted or a `domain`'s handler runs.
     * @param asyncId the unique identifier assigned to the resource which has executed the callback.
     */
    after?(asyncId: number): void;
    /**
     * Called when a promise has resolve() called. This may not be in the same execution id
     * as the promise itself.
     * @param asyncId the unique id for the promise that was resolve()d.
     */
    promiseResolve?(asyncId: number): void;
    /**
     * Called after the resource corresponding to asyncId is destroyed
     * @param asyncId a unique ID for the async resource
     */
    destroy?(asyncId: number): void;
  }
  interface AsyncHook {
    /**
     * Enable the callbacks for a given AsyncHook instance. If no callbacks are provided enabling is a noop.
     */
    enable(): this;
    /**
     * Disable the callbacks for a given AsyncHook instance from the global pool of AsyncHook callbacks to be executed. Once a hook has been disabled it will not be called again until enabled.
     */
    disable(): this;
  }
  /**
   * Registers functions to be called for different lifetime events of each async
   * operation.
   *
   * The callbacks `init()`/`before()`/`after()`/`destroy()` are called for the
   * respective asynchronous event during a resource's lifetime.
   *
   * All callbacks are optional. For example, if only resource cleanup needs to
   * be tracked, then only the `destroy` callback needs to be passed. The
   * specifics of all functions that can be passed to `callbacks` is in the `Hook Callbacks` section.
   *
   * ```js
   * import { createHook } from 'async_hooks';
   *
   * const asyncHook = createHook({
   *   init(asyncId, type, triggerAsyncId, resource) { },
   *   destroy(asyncId) { },
   * });
   * ```
   *
   * The callbacks will be inherited via the prototype chain:
   *
   * ```js
   * class MyAsyncCallbacks {
   *   init(asyncId, type, triggerAsyncId, resource) { }
   *   destroy(asyncId) {}
   * }
   *
   * class MyAddedCallbacks extends MyAsyncCallbacks {
   *   before(asyncId) { }
   *   after(asyncId) { }
   * }
   *
   * const asyncHook = async_hooks.createHook(new MyAddedCallbacks());
   * ```
   *
   * Because promises are asynchronous resources whose lifecycle is tracked
   * via the async hooks mechanism, the `init()`, `before()`, `after()`, and`destroy()` callbacks _must not_ be async functions that return promises.
   * @param callbacks The `Hook Callbacks` to register
   * @return Instance used for disabling and enabling hooks
   */
  function createHook(callbacks: HookCallbacks): AsyncHook;

  interface AsyncResourceOptions {
    /**
     * The ID of the execution context that created this async event.
     * @default executionAsyncId()
     */
    triggerAsyncId?: number;
    /**
     * Disables automatic `emitDestroy` when the object is garbage collected.
     * This usually does not need to be set (even if `emitDestroy` is called
     * manually), unless the resource's `asyncId` is retrieved and the
     * sensitive API's `emitDestroy` is called with it.
     * @default false
     */
    requireManualDestroy?: boolean;
  }

  /**
   * The class `AsyncResource` is designed to be extended by the embedder's async
   * resources. Using this, users can easily trigger the lifetime events of their
   * own resources.
   */
  class AsyncResource {
    /**
     * AsyncResource() is meant to be extended. Instantiating a
     * new AsyncResource() also triggers init. If triggerAsyncId is omitted then
     * async_hook.executionAsyncId() is used.
     * @param type The type of async event.
     * @param triggerAsyncId | AsyncResourceOptions The ID of the execution context
     *   that created this async event (default: `executionAsyncId()`), or an
     *   AsyncResourceOptions object.
     */
    constructor(type: string, options?: AsyncResourceOptions);

    /**
     * Call the provided function with the provided arguments in the execution context
     * of the async resource. This will establish the context, trigger the AsyncHooks
     * before callbacks, call the function, trigger the AsyncHooks after callbacks, and
     * then restore the original execution context.
     * @param fn The function to call in the execution context of this async resource.
     * @param thisArg The receiver to be used for the function call.
     * @param args Optional arguments to pass to the function.
     */
    runInAsyncScope<This, Args extends unknown[], Result>(
      fn: (this: This, ...args: Args) => Result,
      thisArg?: This,
      ...args: Args
    ): Result;

    /**
     * Call all `destroy` hooks for this resource and return `this`.
     *
     * Explicit `emitDestroy()` may be called more than once; each call dispatches
     * destroy hooks again (compatible with Node 24).
     *
     * When constructed with `requireManualDestroy: false` (the default) and
     * destroy hooks are enabled, a `FinalizationRegistry` entry may also dispatch
     * destroy when the instance is garbage-collected. After an explicit
     * `emitDestroy()`, that automatic GC path is suppressed so destroy is not
     * double-fired from finalization.
     *
     * @return A reference to `asyncResource`.
     */
    emitDestroy(): this;

    /**
     * @return The unique `asyncId` assigned to the resource.
     */
    asyncId(): number;

    /**
     * @return The same `triggerAsyncId` that is passed to the `AsyncResource` constructor.
     */
    triggerAsyncId(): number;

    /**
     * Binds the given function to execute to this `AsyncResource`'s scope.
     * @param fn The function to bind to the current `AsyncResource`.
     * @param thisArg Optional receiver to be used for the function call.
     */
    bind<Func extends (...args: any[]) => any>(
      fn: Func,
      thisArg?: ThisParameterType<Func>
    ): OmitThisParameter<Func>;

    /**
     * Binds the given function to execute to a new `AsyncResource`'s scope.
     * @param fn The function to bind to a new `AsyncResource`.
     * @param type An optional name to associate with the underlying `AsyncResource`.
     * @param thisArg Optional receiver to be used for the function call.
     */
    static bind<Func extends (...args: any[]) => any>(
      fn: Func,
      type?: string,
      thisArg?: ThisParameterType<Func>
    ): OmitThisParameter<Func>;
  }

  /**
   * This class creates stores that stay coherent through asynchronous operations.
   *
   * Lexe propagates stores across Promises, `await`, microtasks, and timers when
   * an `AsyncLocalStorage` instance is active. Public `createHook()` callbacks still
   * require `LEXE_ASYNC_HOOKS=1`.
   *
   * @see [source](https://nodejs.org/docs/latest-v22.x/api/async_context.html#class-asynclocalstorage)
   */
  class AsyncLocalStorage<T = any> {
    /**
     * Runs a function synchronously within a context and returns its return value.
     * The store is not accessible outside of the callback function.
     */
    run<R>(store: T, callback: (...args: any[]) => R, ...args: any[]): R;
    /**
     * Transitions into the context for the remainder of the current synchronous
     * execution and then persists the store through any following asynchronous calls.
     */
    enterWith(store: T): void;
    /**
     * Runs a function synchronously outside of a context and returns its return value.
     */
    exit<R>(callback: (...args: any[]) => R, ...args: any[]): R;
    /**
     * Returns the current store. If called outside of an asynchronous context
     * initialized by calling `asyncLocalStorage.run()` or
     * `asyncLocalStorage.enterWith()`, it returns `undefined`.
     */
    getStore(): T | undefined;
    /**
     * Disables the instance. Subsequent `getStore()` calls return `undefined`
     * until `run()` or `enterWith()` is called again.
     */
    disable(): void;
    /**
     * Binds the given function to run within a snapshot of all active
     * `AsyncLocalStorage` stores (Node static API).
     */
    static bind<Func extends (...args: any[]) => any>(fn: Func): Func;
    /**
     * Captures the current stores of all active `AsyncLocalStorage` instances
     * (including instances with no store) and returns a function that runs a
     * callback under that captured context.
     */
    static snapshot(): <R, TArgs extends any[]>(
      fn: (...args: TArgs) => R,
      ...args: TArgs
    ) => R;
  }
}
declare module "node:async_hooks" {
  export * from "async_hooks";
}
