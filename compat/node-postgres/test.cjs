"use strict";

const fs = require("node:fs");

const CASE_TIMEOUT_MS = 8_000;
const CLOSE_TIMEOUT_MS = 1_000;

let pg;
let pgImportError;
try {
  pg = require("pg");
} catch (error) {
  pgImportError = error;
}

const cases = [];
const results = [];

/** @type {{ clients: Set<unknown>, pools: Set<unknown> } | null} */
let activeResources = null;

function test(id, name, fn) {
  cases.push({ id, name, fn });
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(message || "assertion failed");
  }
}

function assertEqual(actual, expected, message) {
  if (actual !== expected) {
    throw new Error(
      message ||
        `expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`
    );
  }
}

function assertDeepEqual(actual, expected, message) {
  const actualJson = JSON.stringify(actual);
  const expectedJson = JSON.stringify(expected);
  if (actualJson !== expectedJson) {
    throw new Error(
      message || `expected ${expectedJson}, got ${actualJson}`
    );
  }
}

async function expectReject(promise, verify, message) {
  try {
    await promise;
  } catch (error) {
    verify(error);
    return error;
  }
  throw new Error(message || "expected promise to reject");
}

function requirePg() {
  if (pgImportError) {
    throw new Error("pg unavailable", { cause: pgImportError });
  }
  return pg;
}

function baseConfig(overrides = {}) {
  return {
    host: process.env.PGHOST || "127.0.0.1",
    port: Number(process.env.PGPORT || 5432),
    database: process.env.PGDATABASE || "lexe_compat",
    user: process.env.PGUSER || "lexe",
    password: process.env.PGPASSWORD || "lexe-compat-secret",
    connectionTimeoutMillis: 2_000,
    ...overrides,
  };
}

function captureErrors(emitter) {
  const errors = [];
  emitter.on("error", (error) => errors.push(error));
  return errors;
}

function trackClient(client) {
  activeResources?.clients.add(client);
  return client;
}

function trackPool(pool) {
  activeResources?.pools.add(pool);
  return pool;
}

function untrackClient(client) {
  activeResources?.clients.delete(client);
}

function untrackPool(pool) {
  activeResources?.pools.delete(pool);
}

function destroyClientStream(client) {
  try {
    client?.connection?.stream?.destroy();
  } catch {
    // ignore cleanup races
  }
}

function destroyPoolStreams(pool) {
  try {
    for (const client of pool?._clients || []) {
      destroyClientStream(client);
    }
  } catch {
    // ignore private pool shape differences
  }
}

/**
 * Race a promise against a timeout, always clearing the timer.
 * Rejects with an Error if the timeout wins.
 */
async function raceWithTimeout(promise, timeoutMs, label = "operation") {
  let timer;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => {
          reject(new Error(`${label} timed out after ${timeoutMs}ms`));
        }, timeoutMs);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

/**
 * @param {object} [options]
 * @param {boolean} [options.strict] When true, surface end failures / close timeouts.
 */
async function closeClient(client, { strict = false } = {}) {
  if (!client) {
    return;
  }

  try {
    await raceWithTimeout(client.end(), CLOSE_TIMEOUT_MS, "client.end");
  } catch (error) {
    destroyClientStream(client);
    if (strict) {
      throw error instanceof Error ? error : new Error(String(error));
    }
  } finally {
    untrackClient(client);
  }
}

/**
 * @param {object} [options]
 * @param {boolean} [options.strict] When true, surface end failures / close timeouts.
 */
async function closePool(pool, { strict = false } = {}) {
  if (!pool) {
    return;
  }

  try {
    await raceWithTimeout(pool.end(), CLOSE_TIMEOUT_MS, "pool.end");
  } catch (error) {
    destroyPoolStreams(pool);
    if (strict) {
      throw error instanceof Error ? error : new Error(String(error));
    }
  } finally {
    untrackPool(pool);
  }
}

function createClient(options = {}) {
  requirePg();
  return trackClient(new pg.Client(baseConfig(options)));
}

function createPool(options = {}) {
  requirePg();
  return trackPool(new pg.Pool(baseConfig(options)));
}

