/**
 * The `node:vm` module exposes APIs for executing JavaScript in isolated contexts.
 *
 * Lexe implements a subset focused on synchronous `runInNewContext` compatibility.
 *
 * @see [source](https://github.com/nodejs/node/blob/v22.x/lib/vm.js)
 */
declare module "vm" {
  export interface RunInNewContextOptions {
    filename?: string | undefined;
  }

  /**
   * Runs the supplied code in a new isolated context and returns the result.
   *
   * Limitations:
   * - Only `filename` is supported in the options object.
   * - Non-enumerable properties, symbol properties, and full property descriptor
   *   forwarding are not supported when synchronizing the sandbox.
   */
  export function runInNewContext(
    code: string,
    contextObject?: object | undefined,
    options?: string | RunInNewContextOptions | undefined
  ): unknown;

  const vm: {
    runInNewContext: typeof runInNewContext;
  };

  export default vm;
}

declare module "node:vm" {
  export * from "vm";
  export { default } from "vm";
}
