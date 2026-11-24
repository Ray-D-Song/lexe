// Ported from Node.js v24.3.0 lib/readline.js (MIT).
// Copyright Joyent, Inc. and other Node contributors.
import {
  DateNow,
  FunctionPrototypeBind,
  FunctionPrototypeCall,
  ObjectDefineProperties,
  ObjectSetPrototypeOf,
  Promise,
  PromiseReject,
  StringPrototypeSlice,
  SymbolDispose,
} from "./readline/internal/primordials";

import {
  clearLine,
  clearScreenDown,
  cursorTo,
  moveCursor,
} from "./readline/internal/callbacks";
import { emitKeypressEvents } from "./readline/internal/emitKeypressEvents";
import * as promises from "./readline/promises";

import {
  Interface as _Interface,
  InterfaceConstructor,
  kAddHistory,
  kDecoder,
  kDeleteLeft,
  kDeleteLineLeft,
  kDeleteLineRight,
  kDeleteRight,
  kDeleteWordLeft,
  kDeleteWordRight,
  kGetDisplayPos,
  kHistoryNext,
  kHistoryPrev,
  kInsertString,
  kLine,
  kLine_buffer,
  kMoveCursor,
  kNormalWrite,
  kOldPrompt,
  kOnLine,
  kPreviousKey,
  kPrompt,
  kQuestion,
  kQuestionCallback,
  kQuestionCancel,
  kRefreshLine,
  kSawKeyPress,
  kSawReturnAt,
  kSetRawMode,
  kTabComplete,
  kTabCompleter,
  kTtyWrite,
  kWordLeft,
  kWordRight,
  kWriteToOutput,
} from "./readline/internal/interface";

import { AbortError } from "./readline/internal/errors";
import { validateAbortSignal } from "./readline/internal/validators";
import {
  kEmptyObject,
  inspect,
  addAbortListener,
} from "./readline/internal/util_helpers";

// promisify.custom symbol
const kCustomPromisifiedSymbol = Symbol.for("nodejs.util.promisify.custom");

function Interface(this: any, input: any, output?: any, completer?: any, terminal?: boolean) {
  if (!(this instanceof Interface)) {
    return new (Interface as any)(input, output, completer, terminal);
  }

  if (
    input?.input &&
    typeof input.completer === "function" &&
    input.completer.length !== 2
  ) {
    const { completer: c } = input;
    input.completer = (v: string, cb: Function) => cb(null, c(v));
  } else if (typeof completer === "function" && completer.length !== 2) {
    const realCompleter = completer;
    completer = (v: string, cb: Function) => cb(null, realCompleter(v));
  }

  FunctionPrototypeCall(
    InterfaceConstructor,
    this,
    input,
    output,
    completer,
    terminal
  );

  if (process.env && process.env.TERM === "dumb") {
    this._ttyWrite = FunctionPrototypeBind(_ttyWriteDumb, this);
  }
}

ObjectSetPrototypeOf(Interface.prototype, _Interface.prototype);
ObjectSetPrototypeOf(Interface, _Interface);

Interface.prototype.question = function question(
  query: string,
  options: any,
  cb?: Function
) {
  cb = typeof options === "function" ? options : cb;
  if (options === null || typeof options !== "object") {
    options = kEmptyObject;
  }

  if (options.signal) {
    validateAbortSignal(options.signal, "options.signal");
    if (options.signal.aborted) {
      return;
    }

    const onAbort = () => {
      this[kQuestionCancel]();
    };
    const disposable = addAbortListener(options.signal, onAbort);
    const originalCb = cb;
    cb =
      typeof cb === "function"
        ? (answer: string) => {
            disposable[SymbolDispose]();
            return (originalCb as Function)(answer);
          }
        : disposable[SymbolDispose];
  }

  if (typeof cb === "function") {
    this[kQuestion](query, cb);
  }
};