async function withClient(options, fn, closeOptions = {}) {
  const client = createClient(options);
  try {
    await client.connect();
    return await fn(client);
  } finally {
    await closeClient(client, closeOptions);
  }
}

async function withPool(options, fn, closeOptions = {}) {
  const pool = createPool(options);
  try {
    return await fn(pool);
  } finally {
    await closePool(pool, closeOptions);
  }
}

async function forceCleanupResources(resources) {
  // Destroy sockets first so hanging connect/query/end unblocks.
  for (const client of resources.clients) {
    destroyClientStream(client);
  }
  for (const pool of resources.pools) {
    destroyPoolStreams(pool);
  }

  await Promise.allSettled([
    ...[...resources.clients].map((client) => closeClient(client)),
    ...[...resources.pools].map((pool) => closePool(pool)),
  ]);
}

async function waitForActivity(admin, pid, marker) {
  const deadline = Date.now() + 2_000;

  while (Date.now() < deadline) {
    const result = await admin.query(
      `
        SELECT state, query
        FROM pg_stat_activity
        WHERE pid = $1
      `,
      [pid]
    );

    const row = result.rows[0];
    if (row?.state === "active" && row.query.includes(marker)) {
      return;
    }

    await new Promise((resolve) => setTimeout(resolve, 20));
  }

  throw new Error(`backend ${pid} did not enter expected active query`);
}

const CERTIFICATE_ERROR_CODES = new Set([
  "UNABLE_TO_VERIFY_LEAF_SIGNATURE",
  "SELF_SIGNED_CERT_IN_CHAIN",
  "DEPTH_ZERO_SELF_SIGNED_CERT",
  "CERT_HAS_EXPIRED",
  "UNABLE_TO_GET_ISSUER_CERT_LOCALLY",
  "ERR_TLS_CERT_ALTNAME_INVALID",
]);

function isCertificateError(error) {
  if (CERTIFICATE_ERROR_CODES.has(error?.code)) {
    return true;
  }

  const message = String(error?.message || "");
  // Narrow message match: known OpenSSL / rustls verification wording only.
  // Do not accept bare "certificate", "TLS", or "ssl" (avoids PG-021 false positives).
  return /self[- ]signed|unknown ca|unable to verify|issuer certificate|certificate verification|CaUsedAsEndEntity|UnknownIssuer|InvalidCertificate|unable to get local issuer|DEPTH_ZERO_SELF_SIGNED|SELF_SIGNED_CERT/i.test(
    message
  );
}

async function runCase(spec) {
  const resources = {
    clients: new Set(),
    pools: new Set(),
  };
  const previous = activeResources;
  activeResources = resources;

  try {
    await raceWithTimeout(
      spec.fn(),
      CASE_TIMEOUT_MS,
      `${spec.id}`
    );
  } catch (error) {
    // Always force-cleanup on any failure so hung ops do not leak into later cases.
    await forceCleanupResources(resources);
    throw error;
  } finally {
    // Successful path: drain anything still tracked (should already be closed).
    if (resources.clients.size > 0 || resources.pools.size > 0) {
      await forceCleanupResources(resources);
    }
    activeResources = previous;
  }
}

// ---------------------------------------------------------------------------
// Test cases
// ---------------------------------------------------------------------------

test("PG-001", "CommonJS module surface", async () => {
  if (pgImportError) throw pgImportError;

  assert(typeof pg.Client === "function", "Client should be a constructor");
  assert(typeof pg.Pool === "function", "Pool should be a constructor");
  assert(
    typeof pg.types?.getTypeParser === "function",
    "types.getTypeParser should be a function"
  );
});

test("PG-002", "plain SCRAM connection", async () => {
  await withClient({}, async (client) => {
    const result = await client.query(`
      SELECT
        current_user AS current_user,
        current_database() AS current_database,
        1 AS value
    `);
    assertEqual(result.rowCount, 1, "rowCount should be 1");
    assertEqual(result.rows[0].current_user, "lexe");
    assertEqual(result.rows[0].current_database, "lexe_compat");
    assertEqual(result.rows[0].value, 1);
  });
});

