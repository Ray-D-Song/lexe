// Ported from Node.js v24.3.0 lib/readline/promises.js (MIT).
import { Promise, SymbolDispose } from "./internal/primordials";
import { Readline } from "./internal/promises";
import {
  Interface as _Interface,
  kQuestion,
  kQuestionCancel,
  kQuestionReject,
} from "./internal/interface";
import { AbortError } from "./internal/errors";
import { validateAbortSignal } from "./internal/validators";
import { kEmptyObject, addAbortListener } from "./internal/util_helpers";

class Interface extends _Interface {
  question(query: string, options: any = kEmptyObject) {
    return new Promise<string>((resolve, reject) => {
      let cb: (answer: string) => void = resolve;

      if (options?.signal) {
        validateAbortSignal(options.signal, "options.signal");
        if (options.signal.aborted) {
          return reject(
            new AbortError(undefined, { cause: options.signal.reason })
          );
        }

        const onAbort = () => {
          this[kQuestionCancel]();
          reject(new AbortError(undefined, { cause: options.signal.reason }));
        };
        const disposable = addAbortListener(options.signal, onAbort);

        cb = (answer: string) => {
          disposable[SymbolDispose]();
          resolve(answer);
        };
      }

      this[kQuestionReject] = reject;
      this[kQuestion](query, cb);
    });
  }
}

function createInterface(
  input: any,
  output?: any,
  completer?: any,
  terminal?: boolean
) {
  return new Interface(input, output, completer, terminal);
}

export { Interface, Readline, createInterface };
export default { Interface, Readline, createInterface };