Interface.prototype.question[kCustomPromisifiedSymbol] = function question(
  query: string,
  options: any
) {
  if (options === null || typeof options !== "object") {
    options = kEmptyObject;
  }

  if (options.signal?.aborted) {
    return PromiseReject(
      new AbortError(undefined, { cause: options.signal.reason })
    );
  }

  return new Promise((resolve, reject) => {
    let cb: any = resolve;

    if (options.signal) {
      const onAbort = () => {
        reject(new AbortError(undefined, { cause: options.signal.reason }));
      };
      const disposable = addAbortListener(options.signal, onAbort);
      cb = (answer: string) => {
        disposable[SymbolDispose]();
        resolve(answer);
      };
    }

    this.question(query, options, cb);
  });
};

function createInterface(
  input: any,
  output?: any,
  completer?: any,
  terminal?: boolean
) {
  return new (Interface as any)(input, output, completer, terminal);
}

ObjectDefineProperties(Interface.prototype, {
  [kSetRawMode]: {
    __proto__: null,
    get() {
      return this._setRawMode;
    },
  },
  [kOnLine]: {
    __proto__: null,
    get() {
      return this._onLine;
    },
  },
  [kWriteToOutput]: {
    __proto__: null,
    get() {
      return this._writeToOutput;
    },
  },
  [kAddHistory]: {
    __proto__: null,
    get() {
      return this._addHistory;
    },
  },
  [kRefreshLine]: {
    __proto__: null,
    get() {
      return this._refreshLine;
    },
  },
  [kNormalWrite]: {
    __proto__: null,
    get() {
      return this._normalWrite;
    },
  },
  [kInsertString]: {
    __proto__: null,
    get() {
      return this._insertString;
    },
  },
  [kTabComplete]: {
    __proto__: null,
    get() {
      return this._tabComplete;
    },
  },
  [kWordLeft]: {
    __proto__: null,
    get() {
      return this._wordLeft;
    },
  },
  [kWordRight]: {
    __proto__: null,
    get() {
      return this._wordRight;
    },
  },
  [kDeleteLeft]: {
    __proto__: null,
    get() {
      return this._deleteLeft;
    },
  },
  [kDeleteRight]: {
    __proto__: null,
    get() {
      return this._deleteRight;
    },
  },
  [kDeleteWordLeft]: {
    __proto__: null,
    get() {
      return this._deleteWordLeft;
    },
  },
  [kDeleteWordRight]: {
    __proto__: null,
    get() {
      return this._deleteWordRight;
    },
  },
  [kDeleteLineLeft]: {
    __proto__: null,
    get() {
      return this._deleteLineLeft;
    },
  },
  [kDeleteLineRight]: {
    __proto__: null,
    get() {
      return this._deleteLineRight;
    },
  },
  [kLine]: {
    __proto__: null,
    get() {
      return this._line;
    },
  },
  [kHistoryNext]: {
    __proto__: null,
    get() {
      return this._historyNext;
    },
  },
  [kHistoryPrev]: {
    __proto__: null,
    get() {
      return this._historyPrev;
    },
  },
  [kGetDisplayPos]: {
    __proto__: null,
    get() {
      return this._getDisplayPos;
    },
  },
  [kMoveCursor]: {
    __proto__: null,
    get() {
      return this._moveCursor;
    },
  },
  [kTtyWrite]: {
    __proto__: null,
    get() {
      return this._ttyWrite;
    },
  },
  _decoder: {
    __proto__: null,
    get() {
      return this[kDecoder];
    },
    set(value: any) {
      this[kDecoder] = value;
    },
  },
  _line_buffer: {
    __proto__: null,
    get() {
      return this[kLine_buffer];
    },
    set(value: any) {
      this[kLine_buffer] = value;
    },
  },
  _oldPrompt: {
    __proto__: null,
    get() {
      return this[kOldPrompt];
    },
    set(value: any) {
      this[kOldPrompt] = value;
    },
  },
  _previousKey: {
    __proto__: null,
    get() {
      return this[kPreviousKey];
    },
    set(value: any) {
      this[kPreviousKey] = value;
    },
  },
  _prompt: {
    __proto__: null,
    get() {
      return this[kPrompt];
    },
    set(value: any) {
      this[kPrompt] = value;
    },
  },
  _questionCallback: {
    __proto__: null,
    get() {
      return this[kQuestionCallback];
    },
    set(value: any) {
      this[kQuestionCallback] = value;
    },
  },
  _sawKeyPress: {
    __proto__: null,
    get() {
      return this[kSawKeyPress];
    },
    set(value: any) {
      this[kSawKeyPress] = value;
    },
  },
  _sawReturnAt: {
    __proto__: null,
    get() {
      return this[kSawReturnAt];
    },
    set(value: any) {
      this[kSawReturnAt] = value;
    },
  },
});

