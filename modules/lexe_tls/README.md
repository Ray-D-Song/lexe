# lexe_tls

Node.js `tls` / `node:tls` compatibility module and shared TLS configuration for HTTP/HTTPS/Fetch.

## Node module (`tls`)

Partial support for the Node TLS core API:

- **Client:** `connect()`, `TLSSocket`, `createSecureContext()`, `checkServerIdentity()`
- **Server:** `createServer()`, `Server` (`listen`, `close`, `addContext`, `setSecureContext`)
- **STARTTLS:** `connect({ socket: netSocket, ... })` upgrades an existing TCP `net.Socket` (mysql2 transport path). `net.Socket.setNoDelay` / `setKeepAlive` are implemented on the plain TCP socket; control-handle transfer to `TLSSocket` is not required for the default non-SSL mysql fixture.
- **TLS versions:** TLS 1.2 and 1.3 only
- **Backends:** `tls-ring` (default), `tls-aws-lc`, `tls-graviola`, `tls-openssl`

### Not supported (throws `ERR_TLS_OPTION_NOT_SUPPORTED`)

PFX, custom `ciphers` / `sigalgs`, `dhparam`, CRL, PSK, OCSP, session/ticket resume, async `SNICallback`, `ALPNCallback`, soft mTLS (`requestCert` + `rejectUnauthorized: false`), encrypted private keys on rustls backends (OpenSSL only), and related advanced APIs.

`TLSSocket.write()` always returns `true`; backpressure / `drain` signaling is not implemented yet.

### Internal HTTP/Fetch API

`build_client_config(BuildClientConfigOptions)`, `TLS_CONFIG`, and root-store helpers remain available for `lexe_http` and Fetch without going through the JS module.
