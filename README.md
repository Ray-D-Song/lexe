# Lexe
Lexe is a fork of AWS's lightweight JavaScript runtime "LLRT".  
You can use it to package your Node.js applications into a single executable file, but the size is only 5~8MB.

```bash
npx lexe build -i index.js

# -i, --input         input file(required)
#
# -o, --output        output file name(optional, default: <input file name>-<platform>)
#
# -d, --directory     output directory(optional, default: ./dist)
#
# -p, --platform      target platform, use "," to separate multiple platforms
#                     options: linux-x64,linux-arm64,darwin-x64,darwin-arm64,windows-x64,windows-arm64
#                     (optional, default: current platform)
```

> [!WARNING]
> Lexe is not a drop-in replacement for Node.js. It only supports a subset of Node.js APIs.  
> You can read more about LLRT in [LLRT README](https://github.com/awslabs/llrt)  
> Since Lexe is a fork of LLRT, the following document is basically a copy of LLRT

## Compatibility matrix

| Modules        | Node.js | LLRT ⚠️ |
| -------------- | ------- | ------- |
| assert         | ✔︎     | ✔︎️    |
| buffer         | ✔︎     | ✔︎️    |
| child_process  | ✔︎     | ✔︎⏱   |
| console        | ✔︎     | ✔︎     |
| crypto         | ✔︎     | ✔︎     |
| dns            | ✔︎     | ✔︎     |
| events         | ✔︎     | ✔︎     |
| fs/promises    | ✔︎     | ✔︎     |
| fs             | ✔︎     | ✘⏱     |
| http           | ✔︎     | ✘⏱\*\* |
| https          | ✔︎     | ✘⏱\*\* |
| net:sockets    | ✔︎     | ✔︎⏱   |
| net:server     | ✔︎     | ✔︎     |
| os             | ✔︎     | ✔︎     |
| path           | ✔︎     | ✔︎     |
| perf_hooks     | ✔︎     | ✔︎     |
| process        | ✔︎     | ✔︎     |
| streams        | ✔︎     | ✔︎\*   |
| string_decoder | ✔︎     | ✔︎     |
| timers         | ✔︎     | ✔︎     |
| tty            | ✔︎     | ✔︎     |
| url            | ✔︎     | ✔︎     |
| util           | ✔︎     | ✔︎     |
| tls            | ✔︎     | ✘⏱     |
| zlib           | ✔︎     | ✔︎     |
| Other modules  | ✔︎     | ✘       |

| Features    | Node.js | LLRT ⚠️ |
| ----------- | ------- | ------- |
| async/await | ✔︎     | ✔︎     |
| encoding    | ✔︎     | ✔︎     |
| fetch       | ✔︎     | ✔︎     |
| ESM         | ✔︎     | ✔︎     |
| CJS         | ✔︎     | ✔︎     |

_⚠️ = partially supported in LLRT_<br />
_⏱ = planned partial support_<br />
_\* = Not native_<br />
_\*\* = Use fetch instead_<br />

## Using node_modules (dependencies) with llrt

Since llrt is meant for performance critical application it's not recommended to deploy `node_modules` without bundling, minification and tree-shaking.

llrt can work with any bundler of your choice. Below are some configurations for popular bundlers:

> [!WARNING]
> LLRT implements native modules that are largely compatible with the following external packages.
> By implementing the following conversions in the bundler's alias function, your application may be faster, but we recommend that you test thoroughly as they are not fully compatible.

| Node.js         | LLRT      |
| --------------- | --------- |
| fast-xml-parser | llrt:xml  |
| uuid            | llrt:uuid |

### ESBuild

```shell
esbuild index.js --platform=browser --target=es2023 --format=esm --bundle --minify --external:@aws-sdk --external:@smithy
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
  external: ["@aws-sdk", "@smithy"],
};
```

### Webpack

```javascript
import TerserPlugin from "terser-webpack-plugin";
import nodeExternals from "webpack-node-externals";

export default {
  entry: "./index.js",
  output: {
    path: "dist",
    filename: "bundle.js",
    libraryTarget: "module",
  },
  target: "web",
  mode: "production",
  resolve: {
    extensions: [".js"],
  },
  externals: [nodeExternals(), "@aws-sdk", "@smithy"],
  optimization: {
    minimize: true,
    minimizer: [
      new TerserPlugin({
        terserOptions: {
          ecma: 2023,
        },
      }),
    ],
  },
};
```
## Running TypeScript with LLRT

Same principle as dependencies applies when using TypeScript. TypeScript must be bundled and transpiled into ES2023 JavaScript.

> [!NOTE]
> LLRT will not support running TypeScript without transpilation. This is by design for performance reasons. Transpiling requires CPU and memory that adds latency and cost during execution. This can be avoided if done ahead of time during deployment.

## Rationale

What justifies the introduction of another JavaScript runtime in light of existing options such as [Node.js](https://nodejs.org/en), [Bun](https://bun.sh) & [Deno](https://deno.com/)?

Node.js, Bun, and Deno represent highly proficient JavaScript runtimes. However, they are designed with general-purpose applications in mind. These runtimes were not specifically tailored for the demands of a Serverless environment, characterized by short-lived runtime instances. They each depend on a ([Just-In-Time compiler (JIT)](https://en.wikipedia.org/wiki/Just-in-time_compilation) for dynamic code compilation and optimization during execution. While JIT compilation offers substantial long-term performance advantages, it carries a computational and memory overhead.

In contrast, LLRT distinguishes itself by not incorporating a JIT compiler, a strategic decision that yields two significant advantages:

A) JIT compilation is a notably sophisticated technological component, introducing increased system complexity and contributing substantially to the runtime's overall size.

B) Without the JIT overhead, LLRT conserves both CPU and memory resources that can be more efficiently allocated to code execution tasks, thereby reducing application startup times.

## Limitations

There are many cases where LLRT shows notable performance drawbacks compared with JIT-powered runtimes, such as large data processing, Monte Carlo simulations or performing tasks with hundreds of thousands or millions of iterations. LLRT is most effective when applied to smaller Serverless functions dedicated to tasks such as data transformation, real time processing, AWS service integrations, authorization, validation etc. It is designed to complement existing components rather than serve as a comprehensive replacement for everything. Notably, given its supported APIs are based on Node.js specification, transitioning back to alternative solutions requires minimal code adjustments.

## Environment Variables

### `LLRT_EXTRA_CA_CERTS=file`

Load extra certificate authorities from a PEM encoded file

### `LLRT_GC_THRESHOLD_MB=value`

Set a memory threshold in MB for garbage collection. Default threshold is 20MB

### `LLRT_HTTP_VERSION=value`

Extends the HTTP request version. By default, only HTTP/1.1 is enabled. Specifying '2' will enable HTTP/1.1 and HTTP/2.

### `LLRT_LOG=[target][=][level][,...]`

Filter the log output by target module, level, or both (using `=`). Log levels are case-insensitive and will also enable any higher priority logs.

Log levels in descending priority order:

- `Error`
- `Warn | Warning`
- `Info`
- `Debug`
- `Trace`

Example filters:

- `warn` will enable all warning and error logs
- `llrt_core::vm=trace` will enable all logs in the `llrt_core::vm` module
- `warn,llrt_core::vm=trace` will enable all logs in the `llrt_core::vm` module and all warning and error logs in other modules

### `LLRT_NET_ALLOW="host[ ...]"`

Space-delimited list of hosts or socket paths which should be allowed for network connections. Network connections will be denied for any host or socket path missing from this list. Set an empty list to deny all connections

### `LLRT_NET_DENY="host[ ...]"`

Space-delimited list of hosts or socket paths which should be denied for network connections

### `LLRT_NET_POOL_IDLE_TIMEOUT=value`

Set a timeout in seconds for idle sockets being kept-alive. Default timeout is 15 seconds

### `LLRT_PLATFORM=value`

Used to explicitly specify a preferred platform for the Node.js package resolver. The default is `browser`. If `node` is specified, "node" takes precedence in the search path. If a value other than `browser` or `node` is specified, it will behave as if "browser" was specified.

### `LLRT_TLS_VERSION=value`

Set the TLS version to be used for network connections. By default only TLS 1.2 is enabled. TLS 1.3 can also be enabled by setting this variable to `1.3`

## Benchmark Methodology

Although Init Duration [reported by Lambda](https://docs.aws.amazon.com/lambda/latest/dg/lambda-runtime-environment.html) is commonly used to understand cold start impact on overall request latency, this metric does not include the time needed to copy code into the Lambda sandbox.

The technical definition of Init Duration ([source](https://docs.aws.amazon.com/lambda/latest/dg/nodejs-logging.html#node-logging-output)):

> For the first request served, the amount of time it took the runtime to load the function and run code outside of the handler method.

Measuring round-trip request duration provides a more complete picture of user facing cold-start latency.

Lambda invocation results (λ-labeled row) report the sum total of Init Duration + Function Duration.

## License

This library is licensed under the Apache-2.0 License. See the [LICENSE](LICENSE) file.