Interface.prototype._setRawMode = _Interface.prototype[kSetRawMode];
Interface.prototype._onLine = _Interface.prototype[kOnLine];
Interface.prototype._writeToOutput = _Interface.prototype[kWriteToOutput];
Interface.prototype._addHistory = _Interface.prototype[kAddHistory];
Interface.prototype._refreshLine = _Interface.prototype[kRefreshLine];
Interface.prototype._normalWrite = _Interface.prototype[kNormalWrite];
Interface.prototype._insertString = _Interface.prototype[kInsertString];
Interface.prototype._tabComplete = function (lastKeypressWasTab: boolean) {
  this.pause();
  const string = StringPrototypeSlice(this.line, 0, this.cursor);
  this.completer(string, (err: any, value: any) => {
    this.resume();
    if (err) {
      this._writeToOutput(`Tab completion error: ${inspect(err)}`);
      return;
    }
    this[kTabCompleter](lastKeypressWasTab, value);
  });
};
Interface.prototype._wordLeft = _Interface.prototype[kWordLeft];
Interface.prototype._wordRight = _Interface.prototype[kWordRight];
Interface.prototype._deleteLeft = _Interface.prototype[kDeleteLeft];
Interface.prototype._deleteRight = _Interface.prototype[kDeleteRight];
Interface.prototype._deleteWordLeft = _Interface.prototype[kDeleteWordLeft];
Interface.prototype._deleteWordRight = _Interface.prototype[kDeleteWordRight];
Interface.prototype._deleteLineLeft = _Interface.prototype[kDeleteLineLeft];
Interface.prototype._deleteLineRight = _Interface.prototype[kDeleteLineRight];
Interface.prototype._line = _Interface.prototype[kLine];
Interface.prototype._historyNext = _Interface.prototype[kHistoryNext];
Interface.prototype._historyPrev = _Interface.prototype[kHistoryPrev];
Interface.prototype._getDisplayPos = _Interface.prototype[kGetDisplayPos];
Interface.prototype._getCursorPos = _Interface.prototype.getCursorPos;
Interface.prototype._moveCursor = _Interface.prototype[kMoveCursor];
Interface.prototype._ttyWrite = _Interface.prototype[kTtyWrite];

function _ttyWriteDumb(this: any, s: string, key: any) {
  key ||= kEmptyObject;
  if (key.name === "escape") return;

  if (this[kSawReturnAt] && key.name !== "enter") this[kSawReturnAt] = 0;

  if (key.ctrl) {
    if (key.name === "c") {
      if (this.listenerCount("SIGINT") > 0) {
        this.emit("SIGINT");
      } else {
        this.close();
      }
      return;
    } else if (key.name === "d") {
      this.close();
      return;
    }
  }

  switch (key.name) {
    case "return":
      this[kSawReturnAt] = DateNow();
      this._line();
      break;
    case "enter":
      if (
        this[kSawReturnAt] === 0 ||
        DateNow() - this[kSawReturnAt] > this.crlfDelay
      ) {
        this._line();
      }
      this[kSawReturnAt] = 0;
      break;
    default:
      if (typeof s === "string" && s) {
        this.line += s;
        this.cursor += s.length;
        this._writeToOutput(s);
      }
  }
}

export {
  Interface,
  createInterface,
  clearLine,
  clearScreenDown,
  cursorTo,
  moveCursor,
  emitKeypressEvents,
  promises,
};

export default {
  Interface,
  createInterface,
  clearLine,
  clearScreenDown,
  cursorTo,
  moveCursor,
  emitKeypressEvents,
  promises,
};
