# Compatibility fixtures

Each fixture installs its own locked dependencies. **Vite+** still runs the upstream CLI under Lexe and inspects build output without executing it. **Next** uses system Node to produce a standalone deployment, then runs that server under Lexe and asserts real HTTP responses.

| Case                               | Versions                   | Flow                                                                                                                                                              | Status                                                                                                                                                                                                                                                                                                                                                |
| ---------------------------------- | -------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Next App Router standalone runtime | Next 16.2.10, React 19.2.5 | Node `next build` (`output: "standalone"`) → Lexe runs `.next/standalone/server.js` → HTTP checks on `/`, `/api/health`, `/posts/42`, concurrent `/api/als/:id` | **Green** for the documented standalone-runtime fixture, including the inspector startup probe, timers/promises, and AsyncLocalStorage propagation/isolation without `LEXE_ASYNC_HOOKS`. `worker_threads.Worker` uses a child `lexe` process (see PR #32). Deferred: Inspector Session/protocol, timer ref/unref, timers/promises setInterval/scheduler. CI uses Node 22.18.0; only the server process runs under Lexe. |
| Vite+ React library build          | Vite+ 0.2.5, React 19.2.5  | Lexe runs `vp build` (requires `cargo build --features napi`) → verifies `dist/index.js`, `index.cjs`, CSS, manifest                                                                                                   | **Green** with N-API + readline/TTY/stdio. Install with Node **22.18.0** (or ≥24.11); Lexe process identity is **24.3.0**. See `make compat-vite-plus`.                                                                                                                                                                                          |
| better-sqlite3 sync API            | better-sqlite3 11.9.1      | Node `test.cjs` baseline → Lexe runs `test.cjs` → stdout contains `better-sqlite3 compat OK`                                                                    | **Green** with `cargo build --features v8-compat` (Node 24.3.0 / ABI 137 V8 shim). Requires `make compat-better-sqlite3`.                                                                                                                                                                                                                             |
| node:sqlite built-in               | Node 24.3.0 / SQLite 3.50.1 | Node and Lexe run the same structured parity probe, including lifecycle and backup stability loops                                                             | **Green** on Linux and macOS against the Node 24.3 baseline; Windows runs Lexe-only parity. ASan is a separate blocking gate. Requires `make compat-node-sqlite`.                                                                                                                                                                                    |
| v8-hello native addon              | node-addon-api 8.3.1       | `npm run build` (node-gyp) → Node `test.cjs` baseline → Lexe runs `test.cjs` → stdout contains `v8-hello compat OK`                                             | **Green** with `cargo build --features v8-compat`. Requires `make compat-v8`.                                                                                                                                                                                                                                                                         |
| N-API hello addon                  | node-addon-api 8.3.1       | `yarn build` (node-gyp) → Node `test.cjs` baseline → Lexe (`--features napi`) runs `test.cjs` → stdout contains `napi-hello compat OK`                          | **Green** when built with `cargo build --features napi` on a dynamically linked target (macOS / glibc). Requires `make compat-napi`.                                                                                                                                                                                                                  |
| mysql2 async API                   | mysql2 3.23.2, MySQL 8.4   | Docker `mysql:8.4` temp container → Node `test.cjs` baseline → Lexe runs `test.cjs` → stdout contains `mysql2 compat OK`                                        | **Green** for default non-SSL MySQL 8.4 (`caching_sha2_password` + Promise/callback/pool). CI job `mysql-compat` is blocking. Requires Docker and `make compat-mysql`.                                                                                                                                                                                |
| node-postgres                      | pg 8.22.0 / PostgreSQL 16.14 | Docker PostgreSQL + TLS CA/leaf + `.pgpass` → Node 25-case baseline → schema reset → Lexe 25-case probe                                                                                     | **Passing locally (25/25 Node and Lexe); observing / non-blocking in CI**: SCRAM, Client, Pool, transactions, types, TLS, channel binding, cancel, disconnect recovery, LISTEN/NOTIFY, clean shutdown, and `.pgpass`. Node baseline is a hard gate; Lexe failures emit CI warnings only. Requires Docker and `make compat-node-postgres`. |

Run `make compat-next`, `make compat-vite-plus`, `make compat-better-sqlite3`, `make compat-node-sqlite`, `make compat-mysql`, `make compat-node-postgres`, `make compat-v8`, or `make compat-napi` after building Lexe. V8-native fixtures (`better-sqlite3`, `v8-hello`) require `cargo build --features v8-compat` (the Makefile targets build this automatically). Upgrade a fixture only in a dedicated change that updates its exact dependency versions and lockfile.

**Local `make compat`:** V8 fixtures pin **Node 24.3.0** for ABI 137 (`nvm use 24.3.0`). **Next** and **vite-plus** install steps expect **Node 22.18.0** (CI default) or **≥24.11.0** per upstream engine fields; with only 24.3.0 active, `yarn install` in `compat/vite-plus` may fail on engine checks. **vite-plus** requires `make compat-vite-plus` which builds Lexe with `--features napi`. Lexe advertises Node **24.3.0** identity (`process.version`). **`make compat` includes `compat-mysql` and `compat-node-postgres`**, which require a running Docker daemon. The **mysql2** and **node-postgres** fixture baselines use **Node 22.18.0** (fixture `engines` accept **≥22.18.0**).

**V8 C++ shim platform support:** Linux (x64, arm64) and macOS (Apple Silicon and Intel) are tested in CI. **Windows is not supported** for `v8-compat` — the C++ shim is Unix-only (`build.rs` links a static archive; no `rstr.dll` proxy path). Use WSL or a Linux/macOS host for V8-native addons (`better-sqlite3`, `v8-hello`).

ABI constants for the V8 shim are pinned to **Node 24.3.0 / NODE_MODULE_VERSION 137** (not `refs/node`, which tracks a newer Node). Run `make check-v8-abi` locally after header changes; CI runs the same check on pull requests.

## Next (standalone runtime)

1. Delete any previous `compat/next/.next`.
2. Build with **system Node** (`process.execPath`), not Lexe: `node node_modules/next/dist/bin/next build` (120s wall-clock timeout).
3. Require `.next/standalone/server.js`.
4. Start that entry with **Lexe** (`HOSTNAME=127.0.0.1`, dynamic `PORT`, `NODE_ENV=production`, `NEXT_TELEMETRY_DISABLED=1`, cwd = `.next/standalone`).
5. Poll `GET /api/health` for up to 30s (do not rely on console "Ready" text).
6. Assert (each request aborts after 5s):
   - `GET /` → 200, body contains `Lexe Next compatibility fixture`
   - `GET /api/health` → 200, JSON `{ "status": "ok" }`
   - `GET /posts/42` → 200, body contains `Post 42`
   - Concurrent `GET /api/als/{id}` for multiple ids → each JSON `{ "id": "<same id>" }` (AsyncLocalStorage isolation across await + timers)
7. Always stop the server (SIGTERM, then SIGKILL after 5s). Lexe is started without `LEXE_ASYNC_HOOKS`.

Diagnostics land in `compat/next/compat.log` (Node build command/output, Lexe start command/output, readiness last error, each HTTP check). Static assets (`.next/static`, CSS, images) are **not** copied or verified in this fixture; coverage is HTML SSR, API, dynamic route, and concurrent ALS isolation only.

A green Next result means **Node-built standalone + Lexe runtime HTTP**, not “Lexe can execute `next build`”.

## better-sqlite3 (sync API)

1. **Node baseline** (30s timeout): `node test.cjs` in `compat/better-sqlite3/` — must exit `0` and print `better-sqlite3 compat OK`. Validates the fixture, dependency install, and test script. If this fails, Lexe is not started.
2. **Lexe run** (60s timeout): `$LEXE_BIN test.cjs` with the same cwd — same exit code and stdout marker. This is the compatibility acceptance step.

`test.cjs` exercises (sync API only):

- In-memory `Database`, `exec`, `prepare`, `run`, `get`, `all`
- `transaction()` commit and rollback on thrown error
- File-backed database, `pragma('journal_mode = WAL')`, cleanup

Deferred: `worker_threads`, `backup()`, custom SQL functions, `loadExtension()`, ESM import.

Diagnostics land in `compat/better-sqlite3/compat.log` (Node baseline and Lexe run stdout/stderr).

`better-sqlite3` uses the V8 C++ native addon ABI (not N-API). Lexe provides a QuickJS-backed V8 shim (`v8_compat`, feature `v8-compat`) that implements the Node 24 / ABI 137 callback layouts, templates, accessors, and handle scopes required by the addon.

## node:sqlite built-in

`make compat-node-sqlite` builds the default Lexe binary and runs the same structured probe under Node 24.3.0 and Lexe. Volatile runtime fields are normalized before the results are compared.

The probe covers module loading and descriptors, SQLite version/constants, in-memory and file databases, prepared statements and named parameters, error metadata, scalar and aggregate functions, sessions/changesets, extension loading, backup, repeated database lifecycle, and repeated backup. The default CI stress settings run 100 lifecycle loops and 20 backup loops on Linux and macOS. Windows CI runs the Lexe side of the same probe, and `make compat-node-sqlite-asan` provides the Linux AddressSanitizer gate.

See [`node-sqlite/README.md`](node-sqlite/README.md) for manual usage.

## v8-hello (V8 C++ addon)

1. **Build addon**: `npm run build` in `compat/v8-hello/` (node-gyp compiles `v8_hello.node`).
2. **Node baseline** (30s): `node test.cjs` — must exit `0` and print `v8-hello compat OK`.
3. **Lexe run** (60s): build Lexe with `cargo build --features v8-compat`, then run `$LEXE_BIN test.cjs` — same marker.

`test.cjs` exercises:

- `node::Buffer::Copy` / `Data` / `Length`
- External `Buffer::New` with finalizer callback
- `Persistent::SetWeak` + `ClearWeak` (weak callback must not run after clear)

Diagnostics land in `compat/v8-hello/compat.log`.

## napi-hello (N-API addon)

1. **Build addon**: `yarn build` in `compat/napi-hello/` (node-gyp compiles `hello.node` via N-API).
2. **Node baseline** (30s): `node test.cjs` — must exit `0` and print `napi-hello compat OK`.
3. **Lexe run** (60s): build Lexe with `cargo build --features napi`, then run `$LEXE_BIN test.cjs` — same marker.

Requires a **dynamically linked** Lexe binary (`-rdynamic` / `--export-dynamic`). Static musl container builds (`make lexe-container-*`) do not support `dlopen`.

Diagnostics land in `compat/napi-hello/compat.log`.

### N-API support (Lexe shim)

- **`napi_wrap` / `napi_add_finalizer`**: finalizers run when the wrapped object is GC'd, deferred to the next N-API safe point (or env shutdown). Weak references (`refcount == 0`) allow collection; dead weak refs return `undefined` from `napi_get_reference_value`.
- **Handle scopes**: flat value/handle arena with per-scope watermarks; outer-scope `napi_value` handles remain valid across nested scopes. Using handles after their scope closes is undefined (same as Node).
- **Thread-safe functions**: cross-thread `napi_call_threadsafe_function` enqueues work to the per-env driver; callbacks run on the JS thread. `napi_ref_threadsafe_function` / `napi_unref_threadsafe_function` control whether TSFN refs keep the event loop alive.
- **`napi_queue_async_work`**: `execute` runs on a tokio blocking thread; `complete` is posted back to the JS thread via the driver. `execute` must not call N-API except via TSFN.

## mysql2 (async API)

`make compat-mysql` manages the full test lifecycle:

1. Starts a temporary **mysql:8.4** Docker container with a random local port (no fixed container name).
2. Waits for Docker health checks to report `healthy` (up to 120s).
3. **Node baseline** (30s timeout): `node test.cjs` in `compat/mysql2/` — must exit `0` and print `mysql2 compat OK`. If this fails, Lexe is not started.
4. **Lexe run** (60s timeout): `$LEXE_BIN test.cjs` with the same cwd — same exit code and stdout marker.
5. Stops and removes the container on success, failure, or interrupt (`SIGINT` / `SIGTERM`).

**Requirements:** Docker daemon available locally or on the CI runner. You do **not** need to pre-install, pre-configure, or manually start MySQL. Lexe is built **without** `napi` or `v8-compat`. Node **22.18.0** (fixture `engines` accept **≥22.18.0**), mysql2 **3.23.2**, MySQL **8.4 LTS** with default `caching_sha2_password` authentication.

**Environment variables** when running `test.cjs` directly against an external database (defaults shown):

| Variable         | Default         |
| ---------------- | --------------- |
| `MYSQL_HOST`     | `127.0.0.1`     |
| `MYSQL_PORT`     | `3306`          |
| `MYSQL_DATABASE` | `lexe_compat` |
| `MYSQL_USER`     | `lexe`        |
| `MYSQL_PASSWORD` | `lexe`        |

`make compat-mysql` overrides these with values from its temporary container.

`test.cjs` exercises:

- Module entry (`mysql2` and `mysql2/promise`)
- Promise `createConnection`, `execute`, transactions (commit/rollback), and server error propagation
- Callback `createConnection`, `connect`, `query`, and `end`
- `createPool`, concurrent `getConnection`, parameterized `pool.execute`, `pool.query`, and `pool.end`

The fixture keeps mysql2's default `enableKeepAlive: true` and default authentication flow. It does **not** disable keepalive, switch to legacy auth plugins, or use MariaDB to bypass compatibility gaps.

**Covered by this fixture (default non-SSL):**

1. `net.Socket.setNoDelay()` / `setKeepAlive(true, 0)` (mysql2 default keepalive).
2. `Buffer.copy` / `Buffer.equals` / float write coercion used by the protocol codec.
3. RSA-OAEP `crypto.publicEncrypt` for first-time non-TLS `caching_sha2_password` authentication on MySQL 8.4. Node baseline success is followed by `FLUSH PRIVILEGES` before Lexe runs so the auth cache does not mask a regression.
4. Promise connection/execute/transaction/error, callback connection/query/end, and two-connection pool execute/query/end.

The `node:tls` module is loadable (mysql2's unconditional `require("tls")` at import time). SSL/TLS connections from mysql2, `net.isIP` helpers, and TLSSocket option handoff are **not** covered by this fixture.

**CI:** The `mysql-compat` job calls `make compat-mysql` on Ubuntu (Docker is available on the runner) and is **blocking**. Failures upload `compat/mysql2/compat.log`.

Diagnostics land in `compat/mysql2/compat.log` (Docker image/container/port, health transitions, container logs on failure, database host/port/database/user, Node baseline and Lexe run stdout/stderr, container stop result). Passwords are never logged.

## node-postgres (pg driver)

`make compat-node-postgres` manages the full test lifecycle:

1. Starts a temporary **postgres:16.14-bookworm** Docker container with SSL enabled via short-lived certs generated inside the container, published on a random local port.
2. Waits for Docker health checks to report `healthy` (up to 120s).
3. Exports the container CA certificate to a temp directory (`PG_CA_FILE`) and creates a mode-`0600` password file (`PGPASSFILE`) for TLS and `.pgpass` tests.
4. **Node baseline** (hard gate): `node test.cjs` in `compat/node-postgres/` — must exit `0` and print `node-postgres compat OK`. If this fails, Lexe is not started.
5. **Schema reset**: `DROP SCHEMA public CASCADE; CREATE SCHEMA public; GRANT ALL ON SCHEMA public TO lexe;` so Lexe starts from the same empty state as Node.
6. **Lexe probe** (non-blocking): `$LEXE_BIN test.cjs` with the same env. Lexe failures are recorded in `compat.log` and emitted as GitHub Actions warnings; they do **not** fail the job.
7. Always stops the container and removes temporary certificates on success, failure, or interrupt (`SIGINT` / `SIGTERM`).

**Requirements:** Docker daemon available locally or on the CI runner. You do **not** need to pre-install PostgreSQL. The fixed image `postgres:16.14-bookworm` is pulled on first run. Private keys exist only inside the temporary container (never committed). Lexe is built **without** `napi` or `v8-compat`. Node **22.18.0** (fixture `engines` accept **≥22.18.0**), **pg 8.22.0**, PostgreSQL **16.14** with `scram-sha-256`.

**Environment variables** when running `test.cjs` directly against an external database (defaults shown):

| Variable      | Default                  |
| ------------- | ------------------------ |
| `PGHOST`      | `127.0.0.1`              |
| `PGPORT`      | `5432`                   |
| `PGDATABASE`  | `lexe_compat`          |
| `PGUSER`      | `lexe`                 |
| `PGPASSWORD`  | `lexe-compat-secret`   |
| `PG_CA_FILE`  | (required for TLS cases) |
| `PGPASSFILE`  | (required for PG-025)    |

`make compat-node-postgres` overrides these with values from its temporary container.

`test.cjs` exercises (25 cases, PG-001 … PG-025):

- CommonJS module surface (`Client`, `Pool`, `types`)
- Plain SCRAM-SHA-256 connect/query/`end`
- Callback API, keepalive (`setNoDelay` / `setKeepAlive`)
- Parameter encoding and default type parsers (int8/numeric/jsonb/bytea/timestamptz)
- Query config `rowMode`, named prepared statements
- DML metadata (`RETURNING`), commit/rollback
- SQL error recovery, pool concurrency and connect/release
- Auth failure (`28P01`), statement timeout, `pg_cancel_backend`
- TLS (`rejectUnauthorized: false`, CA verification, channel binding / SCRAM-SHA-256-PLUS, unknown CA rejection)
- Forced disconnect + pool recovery, LISTEN/NOTIFY, clean shutdown
- `.pgpass` / `PGPASSFILE` authentication (password cleared; Client uses password file)

**Out of scope:** `pg-native` (libpq bindings).

**CI:** The `postgres-compat` (`node-postgres-compat`) job calls `make compat-node-postgres` on Ubuntu. Node/Docker/DB init/cleanup failures fail the job; Lexe probe failures only produce warnings. Artifacts always upload `compat/node-postgres/compat.log`.

Diagnostics land in `compat/node-postgres/compat.log` (Docker image/container/port, health transitions, CA export, schema reset, Node and Lexe stdout/stderr, container stop result). Passwords are never logged.

## Failures and CI

Most compatibility failures block merges. The workflow runs `node:sqlite`, `better-sqlite3`, and `v8-hello` on Ubuntu and macOS with Node 24.3.0; `node:sqlite` also has Windows parity and Linux ASan jobs. The **`mysql-compat` job is blocking** and must print `mysql2 compat OK`. The **`postgres-compat` job** treats the Node baseline as blocking and Lexe results as a non-blocking probe.

When a child exits `0` but produces no expected artifact (or HTTP checks fail), `compat/run.mjs` fails with an explicit diagnosis (see `compat/*/compat.log`). On CI failure, `compat.log` and `.next` / `dist` are uploaded as artifacts.
