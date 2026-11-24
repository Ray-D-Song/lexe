'use strict';

const hello = require('./build/Release/v8_hello.node');

function assertEq(actual, expected, label) {
  if (actual !== expected) {
    console.error(`${label}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
    process.exit(1);
  }
}

assertEq(hello.hello(), 'hello from v8', 'hello()');
// Aligned Smi (e.g. 4) must round-trip; must not be deref'd as ObjectLayout*.
assertEq(hello.returnAlignedSmi(), 4, 'returnAlignedSmi()');
assertEq(hello.bufferCopy(), 'buffer-copy-ok', 'bufferCopy()');
assertEq(hello.bufferExternal(), 'buffer-external-ok', 'bufferExternal()');
const sharedBuf = hello.bufferExternalShared();
sharedBuf.write('1234');
assertEq(hello.bufferExternalVerifyJsWrite(), 'buffer-shared-ok', 'bufferExternalVerifyJsWrite()');
assertEq(sharedBuf.toString('utf8', 0, 4), '5678', 'bufferExternalShared native write');
assertEq(hello.bufferProbe(Buffer.from('probe')), 'buffer-probe-ok', 'bufferProbe()');
assertEq(hello.escapableOnce(), 'escapable-once-ok', 'escapableOnce()');
const escapableTwice = hello.escapableTwice();
if (process.versions.lexe) {
  if (escapableTwice !== 'escapable-twice-ok' && escapableTwice !== undefined) {
    console.error(
      `escapableTwice(): expected lexe rejection, got ${JSON.stringify(escapableTwice)}`
    );
    process.exit(1);
  }
} else {
  assertEq(escapableTwice, 'escapable-twice-node-ok', 'escapableTwice()');
}
if (hello.runGc()) {
  hello.bufferExternalFinalizeOnce();
  hello.runGc();
  assertEq(hello.bufferExternalFinalizeCount(), 1, 'bufferExternalFinalizeCount()');
}
assertEq(hello.weakClear(), 'weak-clear-ok', 'weakClear()');
assertEq(hello.weakTwoPassProbe(), 'weak-two-pass-probe-ok', 'weakTwoPassProbe()');
assertEq(hello.persistentLifecycle(), 'persistent-lifecycle-ok', 'persistentLifecycle()');
if (hello.runGc()) {
  assertEq(hello.weakTwoPassGc(), 'weak-two-pass-gc-ok', 'weakTwoPassGc()');
}
// weakShutdownOnly runs last: leaves weak persistent without GC to exercise shutdown drain.
assertEq(hello.weakShutdownOnly(), 'weak-shutdown-only-ok', 'weakShutdownOnly()');

console.log('v8-hello compat OK');