test("PG-003", "callback API", async () => {
  const client = createClient();
  let connectCalls = 0;
  let queryCalls = 0;
  let endCalls = 0;
  let closed = false;

  try {
    await new Promise((resolve, reject) => {
      client.connect((error) => {
        connectCalls += 1;
        if (error) {
          reject(error);
          return;
        }
        resolve();
      });
    });
    assertEqual(connectCalls, 1, "connect callback should fire once");

    await new Promise((resolve, reject) => {
      client.query("SELECT $1::int AS value", [42], (error, result) => {
        queryCalls += 1;
        if (error) {
          reject(error);
          return;
        }
        try {
          assertEqual(result.rows[0].value, 42);
          resolve();
        } catch (err) {
          reject(err);
        }
      });
    });
    assertEqual(queryCalls, 1, "query callback should fire once");

    await new Promise((resolve, reject) => {
      client.end((error) => {
        endCalls += 1;
        if (error) {
          reject(error);
          return;
        }
        resolve();
      });
    });
    closed = true;
    untrackClient(client);
    assertEqual(endCalls, 1, "end callback should fire once");
  } finally {
    if (!closed) {
      destroyClientStream(client);
      await closeClient(client);
    }
  }
});

test("PG-004", "keep-alive", async () => {
  await withClient(
    {
      keepAlive: true,
      keepAliveInitialDelayMillis: 1000,
    },
    async (client) => {
      const result = await client.query("SELECT 1 AS value");
      assertEqual(result.rows[0].value, 1);
    }
  );
});

test("PG-005", "parameter encoding", async () => {
  await withClient({}, async (client) => {
    const result = await client.query(
      `
      SELECT
        $1::int4 AS integer_value,
        $2::text AS text_value,
        $3::boolean AS boolean_value,
        $4::text AS null_value,
        $5::text AS quoted_value
    `,
      [42, "hello", true, null, "Lexe's PostgreSQL"]
    );
    const row = result.rows[0];
    assertEqual(row.integer_value, 42);
    assertEqual(row.text_value, "hello");
    assertEqual(row.boolean_value, true);
    assertEqual(row.null_value, null);
    assertEqual(row.quoted_value, "Lexe's PostgreSQL");
  });
});

test("PG-006", "default type parsing", async () => {
  await withClient({}, async (client) => {
    const result = await client.query(`
      SELECT
        2147483647::int4 AS i4,
        9007199254740993::int8 AS i8,
        123.45::numeric AS numeric_value,
        true AS flag,
        '{"nested":{"ok":true}}'::jsonb AS document,
        decode('0001feff', 'hex') AS bytes,
        TIMESTAMPTZ '2024-01-02 03:04:05+00' AS happened_at
    `);
    const row = result.rows[0];
    assertEqual(row.i4, 2147483647);
    assertEqual(row.i8, "9007199254740993");
    assertEqual(row.numeric_value, "123.45");
    assertEqual(row.flag, true);
    assertDeepEqual(row.document, { nested: { ok: true } });
    assert(Buffer.isBuffer(row.bytes), "bytes should be a Buffer");
    assertEqual(row.bytes.toString("hex"), "0001feff");
    assert(row.happened_at instanceof Date, "happened_at should be a Date");
    assertEqual(row.happened_at.toISOString(), "2024-01-02T03:04:05.000Z");
  });
});

test("PG-007", "query config and rowMode", async () => {
  await withClient({}, async (client) => {
    const result = await client.query({
      text: "SELECT $1::int AS first, $2::text AS second",
      values: [7, "seven"],
      rowMode: "array",
    });
    assertDeepEqual(result.rows, [[7, "seven"]]);
    assertDeepEqual(
      result.fields.map((field) => field.name),
      ["first", "second"]
    );
  });
});

test("PG-008", "named prepared statements", async () => {
  await withClient({}, async (client) => {
    const first = await client.query({
      name: "lexe-compat-by-id",
      text: "SELECT $1::int AS value",
      values: [1],
    });
    assertEqual(first.rows[0].value, 1);

    const second = await client.query({
      name: "lexe-compat-by-id",
      text: "SELECT $1::int AS value",
      values: [2],
    });
    assertEqual(second.rows[0].value, 2);
  });
});

