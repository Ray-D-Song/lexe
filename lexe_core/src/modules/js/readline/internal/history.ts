// Simplified history manager ported from Node.js internal/repl/history.js (MIT).
// In-memory history is fully supported; file-backed persistence is best-effort.
import {
  ArrayPrototypeIndexOf,
  ArrayPrototypePop,
  ArrayPrototypeShift,
  ArrayPrototypeSplice,
  ArrayPrototypeUnshift,
  StringPrototypeStartsWith,
  StringPrototypeTrim,
} from "./primordials";
import { reverseString } from "./utils";
import { validateArray, validateNumber } from "./validators";

const kHistorySize = 30;

export class ReplHistory {
  private _history: string[];
  private _size: number;
  private _index = -1;
  private _removeHistoryDuplicates: boolean;
  private _context: any;
  private _isFlushing = false;

  constructor(context: any, options: any = {}) {
    if (options.history !== undefined) validateArray(options.history, "history");
    if (options.size !== undefined) validateNumber(options.size, "size");

    this._context = context;
    this._removeHistoryDuplicates = !!options.removeHistoryDuplicates;
    this._size = options.size ?? context.historySize ?? kHistorySize;
    this._history = options.history ? options.history.slice() : [];
  }

  initialize(onReadyCallback?: (err: Error | null, ctx?: any) => void) {
    if (typeof onReadyCallback === "function") {
      onReadyCallback(null, this._context);
    }
  }

  addHistory(isMultiline: boolean, lastCommandErrored: boolean) {
    const line = this._context.line;
    if (line.length === 0) return "";
    if (this._size === 0) return line;
    if (StringPrototypeTrim(line).length === 0) return line;

    if (isMultiline && this._index === -1) {
      ArrayPrototypeShift(this._history);
    } else if (lastCommandErrored && this._index !== -1) {
      ArrayPrototypeShift(this._history);
    }

    const normalizedLine = reverseString(line, "\n", "\r");

    if (this._history.length === 0 || this._history[0] !== normalizedLine) {
      if (this._removeHistoryDuplicates) {
        const dupIndex = ArrayPrototypeIndexOf(this._history, normalizedLine);
        if (dupIndex !== -1) ArrayPrototypeSplice(this._history, dupIndex, 1);
      }
      ArrayPrototypeUnshift(this._history, normalizedLine);
      if (this._history.length > this._size) ArrayPrototypePop(this._history);
    }

    this._index = -1;
    const finalLine = isMultiline
      ? reverseString(this._history[0])
      : this._history[0];
    this._context.emit("history", this._history);
    return finalLine;
  }

  canNavigateToNext() {
    return this._index > -1 && this._history.length > 0;
  }

  navigateToNext(substringSearch?: string) {
    if (!this.canNavigateToNext()) return null;
    const search = substringSearch || "";
    let index = this._index - 1;
    while (
      index >= 0 &&
      (!StringPrototypeStartsWith(this._history[index], search) ||
        this._context.line === this._history[index])
    ) {
      index--;
    }
    this._index = index;
    if (index === -1) return search;
    return reverseString(this._history[index], "\r", "\n");
  }

  canNavigateToPrevious() {
    return this._history.length !== this._index && this._history.length > 0;
  }

  navigateToPrevious(substringSearch = "") {
    if (!this.canNavigateToPrevious()) return null;
    const search = substringSearch || "";
    let index = this._index + 1;
    while (
      index < this._history.length &&
      (!StringPrototypeStartsWith(this._history[index], search) ||
        this._context.line === this._history[index])
    ) {
      index++;
    }
    this._index = index;
    if (index === this._history.length) return search;
    return reverseString(this._history[index], "\r", "\n");
  }

  get size() {
    return this._size;
  }
  get isFlushing() {
    return this._isFlushing;
  }
  get history() {
    return this._history;
  }
  set history(value: string[]) {
    this._history = value;
  }
  get index() {
    return this._index;
  }
  set index(value: number) {
    this._index = value;
  }
}
