# Linux runtime subpackages

Each directory is a separate npm package holding one statically or dynamically linked Linux `lexe` binary.
The `os`, `cpu` and `libc` fields let npm install only the matching variant:

| Package | libc | Linking | N-API native addons |
| --- | --- | --- | --- |
| `@lexe/linux-x64-gnu` | glibc (>= 2.28) | dynamic | supported |
| `@lexe/linux-arm64-gnu` | glibc (>= 2.28) | dynamic | supported |
| `@lexe/linux-x64-musl` | musl | static | not supported |
| `@lexe/linux-arm64-musl` | musl | static | not supported |

`bin/lexe` is populated by `npm/scripts/stage-linux-packages.mjs` in the release workflow and is never committed.