test("PG-009", "DML metadata", async () => {
  await withClient({}, async (client) => {
    await client.query("DROP TABLE IF EXISTS lexe_pg_items");
    await client.query(`
      CREATE TABLE lexe_pg_items (
        id integer PRIMARY KEY,
        name text NOT NULL UNIQUE
      )
    `);

    const insert = await client.query(
      "INSERT INTO lexe_pg_items (id, name) VALUES ($1, $2) RETURNING id, name",
      [1, "alpha"]
    );
    assertEqual(insert.command, "INSERT");
    assertEqual(insert.rowCount, 1);
    assertDeepEqual(insert.rows[0], { id: 1, name: "alpha" });
    assertDeepEqual(
      insert.fields.map((field) => field.name),
      ["id", "name"]
    );

    const update = await client.query(
      "UPDATE lexe_pg_items SET name = $1 WHERE id = $2 RETURNING id, name",
      ["beta", 1]
    );
    assertEqual(update.command, "UPDATE");
    assertEqual(update.rowCount, 1);
    assertDeepEqual(update.rows[0], { id: 1, name: "beta" });
    assertDeepEqual(
      update.fields.map((field) => field.name),
      ["id", "name"]
    );

    const del = await client.query(
      "DELETE FROM lexe_pg_items WHERE id = $1 RETURNING id, name",
      [1]
    );
    assertEqual(del.command, "DELETE");
    assertEqual(del.rowCount, 1);
    assertDeepEqual(del.rows[0], { id: 1, name: "beta" });
    assertDeepEqual(
      del.fields.map((field) => field.name),
      ["id", "name"]
    );
  });
});

test("PG-010", "transaction commit", async () => {
  await withClient({}, async (client) => {
    await client.query("DROP TABLE IF EXISTS lexe_pg_tx");
    await client.query(`
      CREATE TABLE lexe_pg_tx (
        id integer PRIMARY KEY,
        value text NOT NULL
      )
    `);
    await client.query("BEGIN");
    await client.query(
      "INSERT INTO lexe_pg_tx (id, value) VALUES (1, 'committed')"
    );
    await client.query("COMMIT");
  });

  await withClient({}, async (client) => {
    const result = await client.query(
      "SELECT id, value FROM lexe_pg_tx WHERE id = 1"
    );
    assertEqual(result.rowCount, 1);
    assertDeepEqual(result.rows[0], { id: 1, value: "committed" });
  });
});

test("PG-011", "transaction rollback", async () => {
  await withClient({}, async (client) => {
    await client.query("DROP TABLE IF EXISTS lexe_pg_tx_rollback");
    await client.query(`
      CREATE TABLE lexe_pg_tx_rollback (
        id integer PRIMARY KEY,
        value text NOT NULL
      )
    `);
    await client.query(
      "INSERT INTO lexe_pg_tx_rollback (id, value) VALUES (1, 'baseline')"
    );

    await client.query("BEGIN");
    await client.query(
      "INSERT INTO lexe_pg_tx_rollback (id, value) VALUES (2, 'rolled-back')"
    );
    await client.query("ROLLBACK");
  });

  await withClient({}, async (client) => {
    const rolled = await client.query(
      "SELECT id FROM lexe_pg_tx_rollback WHERE id = 2"
    );
    assertEqual(rolled.rowCount, 0, "id=2 should not exist after rollback");

    const baseline = await client.query(
      "SELECT id, value FROM lexe_pg_tx_rollback WHERE id = 1"
    );
    assertEqual(baseline.rowCount, 1, "id=1 baseline should still exist");
    assertDeepEqual(baseline.rows[0], { id: 1, value: "baseline" });
  });
});

test("PG-012", "SQL error then connection reuse", async () => {
  await withClient({}, async (client) => {
    await client.query("DROP TABLE IF EXISTS lexe_pg_unique");
    await client.query(`
      CREATE TABLE lexe_pg_unique (
        id integer PRIMARY KEY
      )
    `);
    await client.query("INSERT INTO lexe_pg_unique (id) VALUES (1)");

    await expectReject(
      client.query("INSERT INTO lexe_pg_unique (id) VALUES (1)"),
      (error) => {
        assert(error instanceof Error, "error should be an Error");
        assertEqual(error.code, "23505");
        assert(error.constraint, "error.constraint should be present");
      },
      "duplicate primary key should reject"
    );

    const result = await client.query("SELECT 1 AS value");
    assertEqual(result.rows[0].value, 1);
  });
});

