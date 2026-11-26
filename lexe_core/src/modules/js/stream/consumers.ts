// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
import { Buffer } from "node:buffer";

const encoder = new TextEncoder();

function chunkToBytes(chunk: unknown): Uint8Array {
  if (typeof chunk === "string") {
    return encoder.encode(chunk);
  }
  if (chunk instanceof Uint8Array) {
    return chunk;
  }
  if (ArrayBuffer.isView(chunk)) {
    return new Uint8Array(chunk.buffer, chunk.byteOffset, chunk.byteLength);
  }
  if (chunk instanceof ArrayBuffer) {
    return new Uint8Array(chunk);
  }
  if (typeof SharedArrayBuffer === "function" && chunk instanceof SharedArrayBuffer) {
    return new Uint8Array(chunk);
  }
  throw new TypeError("Stream chunk must be a string, ArrayBuffer, or ArrayBufferView");
}

function assertAsyncIterable(stream: unknown): asserts stream is AsyncIterable<unknown> {
  if (
    stream === null ||
    (typeof stream !== "object" && typeof stream !== "function") ||
    typeof (stream as AsyncIterable<unknown>)[Symbol.asyncIterator] !== "function"
  ) {
    throw new TypeError("The \"stream\" argument must be an AsyncIterable or Readable stream");
  }
}

export async function bytes(stream: AsyncIterable<unknown>): Promise<Uint8Array> {
  assertAsyncIterable(stream);
  const chunks: Uint8Array[] = [];
  let total = 0;
  for await (const chunk of stream) {
    const view = chunkToBytes(chunk);
    if (view.byteLength > 0) {
      chunks.push(view);
      total += view.byteLength;
    }
  }
  const out = new Uint8Array(total);
  let offset = 0;
  for (const c of chunks) {
    out.set(c, offset);
    offset += c.byteLength;
  }
  return out;
}

export async function arrayBuffer(stream: AsyncIterable<unknown>): Promise<ArrayBuffer> {
  const result = await bytes(stream);
  return result.buffer.slice(result.byteOffset, result.byteOffset + result.byteLength) as ArrayBuffer;
}

export async function buffer(stream: AsyncIterable<unknown>): Promise<Buffer> {
  const ab = await arrayBuffer(stream);
  return Buffer.from(ab);
}

export async function text(stream: AsyncIterable<unknown>): Promise<string> {
  assertAsyncIterable(stream);
  const decoder = new TextDecoder("utf-8", { ignoreBOM: false });
  let out = "";
  for await (const chunk of stream) {
    if (typeof chunk === "string") {
      out += chunk;
    } else {
      out += decoder.decode(chunkToBytes(chunk), { stream: true });
    }
  }
  out += decoder.decode();
  return out;
}

export async function json(stream: AsyncIterable<unknown>): Promise<unknown> {
  return JSON.parse(await text(stream));
}

export async function blob(stream: AsyncIterable<unknown>): Promise<Blob> {
  const ab = await arrayBuffer(stream);
  return new Blob([ab]);
}

export default {
  arrayBuffer,
  blob,
  buffer,
  bytes,
  json,
  text,
};
