// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Minimal stub for `node:http2` so packages that import the surface at load
// time (e.g. vite) can resolve. Full HTTP/2 is not implemented.

function notImplemented(name: string): never {
  throw new Error(`node:http2.${name} is not implemented in lexe yet`);
}

export class Http2ServerRequest {
  constructor() {
    notImplemented("Http2ServerRequest");
  }
}

export class Http2ServerResponse {
  constructor() {
    notImplemented("Http2ServerResponse");
  }
}

export function createServer(..._args: unknown[]) {
  return notImplemented("createServer");
}

export function createSecureServer(..._args: unknown[]) {
  return notImplemented("createSecureServer");
}

export function connect(..._args: unknown[]) {
  return notImplemented("connect");
}

export function performServerHandshake(..._args: unknown[]) {
  return notImplemented("performServerHandshake");
}

export function getDefaultSettings() {
  return {
    headerTableSize: 4096,
    enablePush: true,
    initialWindowSize: 65535,
    maxFrameSize: 16384,
    maxConcurrentStreams: 4294967295,
    maxHeaderSize: 65535,
    maxHeaderListSize: 65535,
    enableConnectProtocol: false,
  };
}

export function getPackedSettings(_settings?: object) {
  return Buffer.alloc(0);
}

export function getUnpackedSettings(_buf?: Uint8Array) {
  return {};
}

export const constants = {
  NGHTTP2_SESSION_SERVER: 0,
  NGHTTP2_SESSION_CLIENT: 1,
  NGHTTP2_STREAM_STATE_IDLE: 1,
  HTTP2_HEADER_STATUS: ":status",
  HTTP2_HEADER_METHOD: ":method",
  HTTP2_HEADER_AUTHORITY: ":authority",
  HTTP2_HEADER_SCHEME: ":scheme",
  HTTP2_HEADER_PATH: ":path",
  HTTP2_METHOD_GET: "GET",
  HTTP2_METHOD_POST: "POST",
};

export const sensitiveHeaders = Symbol("nodejs.http2.sensitiveHeaders");

export default {
  Http2ServerRequest,
  Http2ServerResponse,
  createServer,
  createSecureServer,
  connect,
  performServerHandshake,
  getDefaultSettings,
  getPackedSettings,
  getUnpackedSettings,
  constants,
  sensitiveHeaders,
};