test("PG-013", "pool concurrency", async () => {
  await withPool(
    {
      max: 2,
      idleTimeoutMillis: 2_000,
    },
    async (pool) => {
      captureErrors(pool);
      const values = [1, 2, 3, 4, 5, 6];
      const results = await Promise.all(
        values.map((value) =>
          pool.query(
            "SELECT pg_backend_pid() AS pid, $1::int AS value, pg_sleep(0.1)",
            [value]
          )
        )
      );

      const returnedValues = results
        .map((result) => result.rows[0].value)
        .sort((a, b) => a - b);
      assertDeepEqual(returnedValues, values);

      const pids = new Set(results.map((result) => result.rows[0].pid));
      assertEqual(pids.size, 2, "should use exactly 2 backend PIDs");
      assertEqual(pool.totalCount, 2);
      assertEqual(pool.idleCount, 2);
      assertEqual(pool.waitingCount, 0);
    }
  );
});

test("PG-014", "pool connect and release", async () => {
  await withPool({ max: 2 }, async (pool) => {
    captureErrors(pool);
    let clientA;
    let clientB;
    try {
      clientA = await pool.connect();
      clientB = await pool.connect();
      const pidA = (await clientA.query("SELECT pg_backend_pid() AS pid"))
        .rows[0].pid;
      const pidB = (await clientB.query("SELECT pg_backend_pid() AS pid"))
        .rows[0].pid;
      assert(pidA !== pidB, "pooled clients should use different PIDs");
    } finally {
      try {
        clientA?.release();
      } catch {
        // ignore
      }
      try {
        clientB?.release();
      } catch {
        // ignore
      }
    }

    const result = await pool.query("SELECT 1 AS value");
    assertEqual(result.rows[0].value, 1);
  });
});

test("PG-015", "wrong password", async () => {
  const client = createClient({
    password: "definitely-wrong",
  });
  const errors = captureErrors(client);

  try {
    await expectReject(
      client.connect(),
      (error) => {
        assert(error instanceof Error, "connect should reject with Error");
        assertEqual(error.code, "28P01");
      },
      "wrong password should reject"
    );
    assertEqual(errors.length, 0, "no unhandled error events expected");
  } finally {
    destroyClientStream(client);
    await closeClient(client);
  }
});

test("PG-016", "statement timeout", async () => {
  await withClient({}, async (client) => {
    await client.query("SET statement_timeout = 200");
    const startedAt = Date.now();
    await expectReject(
      client.query("SELECT pg_sleep(3) /* lexe-pg-statement-timeout */"),
      (error) => {
        assertEqual(error.code, "57014");
      },
      "statement timeout should cancel long query"
    );
    const elapsed = Date.now() - startedAt;
    assert(elapsed < 2000, `elapsed ${elapsed}ms should be < 2000ms`);

    const result = await client.query("SELECT 1 AS value");
    assertEqual(result.rows[0].value, 1);
  });
});

test("PG-017", "pg_cancel_backend", async () => {
  const clientA = createClient();
  const admin = createClient();

  try {
    await clientA.connect();
    await admin.connect();

    const pid = (await clientA.query("SELECT pg_backend_pid() AS pid")).rows[0]
      .pid;

    const sleepPromise = clientA.query(
      "SELECT pg_sleep(10) /* lexe-pg-cancel */"
    );

    await waitForActivity(admin, pid, "lexe-pg-cancel");

    const cancel = await admin.query(
      "SELECT pg_cancel_backend($1) AS cancelled",
      [pid]
    );
    assertEqual(cancel.rows[0].cancelled, true);

    await expectReject(
      sleepPromise,
      (error) => {
        assertEqual(error.code, "57014");
      },
      "cancelled query should reject with 57014"
    );

    const result = await clientA.query("SELECT 1 AS value");
    assertEqual(result.rows[0].value, 1);
  } finally {
    await Promise.allSettled([closeClient(clientA), closeClient(admin)]);
  }
});

