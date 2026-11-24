// Validators shim for readline (Node 24.3 semantics, simplified).
import { codes } from "./errors";

export function validateFunction(value: unknown, name: string): asserts value is Function {
  if (typeof value !== "function") {
    throw codes.ERR_INVALID_ARG_TYPE(name, "function", value);
  }
}

export function validateString(value: unknown, name: string): asserts value is string {
  if (typeof value !== "string") {
    throw codes.ERR_INVALID_ARG_TYPE(name, "string", value);
  }
}

export function validateBoolean(value: unknown, name: string): asserts value is boolean {
  if (typeof value !== "boolean") {
    throw codes.ERR_INVALID_ARG_TYPE(name, "boolean", value);
  }
}

export function validateAbortSignal(signal: unknown, name: string) {
  if (
    signal === null ||
    typeof signal !== "object" ||
    !("aborted" in (signal as object))
  ) {
    throw codes.ERR_INVALID_ARG_TYPE(name, "AbortSignal", signal);
  }
}

export function validateUint32(value: unknown, name: string, positive = false) {
  if (
    typeof value !== "number" ||
    !Number.isInteger(value) ||
    value < (positive ? 1 : 0) ||
    value > 0xffffffff
  ) {
    throw codes.ERR_INVALID_ARG_VALUE(name, value);
  }
}

export function validateInteger(
  value: unknown,
  name: string,
  min = Number.MIN_SAFE_INTEGER,
  max = Number.MAX_SAFE_INTEGER
) {
  if (typeof value !== "number" || !Number.isInteger(value)) {
    throw codes.ERR_INVALID_ARG_TYPE(name, "integer", value);
  }
  if (value < min || value > max) {
    throw codes.ERR_INVALID_ARG_VALUE(name, value);
  }
}

export function validateNumber(value: unknown, name: string) {
  if (typeof value !== "number") {
    throw codes.ERR_INVALID_ARG_TYPE(name, "number", value);
  }
}

export function validateArray(value: unknown, name: string) {
  if (!Array.isArray(value)) {
    throw codes.ERR_INVALID_ARG_TYPE(name, "Array", value);
  }
}

export function validateOneOf(value: unknown, name: string, oneOf: string[]) {
  if (!oneOf.includes(value as string)) {
    throw codes.ERR_INVALID_ARG_VALUE(
      name,
      value,
      `must be one of: ${oneOf.join(", ")}`
    );
  }
}

export function validateObject(value: unknown, name: string) {
  if (value === null || typeof value !== "object") {
    throw codes.ERR_INVALID_ARG_TYPE(name, "Object", value);
  }
}
