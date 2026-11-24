/// <reference path="../buffer.d.ts" />
/// <reference path="../fetch.d.ts" />
/// <reference path="../url.d.ts" />
import buffer, {
  Blob,
  Buffer,
  File,
  INSPECT_MAX_BYTES,
  SlowBuffer,
  isAscii,
  isUtf8,
  kMaxLength,
  kStringMaxLength,
  resolveObjectURL,
  transcode,
} from "buffer";

const encoded: Buffer = transcode(Buffer.from("hi"), "utf8", "ucs2");
const utf8: boolean = isUtf8(encoded) && isUtf8(new ArrayBuffer(1));
const ascii: boolean = isAscii(new Uint8Array(1));
const limits: number = kMaxLength + kStringMaxLength + INSPECT_MAX_BYTES;
const slow: Buffer = SlowBuffer(1);
const url: string = URL.createObjectURL(new Blob(["x"]));
const resolved: Blob | undefined = resolveObjectURL(url);
URL.revokeObjectURL(url);
const file: File = new File(["x"], "x.txt");
buffer.INSPECT_MAX_BYTES = 10;

export { ascii, file, limits, resolved, slow, utf8 };