test("PG-018", "TLS with rejectUnauthorized false", async () => {
  await withClient(
    {
      ssl: {
        rejectUnauthorized: false,
      },
    },
    async (client) => {
      const result = await client.query(`
        SELECT ssl
        FROM pg_stat_ssl
        WHERE pid = pg_backend_pid()
      `);
      assertEqual(result.rows[0].ssl, true);
    }
  );
});

test("PG-019", "TLS with CA verification", async () => {
  const caPath = process.env.PG_CA_FILE;
  assert(caPath, "PG_CA_FILE must be set");
  const ca = fs.readFileSync(caPath, "utf8");

  await withClient(
    {
      ssl: {
        ca,
        servername: "localhost",
        rejectUnauthorized: true,
      },
    },
    async (client) => {
      const result = await client.query(`
        SELECT ssl
        FROM pg_stat_ssl
        WHERE pid = pg_backend_pid()
      `);
      assertEqual(result.rows[0].ssl, true);
    }
  );
});

test("PG-020", "TLS channel binding", async () => {
  const caPath = process.env.PG_CA_FILE;
  assert(caPath, "PG_CA_FILE must be set");
  const ca = fs.readFileSync(caPath, "utf8");

  await withClient(
    {
      enableChannelBinding: true,
      ssl: {
        ca,
        servername: "localhost",
        rejectUnauthorized: true,
      },
    },
    async (client) => {
      const result = await client.query("SELECT 1 AS value");
      assertEqual(result.rows[0].value, 1);
    }
  );
});

test("PG-021", "TLS unknown CA", async () => {
  // Guard rails: keep isCertificateError narrow (no bare certificate/TLS/ssl).
  assertEqual(
    isCertificateError({ message: "certificate" }),
    false,
    "bare 'certificate' must not match"
  );
  assertEqual(
    isCertificateError({
      message: "There was an error establishing an SSL connection",
    }),
    false,
    "generic SSL connection error must not match"
  );
  assertEqual(
    isCertificateError({ message: "TLS socket closed" }),
    false,
    "generic TLS close must not match"
  );
  assertEqual(
    isCertificateError({ message: "self-signed certificate" }),
    true,
    "OpenSSL self-signed wording must match"
  );
  assertEqual(
    isCertificateError({ code: "SELF_SIGNED_CERT_IN_CHAIN" }),
    true,
    "known cert error code must match"
  );
  assertEqual(
    isCertificateError({ message: "UnknownIssuer" }),
    true,
    "rustls UnknownIssuer must match"
  );

  const client = createClient({
    ssl: {
      servername: "localhost",
      rejectUnauthorized: true,
    },
  });
  captureErrors(client);

  try {
    await expectReject(
      client.connect(),
      (error) => {
        assert(
          isCertificateError(error),
          `expected certificate-related error, got code=${error?.code} message=${error?.message}`
        );
      },
      "TLS without CA should reject"
    );
  } finally {
    destroyClientStream(client);
    await closeClient(client);
  }
});

test("PG-022", "forced disconnect and pool recovery", async () => {
  const pool = createPool({ max: 2 });
  captureErrors(pool);
  const admin = createClient();
  let client = null;

  try {
    await admin.connect();

    client = await pool.connect();
    const errors = captureErrors(client);
    const ends = [];
    client.on("end", () => ends.push(true));

    const pid = (await client.query("SELECT pg_backend_pid() AS pid")).rows[0]
      .pid;

    const terminated = await admin.query(
      "SELECT pg_terminate_backend($1) AS terminated",
      [pid]
    );
    assertEqual(terminated.rows[0].terminated, true);

    const deadline = Date.now() + 2_000;
    while (Date.now() < deadline && errors.length === 0 && ends.length === 0) {
      await new Promise((resolve) => setTimeout(resolve, 20));
    }
    assert(
      errors.length > 0 || ends.length > 0,
      "terminated client should emit error or end"
    );

    client.release(true);
    client = null;

    const recovered = await pool.query("SELECT pg_backend_pid() AS pid");
    assert(recovered.rows[0].pid !== pid, "new query should use a new PID");
    assertEqual(pool.waitingCount, 0);
  } finally {
    if (client) {
      try {
        client.release(true);
      } catch {
        // ignore
      }
      destroyClientStream(client);
    }
    await Promise.allSettled([closeClient(admin), closePool(pool)]);
  }
});

