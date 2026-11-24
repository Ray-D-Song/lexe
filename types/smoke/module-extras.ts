import Module, {
  SourceMap,
  constants,
  enableCompileCache,
  findPackageJSON,
  findSourceMap,
  flushCompileCache,
  getCompileCacheDir,
  getSourceMapsSupport,
  setSourceMapsSupport,
  syncBuiltinESMExports,
} from "node:module";

const map: SourceMap | undefined = findSourceMap("/app/a.js");
map?.findOrigin(1, 1);
const entry = new SourceMap({
  file: "out.js",
  version: 3,
  sources: ["a.ts"],
  sourcesContent: ["let a = 1;"],
  names: [],
  mappings: "",
  sourceRoot: "",
}).findEntry(0, 0);
void entry;

const pkg: string | undefined = findPackageJSON("dep", "file:///app/index.js");
void pkg;

const result = enableCompileCache();
const failed: boolean = result.status === constants.compileCacheStatus.FAILED;
void failed;
flushCompileCache();
const dir: string | undefined = getCompileCacheDir();
void dir;

setSourceMapsSupport(true, { nodeModules: false });
const support = getSourceMapsSupport();
const enabled: boolean = support.enabled;
void enabled;

syncBuiltinESMExports();
Module.syncBuiltinESMExports();
