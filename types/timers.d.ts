declare module "timers" {
  export import setTimeout = globalThis.setTimeout;
  export import clearTimeout = globalThis.clearTimeout;
  export import setInterval = globalThis.setInterval;
  export import clearInterval = globalThis.clearInterval;
  export import setImmediate = globalThis.setImmediate;
  export import clearImmediate = globalThis.clearImmediate;

  global {
    /**
     * This object is created internally and is returned from `setTimeout()` and `setInterval()`. It can be passed to either `clearTimeout()` or `clearInterval()` in order to cancel the
     * scheduled actions.
     *
     * Lexe returns a `Timeout` handle object (Node-compatible). Use `ref()` / `unref()` to
     * control whether the timer alone keeps the process alive; `{ ref: false }` on
     * `timers/promises` schedules an unref'd timer.
     */
    class Timeout {
      ref(): this;
      unref(): this;
      hasRef(): boolean;
    }

    /**
     * Schedules execution of a one-time `callback` after `delay` milliseconds.
     *
     * Additional arguments are passed to the callback when it is invoked.
     *
     * @param callback The function to call when the timer elapses.
     * @param [delay=0] The number of milliseconds to wait before calling the `callback`.
     * @param args Optional arguments to pass when the `callback` is called.
     * @return for use with {@link clearTimeout}
     */
    function setTimeout<TArgs extends any[]>(
      callback: (...args: TArgs) => void,
      ms?: number,
      ...args: TArgs
    ): Timeout;

    /**
     * Cancels a `Timeout` object created by `setTimeout()`.
     * @param timeout A `Timeout` object as returned by {@link setTimeout}.
     */
    function clearTimeout(timeout?: Timeout | number | null): void;

    /**
     * Schedules repeated execution of `callback` every `delay` milliseconds.
     *
     * Additional arguments are passed to the callback when it is invoked.
     *
     * @param callback The function to call when the timer elapses.
     * @param [delay=0] The number of milliseconds to wait before calling the `callback`.
     * @param args Optional arguments to pass when the `callback` is called.
     * @return for use with {@link clearInterval}
     */
    function setInterval<TArgs extends any[]>(
      callback: (...args: TArgs) => void,
      ms?: number,
      ...args: TArgs
    ): Timeout;

    /**
     * Cancels a `Timeout` object created by `setInterval()`.
     * @param interval A `Timeout` object as returned by {@link setInterval}
     */
    function clearInterval(interval?: Timeout | number | null): void;

    /**
     * Schedules the "immediate" execution of the `callback` after I/O events'
     * callbacks.
     *
     * Additional arguments are passed to the callback when it is invoked.
     * `util.promisify(setImmediate)` is supported via `promisify.custom`.
     *
     * @param callback The function to call at the end of this turn of the Node.js `Event Loop`
     * @param args Optional arguments to pass when the `callback` is called.
     * @return for use with {@link clearImmediate}
     */
    function setImmediate<TArgs extends any[]>(
      callback: (...args: TArgs) => void,
      ...args: TArgs
    ): Timeout;

    /**
     * Cancels an Immediate object created by {@link setImmediate}.
     */
    function clearImmediate(immediate?: Timeout | number | null): void;
  }
}

declare module "node:timers" {
  export * from "timers";
}