test("PG-023", "LISTEN/NOTIFY", async () => {
  const listener = createClient();
  const sender = createClient();

  try {
    await listener.connect();
    await sender.connect();

    await listener.query("LISTEN lexe_compat_channel");

    const notificationPromise = new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        reject(new Error("notification timed out after 2000ms"));
      }, 2_000);
      listener.once("notification", (notification) => {
        clearTimeout(timer);
        resolve(notification);
      });
    });

    await sender.query(
      "SELECT pg_notify('lexe_compat_channel', 'hello-lexe')"
    );

    const notification = await notificationPromise;
    assertEqual(notification.channel, "lexe_compat_channel");
    assertEqual(notification.payload, "hello-lexe");
    assert(notification.processId > 0, "processId should be > 0");
  } finally {
    await Promise.allSettled([closeClient(listener), closeClient(sender)]);
  }
});

test("PG-024", "clean shutdown without residual handles", async () => {
  // Strict close: end() failures and close timeouts must surface as FAIL.
  await withClient(
    {},
    async (client) => {
      const result = await client.query("SELECT 1 AS value");
      assertEqual(result.rows[0].value, 1);
    },
    { strict: true }
  );

  await withPool(
    {},
    async (pool) => {
      const result = await pool.query("SELECT 1 AS value");
      assertEqual(result.rows[0].value, 1);
    },
    { strict: true }
  );
});

test("PG-025", ".pgpass authentication via PGPASSFILE", async () => {
  assert(
    process.env.PGPASSFILE,
    "PGPASSFILE must be set by the compat runner"
  );

  // Force the pgpass path: clear env password and pass null so Client
  // does not fall back to process.env.PGPASSWORD from baseConfig.
  const savedPassword = process.env.PGPASSWORD;
  delete process.env.PGPASSWORD;

  let client;
  try {
    client = createClient({ password: null });
    await client.connect();
    const result = await client.query("SELECT 1 AS value");
    assertEqual(result.rows[0].value, 1);
  } finally {
    if (savedPassword === undefined) {
      delete process.env.PGPASSWORD;
    } else {
      process.env.PGPASSWORD = savedPassword;
    }
    if (client) {
      await closeClient(client, { strict: true });
    }
  }
});

async function main() {
  for (const spec of cases) {
    const startedAt = Date.now();

    if (pgImportError && spec.id !== "PG-001") {
      results.push({
        id: spec.id,
        name: spec.name,
        status: "SKIP",
        durationMs: Date.now() - startedAt,
        reason: "pg module could not be loaded",
      });
      console.log(
        `SKIP ${spec.id} ${spec.name}: pg module unavailable`
      );
      continue;
    }

    try {
      await runCase(spec);
      results.push({
        id: spec.id,
        name: spec.name,
        status: "PASS",
        durationMs: Date.now() - startedAt,
      });
      console.log(`PASS ${spec.id} ${spec.name}`);
    } catch (error) {
      results.push({
        id: spec.id,
        name: spec.name,
        status: "FAIL",
        durationMs: Date.now() - startedAt,
        error: error?.stack || String(error),
      });
      console.error(`FAIL ${spec.id} ${spec.name}`);
      console.error(error?.stack || error);
    }
  }

  const failures = results.filter((result) => result.status === "FAIL");
  const skips = results.filter((result) => result.status === "SKIP");
  const passes = results.filter((result) => result.status === "PASS");
  console.log(
    `node-postgres summary: ${passes.length} passed, ` +
      `${failures.length} failed, ${skips.length} skipped`
  );

  if (failures.length > 0) {
    process.exitCode = 1;
    return;
  }

  console.log("node-postgres compat OK");
}

main().catch((error) => {
  console.error(error?.stack || error);
  process.exitCode = 1;
});
