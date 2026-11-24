# Compiling to Standalone Executables

Lexe can compile JavaScript modules to compressed QuickJS bytecode (`.lrt`) or package them directly into self-contained native executables with no external runtime dependencies.

## Bytecode Compilation (`.lrt`)

Compile a JavaScript entry file to zstd-compressed QuickJS bytecode:

```shell
lexe compile app.js
# produces app.lrt

lexe compile app.js build/app.lrt
```

The resulting `.lrt` file can be run directly by the matching `lexe` version:

```shell
lexe app.lrt
```

## Single-Executable Compilation (`--executable`)

### Current Platform

Pass `--executable` to embed the compiled bytecode directly into a copy of the Lexe runtime:

```shell
lexe compile app.js app --executable
./app --help
```

All command-line arguments (including `--version` and `--help`) are forwarded straight to your bundled script via `process.argv`.

### Cross-Platform Compilation (`-p` / `--platform`)

Lexe supports packaging standalone executables for 6 target platforms:

- `linux-x64`
- `linux-arm64`
- `darwin-x64` (alias: `macos-x64`)
- `darwin-arm64` (alias: `macos-arm64`)
- `windows-x64`
- `windows-arm64`

Example targeting multiple platforms at once:

```shell
lexe compile app.js --executable -o my-cli -d ./dist -p linux-x64,darwin-arm64,windows-x64
```

This produces:

- `./dist/my-cli-linux-x64`
- `./dist/my-cli-darwin-arm64`
- `./dist/my-cli-windows-x64.exe`

### Options

- `--executable`: Embed compiled bytecode into a standalone executable instead of emitting `.lrt`.
- `-o, --output <name>`: Output file path (single-platform) or base filename (multi-platform).
- `-d, --outdir <dir>`: Output directory (defaults to `./dist` when `-p` is used).
- `-p, --platform <list>`: Comma-separated list of target platforms.
- `--runtime-dir <dir>`: Local directory containing prebuilt `lexe` binaries for target platforms (can also be set via `LEXE_BIN_DIR`).

## Runtime Template Resolution & Auto-Download

When compiling for the current platform without `--runtime-dir` or `LEXE_BIN_DIR`, `lexe` uses its own executable as the template binary (zero network access required).

When cross-compiling for another platform (or when `--runtime-dir` / `LEXE_BIN_DIR` is specified), `lexe` looks for the target template binary in:

1. `--runtime-dir`
2. `LEXE_BIN_DIR`
3. The directory (and parent directory) of the current `lexe` binary
4. The local cache directory (`$LEXE_CACHE_DIR/v<VERSION>`, or `$XDG_CACHE_HOME/lexe/v<VERSION>`, `~/.cache/lexe/v<VERSION>`, `%LOCALAPPDATA%\lexe\cache\v<VERSION>`)

If the template binary is not found locally, `lexe` automatically downloads `SHA256SUMS` and the platform asset (`lexe-<os>-<arch>[.exe]`) from GitHub Releases (`$LEXE_RELEASE_BASE_URL` or `https://github.com/Ray-D-Song/lexe/releases/download/v<VERSION>`), verifies the SHA-256 checksum before saving, and caches it for subsequent builds.

## How It Works

1. **Prebuilt compression dictionary**: Lexe ships a pre-generated zstd dictionary (`prebuild/compression.dict`) built into every platform binary, so bytecode compressed on one host decompresses identically on every target platform.
2. **Native binary section injection (`libsui`)**:
   - **Linux (ELF)**: Embedded as a `.note.sui` ELF note inside a mapped `PT_LOAD` / `PT_NOTE` segment.
   - **macOS (Mach-O)**: Written as a dedicated Mach-O section and automatically ad-hoc code-signed on Apple Silicon (and appended safely on Intel macOS) via `libsui::Macho::build_and_sign`.
   - **Windows (PE)**: Embedded as an `RT_RCDATA` resource directory entry via `libsui::PortableExecutable`.

## Multi-File Projects & `node_modules`

`lexe compile` compiles a single JavaScript file. If your application imports local files, TypeScript sources, or npm dependencies, bundle them into a single ES2023 JavaScript file first (see [Using node_modules](node-modules.md) and [Running TypeScript](typescript.md)):

```shell
npx esbuild src/index.ts --platform=node --target=es2023 --format=esm --bundle --minify --outfile=dist/bundle.mjs
lexe compile dist/bundle.mjs --executable -o my-cli -d ./dist -p linux-x64,darwin-arm64,windows-x64
```
