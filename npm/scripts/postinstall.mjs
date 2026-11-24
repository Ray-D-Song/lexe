import { ensureBinary } from "../lib/download.mjs";

const skip = process.env.LEXE_SKIP_DOWNLOAD?.trim().toLowerCase();
if (skip === "1" || skip === "true" || process.env.LEXE_BINARY_PATH?.trim()) {
  process.exit(0);
}

try {
  await ensureBinary({ preferPackageDir: true });
} catch (err) {
  if (err && err.code === "LEXE_CHECKSUM_MISMATCH") {
    console.error(`lexe: ${err.message}`);
    process.exit(1);
  }
  const message = err instanceof Error ? err.message : String(err);
  console.warn(
    `lexe: skipped pre-downloading native binary during install (${message}); Lexe will download and verify it on first run.`
  );
}
