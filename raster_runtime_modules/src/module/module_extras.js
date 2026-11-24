// Source maps, package.json lookup and compile-cache entry points of `node:module`.
//
// References (refs/node/lib):
//   internal/source_map/source_map.js, internal/source_map/source_map_cache.js
//   internal/modules/package_json_reader.js (findPackageJSON)
//   internal/modules/helpers.js (enableCompileCache and friends)
//
// Evaluated once per context; the returned object's properties are attached to
// the `Module` constructor and exported from `node:module`.
(function () {
  "use strict";

  const load = (name) => globalThis.require(name);

  function makeError(Base, code, message) {
    const error = new Base(message);
    Object.defineProperty(error, "code", {
      value: code,
      enumerable: true,
      configurable: true,
      writable: true,
    });
    return error;
  }

  function describeReceived(value) {
    if (value === undefined) return "undefined";
    if (value === null) return "null";
    if (typeof value === "function") {
      return value.name ? `function ${value.name}` : "function ";
    }
    if (typeof value === "object") {
      const name = value.constructor && value.constructor.name;
      return name ? `an instance of ${name}` : "[Object: null prototype]";
    }
    let shown = String(value);
    if (typeof value === "string") {
      if (shown.length > 28) shown = `${shown.slice(0, 25)}...`;
      shown = `'${shown}'`;
    }
    return `type ${typeof value} (${shown})`;
  }

  function invalidArgType(name, expected, value) {
    return makeError(
      TypeError,
      "ERR_INVALID_ARG_TYPE",
      `The "${name}" argument must be of type ${expected}. Received ${describeReceived(value)}`
    );
  }

  function validateObject(value, name) {
    if (value === null || Array.isArray(value) || typeof value !== "object") {
      throw invalidArgType(name, "object", value);
    }
  }

  function validateString(value, name) {
    if (typeof value !== "string") throw invalidArgType(name, "string", value);
  }

  function validateBoolean(value, name) {
    if (typeof value !== "boolean") throw invalidArgType(name, "boolean", value);
  }

  // ---------------------------------------------------------------- SourceMap

  let base64Map;
  const VLQ_BASE_SHIFT = 5;
  const VLQ_BASE_MASK = (1 << 5) - 1;
  const VLQ_CONTINUATION_MASK = 1 << 5;

  class StringCharIterator {
    constructor(string) {
      this._string = string;
      this._position = 0;
    }
    next() {
      return this._string.charAt(this._position++);
    }
    peek() {
      return this._string.charAt(this._position);
    }
    hasNext() {
      return this._position < this._string.length;
    }
  }

  function isSeparator(char) {
    return char === "," || char === ";";
  }

  function decodeVLQ(iterator) {
    let result = 0;
    let shift = 0;
    let digit;
    do {
      digit = base64Map[iterator.next()];
      result += (digit & VLQ_BASE_MASK) << shift;
      shift += VLQ_BASE_SHIFT;
    } while (digit & VLQ_CONTINUATION_MASK);

    const negative = result & 1;
    result >>>= 1;
    if (!negative) return result;
    return -result | (1 << 31);
  }

  function cloneSourceMapV3(payload) {
    validateObject(payload, "payload");
    payload = { ...payload };
    for (const key in payload) {
      if (Object.prototype.hasOwnProperty.call(payload, key) && Array.isArray(payload[key])) {
        payload[key] = payload[key].slice();
      }
    }
    return payload;
  }

  function compareSourceMapEntry(entry1, entry2) {
    const { 0: line1, 1: column1 } = entry1;
    const { 0: line2, 1: column2 } = entry2;
    if (line1 !== line2) return line1 - line2;
    return column1 - column2;
  }

  class SourceMap {
    #payload;
    #mappings = [];
    #sources = {};
    #sourceContentByURL = {};
    #lineLengths = undefined;

    constructor(payload, { lineLengths } = { __proto__: null }) {
      if (!base64Map) {
        const digits = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        base64Map = {};
        for (let i = 0; i < digits.length; ++i) base64Map[digits[i]] = i;
      }
      this.#payload = cloneSourceMapV3(payload);
      this.#parseMappingPayload();
      if (Array.isArray(lineLengths) && lineLengths.length) {
        this.#lineLengths = lineLengths;
      }
    }

    get payload() {
      return cloneSourceMapV3(this.#payload);
    }

    get lineLengths() {
      if (this.#lineLengths) return this.#lineLengths.slice();
      return undefined;
    }

    #parseMappingPayload() {
      if (this.#payload.sections) {
        this.#parseSections(this.#payload.sections);
      } else {
        this.#parseMap(this.#payload, 0, 0);
      }
      this.#mappings.sort(compareSourceMapEntry);
    }

    #parseSections(sections) {
      for (let i = 0; i < sections.length; ++i) {
        const section = sections[i];
        this.#parseMap(section.map, section.offset.line, section.offset.column);
      }
    }

    findEntry(lineOffset, columnOffset) {
      let first = 0;
      let count = this.#mappings.length;
      while (count > 1) {
        const step = count >> 1;
        const middle = first + step;
        const mapping = this.#mappings[middle];
        if (
          lineOffset < mapping[0] ||
          (lineOffset === mapping[0] && columnOffset < mapping[1])
        ) {
          count = step;
        } else {
          first = middle;
          count -= step;
        }
      }
      const entry = this.#mappings[first];
      if (
        !first &&
        entry &&
        (lineOffset < entry[0] || (lineOffset === entry[0] && columnOffset < entry[1]))
      ) {
        return {};
      } else if (!entry) {
        return {};
      }
      return {
        generatedLine: entry[0],
        generatedColumn: entry[1],
        originalSource: entry[2],
        originalLine: entry[3],
        originalColumn: entry[4],
        name: entry[5],
      };
    }

    findOrigin(lineNumber, columnNumber) {
      const range = this.findEntry(lineNumber - 1, columnNumber - 1);
      if (
        range.originalSource === undefined ||
        range.originalLine === undefined ||
        range.originalColumn === undefined ||
        range.generatedLine === undefined ||
        range.generatedColumn === undefined
      ) {
        return {};
      }
      const lineOffset = lineNumber - range.generatedLine;
      const columnOffset = columnNumber - range.generatedColumn;
      return {
        name: range.name,
        fileName: range.originalSource,
        lineNumber: range.originalLine + lineOffset,
        columnNumber: range.originalColumn + columnOffset,
      };
    }

    #parseMap(map, lineNumber, columnNumber) {
      let sourceIndex = 0;
      let sourceLineNumber = 0;
      let sourceColumnNumber = 0;
      let nameIndex = 0;

      const sources = [];
      for (let i = 0; i < map.sources.length; ++i) {
        const url = map.sources[i];
        sources.push(url);
        this.#sources[url] = true;
        if (map.sourcesContent?.[i]) this.#sourceContentByURL[url] = map.sourcesContent[i];
      }

      const iterator = new StringCharIterator(map.mappings);
      let sourceURL = sources[sourceIndex];
      while (true) {
        if (iterator.peek() === ",") {
          iterator.next();
        } else {
          while (iterator.peek() === ";") {
            lineNumber += 1;
            columnNumber = 0;
            iterator.next();
          }
          if (!iterator.hasNext()) break;
        }

        columnNumber += decodeVLQ(iterator);
        if (isSeparator(iterator.peek())) {
          this.#mappings.push([lineNumber, columnNumber]);
          continue;
        }

        const sourceIndexDelta = decodeVLQ(iterator);
        if (sourceIndexDelta) {
          sourceIndex += sourceIndexDelta;
          sourceURL = sources[sourceIndex];
        }
        sourceLineNumber += decodeVLQ(iterator);
        sourceColumnNumber += decodeVLQ(iterator);

        let name;
        if (!isSeparator(iterator.peek())) {
          nameIndex += decodeVLQ(iterator);
          name = map.names?.[nameIndex];
        }

        this.#mappings.push([
          lineNumber,
          columnNumber,
          sourceURL,
          sourceLineNumber,
          sourceColumnNumber,
          name,
        ]);
      }
    }
  }

  // ------------------------------------------------------- source map support

  const kLeadingProtocol = /^\w+:\/\//;
  const kSourceMappingURLMagicComment = /\/[*/]#\s+sourceMappingURL=(?<sourceMappingURL>[^\s]+)/g;
  const kNodeModulesRE = /^(?:.*)[\\/]node_modules[\\/]/;

  let sourceMapsSupport = Object.freeze({
    __proto__: null,
    enabled: false,
    nodeModules: false,
    generatedCode: false,
  });

  function getSourceMapsSupport() {
    return sourceMapsSupport;
  }

  // Raster does not rewrite `Error.prepareStackTrace`; this flag only governs
  // what `findSourceMap` returns.
  function setSourceMapsSupport(enabled, options = {}) {
    validateBoolean(enabled, "enabled");
    validateObject(options, "options");
    const { nodeModules = false, generatedCode = false } = options;
    validateBoolean(nodeModules, "options.nodeModules");
    validateBoolean(generatedCode, "options.generatedCode");
    sourceMapsSupport = Object.freeze({
      __proto__: null,
      enabled,
      nodeModules,
      generatedCode,
    });
  }

  function isUnderNodeModules(filename) {
    return Boolean(filename) && kNodeModulesRE.exec(filename) !== null;
  }

  function extractSourceMapURLMagicComment(content) {
    let match;
    let lastMatch;
    kSourceMappingURLMagicComment.lastIndex = 0;
    while ((match = kSourceMappingURLMagicComment.exec(content))) lastMatch = match;
    if (lastMatch == null) return null;
    return lastMatch.groups.sourceMappingURL;
  }

  function lineLengths(content) {
    const output = [];
    let lineLength = 0;
    for (let i = 0; i < content.length; i++, lineLength++) {
      const codePoint = content.codePointAt(i);
      if (codePoint === 10 || codePoint === 0x2028 || codePoint === 0x2029) {
        output.push(lineLength);
        lineLength = -1;
      }
    }
    output.push(lineLength);
    return output;
  }

  function sourcesToAbsolute(baseURL, data) {
    const { isAbsolute } = load("node:path");
    const { pathToFileURL } = load("node:url");
    data.sources = data.sources.map((source) => {
      source = (data.sourceRoot || "") + source;
      if (isAbsolute(source)) return pathToFileURL(source).href;
      return new URL(source, baseURL).href;
    });
    data.sourceRoot = "";
    return data;
  }

  function sourceMapFromFile(mapURL) {
    try {
      const { fileURLToPath } = load("node:url");
      const content = load("node:fs").readFileSync(fileURLToPath(mapURL), "utf8");
      return sourcesToAbsolute(mapURL, JSON.parse(content));
    } catch {
      return null;
    }
  }

  function sourceMapFromDataUrl(sourceURL, url) {
    const { 0: format, 1: data } = url.split(",", 2);
    const splitFormat = format.split(";");
    const contentType = splitFormat[0];
    const base64 = splitFormat[splitFormat.length - 1] === "base64";
    if (contentType !== "application/json") return null;
    try {
      const decoded = base64 ? Buffer.from(data, "base64").toString("utf8") : data;
      return sourcesToAbsolute(sourceURL, JSON.parse(decoded));
    } catch {
      return null;
    }
  }

  function dataFromUrl(sourceURL, sourceMappingURL) {
    let url = null;
    try {
      url = new URL(sourceMappingURL);
    } catch {
      url = null;
    }
    if (url != null) {
      if (url.protocol === "data:") return sourceMapFromDataUrl(sourceURL, url.pathname);
      return null;
    }
    return sourceMapFromFile(new URL(sourceMappingURL, sourceURL).href);
  }

  const sourceMapCache = new Map();

  // Node fills its cache from the CJS/ESM loaders while support is enabled.
  // Raster's loaders do not feed it, so a lookup that misses reads the file on
  // demand, and only while support is enabled.
  function cacheFromDisk(sourceURL) {
    const { fileURLToPath } = load("node:url");
    let content;
    try {
      content = load("node:fs").readFileSync(fileURLToPath(sourceURL), "utf8");
    } catch {
      return undefined;
    }
    const sourceMapURL = extractSourceMapURLMagicComment(content);
    if (typeof sourceMapURL !== "string") return undefined;
    const data = dataFromUrl(sourceURL, sourceMapURL);
    const entry = {
      __proto__: null,
      lineLengths: lineLengths(content),
      data,
      sourceMapURL: data ? null : sourceMapURL,
      sourceURL: undefined,
    };
    sourceMapCache.set(sourceURL, entry);
    return entry;
  }

  function findSourceMap(sourceURL) {
    if (typeof sourceURL !== "string") return undefined;
    if (sourceURL.startsWith("node:")) return undefined;

    const support = getSourceMapsSupport();
    if (!support.nodeModules && isUnderNodeModules(sourceURL)) return undefined;

    try {
      if (kLeadingProtocol.exec(sourceURL) === null) {
        sourceURL = load("node:url").pathToFileURL(sourceURL).href;
      }
      let entry = sourceMapCache.get(sourceURL);
      if (entry === undefined && support.enabled && sourceURL.startsWith("file:")) {
        entry = cacheFromDisk(sourceURL);
      }
      if (entry?.data == null) return undefined;

      let sourceMap = entry.sourceMap;
      if (sourceMap === undefined) {
        sourceMap = new SourceMap(entry.data, { lineLengths: entry.lineLengths });
        entry.sourceMap = sourceMap;
      }
      return sourceMap;
    } catch {
      return undefined;
    }
  }

  // --------------------------------------------------------- findPackageJSON

  const invalidPackageNameRegEx = /^\.|%|\\/;

  function fileURLToPathStrict(url) {
    if (!`${url}`.startsWith("file:")) {
      throw makeError(TypeError, "ERR_INVALID_URL_SCHEME", "The URL must be of scheme file");
    }
    return load("node:url").fileURLToPath(url);
  }

  function parsePackageName(specifier, base) {
    let separatorIndex = specifier.indexOf("/");
    let validPackageName = true;
    let isScoped = false;
    if (specifier[0] === "@") {
      isScoped = true;
      if (separatorIndex === -1 || specifier.length === 0) {
        validPackageName = false;
      } else {
        separatorIndex = specifier.indexOf("/", separatorIndex + 1);
      }
    }

    const packageName = separatorIndex === -1 ? specifier : specifier.slice(0, separatorIndex);
    if (invalidPackageNameRegEx.exec(packageName) !== null) validPackageName = false;

    if (!validPackageName) {
      throw makeError(
        TypeError,
        "ERR_INVALID_MODULE_SPECIFIER",
        `Invalid module "${specifier}" is not a valid package name imported from ${fileURLToPathStrict(base)}`
      );
    }
    return { packageName, isScoped };
  }

  function readPackageJSON(packageJSONPath) {
    const fs = load("node:fs");
    let text;
    try {
      text = fs.readFileSync(packageJSONPath, "utf8");
    } catch {
      return undefined;
    }
    try {
      return JSON.parse(text);
    } catch (error) {
      throw makeError(
        Error,
        "ERR_INVALID_PACKAGE_CONFIG",
        `Invalid package config ${packageJSONPath}.`
      );
    }
  }

  // Mirrors `TraverseParent` in refs/node/src/node_modules.cc: start at the
  // parent of `checkPath`, stop below a `node_modules` directory and at the root.
  function getNearestParentPackageJSON(checkPath) {
    const path = load("node:path");
    let current = checkPath.endsWith("/") ? checkPath.slice(0, -1) : path.dirname(checkPath);
    for (;;) {
      if (path.dirname(current) === current) return undefined;
      if (current.replace(/\\/g, "/").endsWith("/node_modules")) return undefined;
      const candidate = path.join(current, "package.json");
      const data = readPackageJSON(candidate);
      if (data !== undefined) return { path: candidate, data };
      current = path.dirname(current);
    }
  }

  function getPackageJSONPathForBare(specifier, parentURL) {
    const path = load("node:path");
    const fs = load("node:fs");
    const { packageName, isScoped } = parsePackageName(specifier, parentURL);

    const scope = getNearestParentPackageJSON(fileURLToPathStrict(parentURL));
    if (scope && scope.data.exports != null && scope.data.name === packageName) {
      return scope.path;
    }

    let dir = path.dirname(fileURLToPathStrict(parentURL));
    for (;;) {
      const packageDir = path.join(dir, "node_modules", packageName);
      let isDir = false;
      try {
        isDir = fs.statSync(packageDir).isDirectory();
      } catch {
        isDir = false;
      }
      if (isDir) return path.join(packageDir, "package.json");
      const up = path.dirname(dir);
      if (up === dir) break;
      dir = up;
    }
    throw makeError(
      Error,
      "ERR_MODULE_NOT_FOUND",
      `Cannot find package '${packageName}' imported from ${fileURLToPathStrict(parentURL)}`
    );
  }

  function isURLObject(value) {
    return typeof value === "object" && value !== null && typeof value.href === "string" && typeof value.protocol === "string";
  }

  function canParseURL(value) {
    try {
      new URL(value);
      return true;
    } catch {
      return false;
    }
  }

  function findPackageJSON(specifier, base = "data:") {
    if (arguments.length === 0) {
      throw makeError(TypeError, "ERR_MISSING_ARGS", 'The "specifier" argument must be specified');
    }
    try {
      specifier = `${specifier}`;
    } catch {
      validateString(specifier, "specifier");
    }

    const path = load("node:path");
    const { pathToFileURL } = load("node:url");
    let parentURL = base;
    if (!isURLObject(base)) {
      validateString(base, "base");
      if (path.isAbsolute(base)) {
        parentURL = pathToFileURL(base);
      } else {
        try {
          parentURL = new URL(base);
        } catch {
          throw makeError(TypeError, "ERR_INVALID_URL", "Invalid URL");
        }
      }
    }

    if (specifier && specifier[0] !== "." && specifier[0] !== "/" && !canParseURL(specifier)) {
      return getPackageJSONPathForBare(specifier, parentURL);
    }

    if (specifier === "") {
      let from;
      try {
        from = fileURLToPathStrict(parentURL);
      } catch {
        from = `${parentURL}`;
      }
      throw makeError(Error, "ERR_MODULE_NOT_FOUND", `Cannot find package '' imported from ${from}`);
    }

    let resolved;
    try {
      resolved = new URL(specifier, `${parentURL}`);
    } catch {
      throw makeError(
        TypeError,
        "ERR_UNSUPPORTED_RESOLVE_REQUEST",
        `Failed to resolve module specifier "${specifier}" from "${parentURL}": Invalid relative URL or base scheme is not hierarchical.`
      );
    }

    const resolvedPath = fileURLToPathStrict(resolved);
    // A trailing slash names a directory and skips the existence check.
    if (!resolvedPath.endsWith("/")) {
      let stat;
      try {
        stat = load("node:fs").statSync(resolvedPath);
      } catch {
        stat = undefined;
      }
      if (stat === undefined) {
        let from;
        try {
          from = fileURLToPathStrict(parentURL);
        } catch {
          from = `${parentURL}`;
        }
        throw makeError(
          Error,
          "ERR_MODULE_NOT_FOUND",
          `Cannot find module '${resolvedPath}' imported from ${from}`
        );
      }
    }

    // Node 24.21 returns the resolved path when no package.json is found,
    // contradicting its docs and lib source (which return undefined).
    const pkg = getNearestParentPackageJSON(resolvedPath);
    return pkg?.path ?? resolvedPath;
  }

  // ------------------------------------------------------------ compile cache
  //
  // Node's compile cache stores V8 code-cache blobs on disk. QuickJS has no
  // equivalent for source modules, and the feature has no observable effect
  // other than start-up time, so the entry points validate their arguments like
  // Node and report that the cache could not be enabled.

  const compileCacheStatus = Object.freeze({
    __proto__: null,
    FAILED: 0,
    ENABLED: 1,
    ALREADY_ENABLED: 2,
    DISABLED: 3,
  });
  const constants = Object.freeze({ __proto__: null, compileCacheStatus });

  function enableCompileCache(cacheDir) {
    if (cacheDir !== undefined && typeof cacheDir !== "string") {
      throw makeError(TypeError, "ERR_INVALID_ARG_TYPE", "cacheDir should be a string");
    }
    return {
      status: compileCacheStatus.FAILED,
      message: "The on-disk compile cache is not supported by this runtime",
    };
  }

  function flushCompileCache() {
    const keepDeserializedCache = arguments[0];
    if (keepDeserializedCache !== undefined && typeof keepDeserializedCache !== "boolean") {
      throw makeError(
        TypeError,
        "ERR_INVALID_ARG_TYPE",
        "keepDeserializedCache should be a boolean"
      );
    }
  }

  function getCompileCacheDir() {
    return undefined;
  }

  return {
    SourceMap,
    findSourceMap,
    getSourceMapsSupport,
    setSourceMapsSupport,
    findPackageJSON,
    enableCompileCache,
    flushCompileCache,
    getCompileCacheDir,
    constants,
  };
})()
