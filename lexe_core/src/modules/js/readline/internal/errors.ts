// Copyright Joyent, Inc. and other Node contributors. MIT licensed (Node.js).
// Compatibility error types used by the readline port.

export class AbortError extends Error {
  code = "ABORT_ERR";
  cause: unknown;
  constructor(message?: string, options?: { cause?: unknown }) {
    super(message ?? "The operation was aborted");
    this.name = "AbortError";
    this.cause = options?.cause;
  }
}

export class ERR_INVALID_ARG_TYPE extends TypeError {
  code = "ERR_INVALID_ARG_TYPE";
  constructor(name: string, expected: string | string[], actual: unknown) {
    const exp = Array.isArray(expected) ? expected.join(" or ") : expected;
    const type =
      actual === null ? "null" : Array.isArray(actual) ? "array" : typeof actual;
    super(`The "${name}" argument must be of type ${exp}. Received type ${type}`);
    this.name = "TypeError";
  }
}

export class ERR_INVALID_ARG_VALUE extends TypeError {
  code = "ERR_INVALID_ARG_VALUE";
  constructor(name: string, value: unknown, reason?: string) {
    super(
      `The argument '${name}' is invalid. Received ${String(value)}${
        reason ? `. ${reason}` : ""
      }`
    );
    this.name = "TypeError";
  }
}

export class ERR_INVALID_CURSOR_POS extends TypeError {
  code = "ERR_INVALID_CURSOR_POS";
  constructor() {
    super("Cannot set cursor row without setting its column");
    this.name = "TypeError";
  }
}

export class ERR_USE_AFTER_CLOSE extends Error {
  code = "ERR_USE_AFTER_CLOSE";
  constructor(name: string) {
    super(`${name} was closed`);
    this.name = "Error";
  }
}

export const codes = {
  ERR_INVALID_ARG_TYPE,
  ERR_INVALID_ARG_VALUE,
  ERR_INVALID_CURSOR_POS,
  ERR_USE_AFTER_CLOSE,
};
