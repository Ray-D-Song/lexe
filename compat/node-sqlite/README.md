# node-sqlite compatibility parity

Structured differential tests for `node:sqlite` parity between Node 24.3 and Lexe.

## Usage

```bash
make compat-node-sqlite
```

Or manually:

```bash
cd compat/node-sqlite
make
SQLITE_EXTENSION_PATH=$PWD/build/sqlite_extension.dylib node parity.mjs
```

The parity probe prints one JSON document to stdout. `compat/run.mjs` runs the same script under Node 24.3 and Lexe, normalizes volatile fields, and compares the results.

## ASan gate (Linux)

```bash
make compat-node-sqlite-asan
```

Runs AddressSanitizer builds with reduced stability loops and sqlite unit tests.
