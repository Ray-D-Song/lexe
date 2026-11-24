# raster_runtime

raster_runtime is a lightweight JavaScript runtime built on QuickJS.

It takes Node.js compatibility as its primary goal, aiming to become a lightweight direct drop-in replacement for Node.js. Its compatibility matrix includes Wasm, N-API, and V8-API.

It's built in Rust, utilizing QuickJS as the JavaScript engine, ensuring efficient memory usage and swift startup.

## Install

```shell
# Linux
curl -fsSL https://raw.githubusercontent.com/Ray-D-Song/raster/main/install-linux.sh | sh

# macOS
curl -fsSL https://raw.githubusercontent.com/Ray-D-Song/raster/main/install-macos.sh | sh
```

```powershell
# Windows PowerShell
irm https://raw.githubusercontent.com/Ray-D-Song/raster/main/install-windows.ps1 | iex
```

## Quick start

```shell
printf 'console.log("Hello from Raster")\n' > main.mjs
raster ./main.mjs
raster --version
```

## Documentation

- [Test runner](docs/test-runner.md)
- [Compatibility matrix](docs/compatibility.md)
- [Using node_modules (dependencies) with raster_runtime](docs/node-modules.md)
- [Running TypeScript with raster_runtime](docs/typescript.md)
- [Rationale](docs/rationale.md)
- [Crypto and TLS Backend Options](docs/crypto-tls.md)
- [Environment Variables](docs/environment-variables.md)

See also [API.md](API.md).

## License

This library is licensed under the Apache-2.0 License. See the [LICENSE](LICENSE) file.
