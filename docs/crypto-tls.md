# Crypto and TLS Backend Options

Lexe supports multiple cryptographic backends for both the crypto module and TLS connections. These can be configured via Cargo features.

### Crypto Provider Features

| Feature                 | Description                                                       |
| ----------------------- | ----------------------------------------------------------------- |
| `crypto-rust` (default) | Pure Rust crypto using RustCrypto crates                          |
| `crypto-ring`           | Ring-only crypto (limited algorithm support)                      |
| `crypto-ring-rust`      | Ring for hashing/HMAC, RustCrypto for everything else             |
| `crypto-graviola`       | Graviola-only crypto (limited algorithm support)                  |
| `crypto-graviola-rust`  | Graviola for hashing/HMAC/AES-GCM, RustCrypto for everything else |
| `crypto-openssl`        | OpenSSL-based crypto                                              |

### TLS Backend Features

| Feature              | Description                 |
| -------------------- | --------------------------- |
| `tls-ring` (default) | rustls with ring crypto     |
| `tls-graviola`       | rustls with graviola crypto |
| `tls-openssl`        | OpenSSL for TLS             |

`node:tls` is implemented as a **partial** Node core subset (client/server, `TLSSocket`, `SecureContext`, STARTTLS handoff for `net.Socket`, TLS 1.2/1.3, ALPN, SNI, mTLS). Not supported: session resume/tickets, PFX, custom ciphers/sigalgs, OCSP, renegotiation, PSK, and other advanced options (these throw `ERR_TLS_OPTION_NOT_SUPPORTED`). HTTP/HTTPS/Fetch continue to use the internal `BuildClientConfigOptions` / `TLS_CONFIG` Rust API unchanged.

### TLS Root Certificate Features

| Feature                  | Description                                                              |
| ------------------------ | ------------------------------------------------------------------------ |
| `native-roots` (default) | Load CA certificates from the host OS trust store (`rustls-native-certs`) |
| `webpki-roots`           | Bundle Mozilla's root CA store (`webpki-roots`) for minimal/musl images  |

### Building with Different Backends

```bash
# Default (crypto-rust + tls-ring)
cargo build --release

# Using OpenSSL for both crypto and TLS
cargo build --release --no-default-features --features "macro,crypto-openssl,tls-openssl"

# Using Graviola for both crypto and TLS
cargo build --release --no-default-features --features "macro,crypto-graviola-rust,tls-graviola"
```
