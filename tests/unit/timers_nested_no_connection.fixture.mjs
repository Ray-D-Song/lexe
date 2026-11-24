// Fixture: reverse guard. Nested timers registered from a continuation, with no
// connection, must still let the process exit. Catches a fix that keeps the
// timer loop alive whenever it saw any callback run.
await new Promise((r) => setTimeout(r, 10));
console.log("outer");

await new Promise((r) => setTimeout(r, 50));
console.log("inner");
