# Lexe
<a href="https://hellogithub.com/repository/Ray-D-Song/lexe" target="_blank"><img src="https://abroad.hellogithub.com/v1/widgets/recommend.svg?rid=9a55d6aa58d2467086642f3fdbd2cc1b&claim_uid=4QeJb78I1ZfwtVi&theme=small" alt="Featured｜HelloGitHub" /></a>

Lexe is a lightweight JavaScript runtime built on QuickJS.

It started a year ago as a fork of [LLRT](https://github.com/awslabs/llrt), and was later spun out into an independent project to advance Node.js API compatibility.

It takes Node.js compatibility as its primary goal, aiming to become a lightweight direct drop-in replacement for Node.js. Its compatibility matrix includes Wasm, N-API, V8-API, and standalone single-executable compilation.

Lexe is designed for low-latency, low-footprint backend services and Agent Harnesses.

It's built in Rust, utilizing QuickJS as the JavaScript engine, ensuring efficient memory usage and swift startup.

## Install

```shell
# npm (CLI + TypeScript types)
npm install -g lexe
npx lexe --version
npm install -D lexe-types

# Linux
curl -fsSL https://raw.githubusercontent.com/Ray-D-Song/lexe/main/install-linux.sh | sh

# macOS
curl -fsSL https://raw.githubusercontent.com/Ray-D-Song/lexe/main/install-macos.sh | sh
```

```powershell
# Windows PowerShell
irm https://raw.githubusercontent.com/Ray-D-Song/lexe/main/install-windows.ps1 | iex
```

## Quick start

```shell
printf 'console.log("Hello from Lexe")\n' > main.mjs
lexe ./main.mjs
lexe --version

# Compile to a standalone executable
lexe compile ./main.mjs ./app --executable
./app

# Upgrade Lexe (standalone installs)
lexe upgrade
```

## Documentation

- [Compiling to standalone executables](docs/executable.md)
- [Test runner](docs/test-runner.md)
- [Compatibility matrix](docs/compatibility.md)
- [Using node_modules (dependencies) with Lexe](docs/node-modules.md)
- [Running TypeScript with Lexe](docs/typescript.md)
- [Rationale](docs/rationale.md)
- [Crypto and TLS Backend Options](docs/crypto-tls.md)
- [Environment Variables](docs/environment-variables.md)

See also [API.md](API.md).

## License

This library is licensed under the Apache-2.0 License. See the [LICENSE](LICENSE) file.
