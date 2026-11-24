# Changelog

All notable changes to Lexe are documented in this file.

## [Unreleased]

- Replaced the legacy LLRT runtime core with the upgraded QuickJS-based Node.js-compatible runtime core (supporting `node:http`, `node:https`, `node:http2`, `node:tls`, `node:sqlite`, `node:worker_threads`, N-API, V8 ABI 137 shim, and WebAssembly).
- Rebranded all crates, built-in modules (`lexe:*`), environment variables (`LEXE_*`), scripts, CI workflows, and documentation to `lexe`.
- Unified single-executable compilation under `lexe compile --executable` using `libsui` across 6 platforms (`linux-x64`, `linux-arm64`, `darwin-x64`, `darwin-arm64`, `windows-x64`, `windows-arm64`), a prebuilt zstd dictionary (`prebuild/compression.dict`), and built-in GitHub Release auto-download with `SHA256SUMS` verification.

## [0.1.0] - 2026-08-09

Initial release.
