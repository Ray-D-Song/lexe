/**
 * @since v0.3.7
 */
declare module "module" {
  import { URL } from "node:url";
  class Module {
    constructor(id: string, parent?: Module);
    id: string;
    filename: string;
    path: string;
    paths: string[];
    exports: any;
    loaded: boolean;
    parent: Module | null;
    children: Module[];
    require(id: string): any;
    _compile(content: string, filename: string): void;
  }
  interface Module extends NodeJS.Module {}
  namespace Module {
    export { Module };
  }
  namespace Module {
    /**
     * A list of the names of all modules provided by Node.js. Can be used to verify
     * if a module is maintained by a third party or not.
     *
     * Note: the list doesn't contain prefix-only modules like `node:test`.
     */
    const builtinModules: readonly string[];
    /**
     * @param path Filename to be used to construct the require
     * function. Must be a file URL object, file URL string, or absolute path
     * string.
     */
    function createRequire(path: string | URL): NodeJS.Require;
    /**
     */
    function isBuiltin(moduleName: string): boolean;
    /**
     * Copy the current properties of each loaded built-in's `module.exports` back
     * into its ES module named exports. Names that were deleted become
     * `undefined`; names that were added are not exported.
     */
    function syncBuiltinESMExports(): void;
    interface SourceMapPayload {
      file: string;
      version: number;
      sources: string[];
      sourcesContent: string[];
      names: string[];
      mappings: string;
      sourceRoot: string;
    }
    interface SourceMapping {
      generatedLine: number;
      generatedColumn: number;
      originalSource: string;
      originalLine: number;
      originalColumn: number;
      name?: string;
    }
    interface SourceOrigin {
      name?: string;
      fileName: string;
      lineNumber: number;
      columnNumber: number;
    }
    /**
     * A Source Map V3 consumer.
     */
    class SourceMap {
      constructor(payload: SourceMapPayload, options?: { lineLengths?: number[] });
      /** A copy of the payload the map was built from. */
      readonly payload: SourceMapPayload;
      /** Line lengths of the generated source, when they were supplied. */
      readonly lineLengths: number[] | undefined;
      /**
       * Find the mapping that covers a 0-indexed generated position, or `{}`.
       */
      findEntry(lineOffset: number, columnOffset: number): SourceMapping | {};
      /**
       * Find the original position of a 1-indexed generated call site, or `{}`.
       */
      findOrigin(lineNumber: number, columnNumber: number): SourceOrigin | {};
    }
    /**
     * The source map of a file, when source map support is enabled and the file
     * declares one. Never throws.
     * @param path A file path or URL.
     */
    function findSourceMap(path: string): SourceMap | undefined;
    interface SourceMapsSupport {
      readonly enabled: boolean;
      readonly nodeModules: boolean;
      readonly generatedCode: boolean;
    }
    function getSourceMapsSupport(): SourceMapsSupport;
    /**
     * Governs what {@link findSourceMap} returns. Raster does not rewrite
     * `Error.prepareStackTrace`.
     */
    function setSourceMapsSupport(
      enabled: boolean,
      options?: { nodeModules?: boolean | undefined; generatedCode?: boolean | undefined }
    ): void;
    /**
     * The path of the `package.json` that governs `specifier`.
     * @param specifier A bare package name, or a relative or `file:` specifier.
     * @param base The URL or absolute path to resolve against. Defaults to `data:`.
     */
    function findPackageJSON(specifier: string | URL, base?: string | URL): string | undefined;
    interface EnableCompileCacheResult {
      status: number;
      message?: string;
      directory?: string;
    }
    /**
     * Raster has no on-disk compile cache: this validates its argument and
     * reports `constants.compileCacheStatus.FAILED`.
     */
    function enableCompileCache(cacheDir?: string): EnableCompileCacheResult;
    function flushCompileCache(keepDeserializedCache?: boolean): void;
    /** Always `undefined`: the compile cache is never enabled. */
    function getCompileCacheDir(): string | undefined;
    const constants: {
      readonly compileCacheStatus: {
        readonly FAILED: 0;
        readonly ENABLED: 1;
        readonly ALREADY_ENABLED: 2;
        readonly DISABLED: 3;
      };
    };
    interface RegisterOptions<Data> {
      /**
       * If you want to resolve `specifier` relative to a
       * base URL, such as `import.meta.url`, you can pass that URL here. This
       * property is ignored if the `parentURL` is supplied as the second argument.
       * @default 'data:'
       */
      parentURL?: string | URL | undefined;
      /**
       * Any arbitrary, cloneable JavaScript value to pass into the
       * {@link initialize} hook.
       */
      data?: Data | undefined;
      /**
       * [Transferable objects](https://nodejs.org/docs/latest-v24.x/api/worker_threads.html#portpostmessagevalue-transferlist)
       * to be passed into the `initialize` hook.
       */
      transferList?: any[] | undefined;
    }
    interface RegisterHooksOptions {
      /**
       * See [load hook](https://nodejs.org/docs/latest-v24.x/api/module.html#loadurl-context-nextload).
       * @default undefined
       */
      load?: LoadHookSync | undefined;
      /**
       * See [resolve hook](https://nodejs.org/docs/latest-v24.x/api/module.html#resolvespecifier-context-nextresolve).
       * @default undefined
       */
      resolve?: ResolveHookSync | undefined;
    }
    interface ModuleHooks {
      /**
       * Deregister the hook instance.
       */
      deregister(): void;
    }
    /**
     * Register [hooks](https://nodejs.org/docs/latest-v24.x/api/module.html#customization-hooks)
     * that customize Node.js module resolution and loading behavior.
     * @experimental
     */
    function registerHooks(options: RegisterHooksOptions): ModuleHooks;
    /**
     * @internal CommonJS loader compatibility — writable for require hooks.
     */
    function _resolveFilename(
      request: string,
      parent?: Module | null,
      isMain?: boolean,
      options?: { paths?: string[] }
    ): string;
    /**
     * @internal CommonJS loader compatibility.
     */
    function _nodeModulePaths(from: string): string[];
    /**
     * @internal Shared with `require.cache`.
     */
    const _cache: NodeJS.RequireCache;
    /**
     * @internal Shared with `require.extensions`.
     */
    const _extensions: NodeJS.RequireExtensions;
    interface ImportAttributes extends NodeJS.Dict<string> {
      type?: string | undefined;
    }
    type ImportPhase = "source" | "evaluation";
    type ModuleFormat =
      | "addon"
      | "builtin"
      | "commonjs"
      | "commonjs-typescript"
      | "json"
      | "module"
      | "module-typescript"
      | "wasm";
    type ModuleSource = string | ArrayBuffer | NodeJS.TypedArray;
    interface ResolveHookContext {
      /**
       * Export conditions of the relevant `package.json`
       */
      conditions: string[];
      /**
       *  An object whose key-value pairs represent the assertions for the module to import
       */
      importAttributes: ImportAttributes;
      /**
       * The module importing this one, or undefined if this is the Node.js entry point
       */
      parentURL: string | undefined;
    }
    interface ResolveFnOutput {
      /**
       * A hint to the load hook (it might be ignored); can be an intermediary value.
       */
      format?: string | null | undefined;
      /**
       * The import attributes to use when caching the module (optional; if excluded the input will be used)
       */
      importAttributes?: ImportAttributes | undefined;
      /**
       * A signal that this hook intends to terminate the chain of `resolve` hooks.
       * @default false
       */
      shortCircuit?: boolean | undefined;
      /**
       * The absolute URL to which this input resolves
       */
      url: string;
    }
    /**
     * The `resolve` hook chain is responsible for telling Node.js where to find and
     * how to cache a given `import` statement or expression, or `require` call. It can
     * optionally return a format (such as `'module'`) as a hint to the `load` hook. If
     * a format is specified, the `load` hook is ultimately responsible for providing
     * the final `format` value (and it is free to ignore the hint provided by
     * `resolve`); if `resolve` provides a `format`, a custom `load` hook is required
     * even if only to pass the value to the Node.js default `load` hook.
     */
    type ResolveHook = (
      specifier: string,
      context: ResolveHookContext,
      nextResolve: (
        specifier: string,
        context?: Partial<ResolveHookContext>
      ) => ResolveFnOutput | Promise<ResolveFnOutput>
    ) => ResolveFnOutput | Promise<ResolveFnOutput>;
    type ResolveHookSync = (
      specifier: string,
      context: ResolveHookContext,
      nextResolve: (
        specifier: string,
        context?: Partial<ResolveHookContext>
      ) => ResolveFnOutput
    ) => ResolveFnOutput;
    interface LoadHookContext {
      /**
       * Export conditions of the relevant `package.json`
       */
      conditions: string[];
      /**
       * The format optionally supplied by the `resolve` hook chain (can be an intermediary value).
       */
      format: string | null | undefined;
      /**
       *  An object whose key-value pairs represent the assertions for the module to import
       */
      importAttributes: ImportAttributes;
    }
    interface LoadFnOutput {
      format: string | null | undefined;
      /**
       * A signal that this hook intends to terminate the chain of `resolve` hooks.
       * @default false
       */
      shortCircuit?: boolean | undefined;
      /**
       * The source for Node.js to evaluate
       */
      source?: ModuleSource | undefined;
    }
    /**
     * The `load` hook provides a way to define a custom method of determining how a
     * URL should be interpreted, retrieved, and parsed. It is also in charge of
     * validating the import attributes.
     */
    type LoadHook = (
      url: string,
      context: LoadHookContext,
      nextLoad: (
        url: string,
        context?: Partial<LoadHookContext>
      ) => LoadFnOutput | Promise<LoadFnOutput>
    ) => LoadFnOutput | Promise<LoadFnOutput>;
    type LoadHookSync = (
      url: string,
      context: LoadHookContext,
      nextLoad: (
        url: string,
        context?: Partial<LoadHookContext>
      ) => LoadFnOutput
    ) => LoadFnOutput;
  }
  global {
    interface ImportMeta {
      /**
       * The absolute `file:` URL of the module.
       *
       * This is defined exactly the same as it is in browsers providing the URL of the
       * current module file.
       *
       * This enables useful patterns such as relative file loading:
       *
       * ```js
       * import { readFileSync } from 'node:fs';
       * const buffer = readFileSync(new URL('./data.proto', import.meta.url));
       * ```
       */
      url: string;
    }
    namespace NodeJS {
      interface Module {
        /**
         * The `module.exports` object is created by the `Module` system. Sometimes this is
         * not acceptable; many want their module to be an instance of some class. To do
         * this, assign the desired export object to `module.exports`.
         */
        exports: any;
        /**
         * The directory name of the module. This is usually the same as the
         * `path.dirname()` of the `module.id`.
         */
        path: string;
        /**
         * The `module.require()` method provides a way to load a module as if
         * `require()` was called from the original module.
         */
        require(id: string): any;
      }
      interface Require {
        /**
         * Shared module cache view; deleting a key forces the next `require()` to reload.
         */
        cache: RequireCache;
        /**
         * CommonJS extension handlers keyed by extension (for example `.js`, `.json`).
         * Mutations are visible to all `require` functions, including `createRequire()`.
         */
        extensions: RequireExtensions;
        /**
         */
        resolve: RequireResolve;
      }
      interface RequireExtensions {
        [ext: string]: (module: Module, filename: string) => void;
      }
      interface RequireCache {
        [path: string]: Module;
      }
      interface RequireResolveOptions {
        /**
         * Paths to resolve module location from. If present, these
         * paths are used instead of the default resolution paths, with the exception
         * of
         * [GLOBAL\_FOLDERS](https://nodejs.org/docs/latest-v24.x/api/modules.html#loading-from-the-global-folders)
         * like `$HOME/.node_modules`, which are
         * always included. Each of these paths is used as a starting point for
         * the module resolution algorithm, meaning that the `node_modules` hierarchy
         * is checked from this location.
         */
        paths?: string[] | undefined;
      }
      interface RequireResolve {
        /**
         * Use the internal `require()` machinery to look up the location of a module,
         * but rather than loading the module, just return the resolved filename.
         *
         * If the module can not be found, a `MODULE_NOT_FOUND` error is thrown.
         * @param request The module path to resolve.
         */
        (request: string, options?: RequireResolveOptions): string;
        /**
         * Returns an array containing the paths searched during resolution of `request` or
         * `null` if the `request` string references a core module, for example `http` or
         * `fs`.
         * @param request The module path whose lookup paths are being retrieved.
         */
        paths(request: string): string[] | null;
      }
    }
    /**
     * The `exports` variable is available within a module's file-level scope, and is
     * assigned the value of `module.exports` before the module is evaluated.
     */
    var exports: NodeJS.Module["exports"];
    /**
     * A reference to the current module.
     * @since v0.1.16
     */
    var module: NodeJS.Module;
    /**
     * @since v0.1.13
     */
    var require: NodeJS.Require;
  }
  export = Module;
}
declare module "node:module" {
  import module = require("module");
  export = module;
}
