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

Installing via npm installs the matching native `lexe` binary. On Linux, npm picks one of the `@lexe/linux-<arch>-<libc>` optional subpackages: `gnu` (glibc, supports native `.node` addons) or `musl` (static, no native addons). On macOS and Windows the binary is fetched during install. Each download is SHA256-verified against the GitHub Release. If the pre-download during install is skipped or offline, Lexe downloads and caches the binary on first invocation.

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
- `LEXE_LIBC`: Set to `gnu` or `musl` to override Linux libc detection when choosing a binary.

## Documentation

Full documentation is available in the [Lexe repository](https://github.com/Ray-D-Song/lexe#readme).
