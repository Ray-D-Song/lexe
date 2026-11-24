Pre-generated zstd compression dictionary used across platforms when compiling and decompressing QuickJS bytecode modules.

Keeping this dictionary checked into the repository guarantees that bytecode compiled on one OS/architecture (via `lexe compile` or `lexe compile --executable -p ...`) decompresses reliably on every target runtime binary without dictionary mismatch errors.

To regenerate `compression.dict` after significant changes to built-in JavaScript modules in `lexe_core/src/modules/js`, run:

```sh
make update-dict
```
