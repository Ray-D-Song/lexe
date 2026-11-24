# lexe

Lightweight JavaScript runtime built on QuickJS with Node.js compatibility, Wasm/N-API/V8-API support, and single-executable compilation.

## Install

```shell
npm install -g lexe
```

Or run directly with `npx`:

```shell
npx lexe --version
```

Installing via npm automatically fetches and SHA256-verifies the matching native `lexe` release binary (`linux-x64`, `linux-arm64`, `macos-x64`, `macos-arm64`, `windows-x64`, `windows-arm64`) from [GitHub Releases](https://github.com/Ray-D-Song/lexe/releases). If the pre-download during install is skipped or offline, Lexe downloads and caches the binary on first invocation.

## Quick start

```shell
printf 'console.log("Hello from Lexe")\n' > main.mjs
lexe ./main.mjs

# Compile to a standalone single executable
lexe compile ./main.mjs ./app --executable
./app
```

## TypeScript definitions

Install [`lexe-types`](https://www.npmjs.com/package/lexe-types) for IDE autocomplete and type checking:

```shell
npm install -D lexe-types
```

## Environment variables

- `LEXE_BINARY_PATH`: Use an explicit local `lexe` binary path instead of downloading from GitHub Releases.
- `LEXE_SKIP_DOWNLOAD`: Set to `1` or `true` to skip pre-downloading the binary during `npm install`.
- `LEXE_CACHE_DIR`: Override the cache directory where downloaded release binaries are stored.
- `LEXE_RELEASE_BASE_URL`: Override the base download URL for release binaries and `SHA256SUMS`.

## Documentation

Full documentation is available in the [Lexe repository](https://github.com/Ray-D-Song/lexe#readme).
