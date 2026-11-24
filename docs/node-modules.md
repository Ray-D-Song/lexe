# Using node_modules (dependencies) with raster_runtime

Since raster_runtime is meant for performance critical application it's not recommended to deploy `node_modules` without bundling, minification and tree-shaking.

raster_runtime can work with any bundler of your choice. Below are some configurations for popular bundlers:

> [!WARNING]
> raster_runtime implements native modules that are largely compatible with the following external packages.
> By implementing the following conversions in the bundler's alias function, your application may be faster, but we recommend that you test thoroughly as they are not fully compatible.

| Node.js         | raster_runtime     |
| --------------- | ------------------ |
| fast-xml-parser | raster_runtime:xml |

### ESBuild

```shell
esbuild index.js --platform=browser --target=es2023 --format=esm --bundle --minify
```

### Rollup

```javascript
import resolve from "@rollup/plugin-node-resolve";
import commonjs from "@rollup/plugin-commonjs";
import terser from "@rollup/plugin-terser";

export default {
  input: "index.js",
  output: {
    file: "dist/bundle.js",
    format: "esm",
    sourcemap: true,
    target: "es2023",
  },
  plugins: [resolve(), commonjs(), terser()],
};
```
