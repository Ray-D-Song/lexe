// WHATWG URLPattern implementation (https://urlpattern.spec.whatwg.org/).
// Installed on `globalThis.URLPattern` and re-exported by `node:url`.
(function (globalObj) {
  "use strict";

  const URLCtor = globalObj.URL;

  const RE_ID_START = /^\p{ID_Start}$/u;
  const RE_ID_CONTINUE = /^\p{ID_Continue}$/u;
  const RE_REGEXP_ESCAPE = /[.+*?^${}()|[\]/\\]/g;
  const RE_PATTERN_ESCAPE = /[+*?:{}()\\]/g;
  const SPECIAL_SCHEMES = new Set(["http", "https", "ws", "wss", "ftp"]);

  const DEFAULT_OPTIONS = {
    delimiter: "",
    prefix: "",
    ignoreCase: false,
  };

  const PATHNAME_OPTIONS = {
    delimiter: "/",
    prefix: "/",
    ignoreCase: false,
  };

  const HOSTNAME_OPTIONS = {
    delimiter: ".",
    prefix: "",
    ignoreCase: false,
  };

  const COMPONENT_KEYS = [
    "protocol",
    "username",
    "password",
    "hostname",
    "port",
    "pathname",
    "search",
    "hash",
  ];

  const SPECIAL_SCHEME_DEFAULT_PORT = {
    http: "80",
    https: "443",
    ws: "80",
    wss: "443",
    ftp: "21",
  };

  function makeCodedError(code, message) {
    const err = new TypeError(message);
    Object.defineProperty(err, "code", {
      value: code,
      enumerable: true,
      configurable: true,
      writable: true,
    });
    return err;
  }

  function escapeRegExpString(input) {
    return input.replace(RE_REGEXP_ESCAPE, "\\$&");
  }

  function escapePatternString(input) {
    return input.replace(RE_PATTERN_ESCAPE, "\\$&");
  }

  function isASCIIHexDigit(ch) {
    const c = ch.charCodeAt(0);
    return (c >= 48 && c <= 57) || (c >= 65 && c <= 70) || (c >= 97 && c <= 102);
  }

  function isValidNameCodePoint(ch, first) {
    if (!ch) return false;
    if (ch === "$" || ch === "_") return true;
    if (!first && (ch === "\u200C" || ch === "\u200D")) return true;
    return first ? RE_ID_START.test(ch) : RE_ID_CONTINUE.test(ch);
  }

  function nextCodePoint(str, index) {
    const cp = str.codePointAt(index);
    const len = cp > 0xffff ? 2 : 1;
    return {
      cp,
      ch: str.slice(index, index + len),
      nextIndex: index + len,
    };
  }

  // ---------------------------------------------------------------------------
  // Tokenizer (WHATWG URLPattern Section 2.1)
  // ---------------------------------------------------------------------------
  function tokenize(input, policy) {
    const tokens = [];
    const len = input.length;
    let index = 0;

    function pushError(tokenIndex, value, nextIndex) {
      if (policy === "strict") {
        throw makeCodedError("ERR_INVALID_URL_PATTERN", "Failed to construct URLPattern");
      }
      tokens.push({ type: "invalid-char", index: tokenIndex, value });
      index = nextIndex;
    }

    while (index < len) {
      const { cp, ch, nextIndex } = nextCodePoint(input, index);

      if (ch === "*") {
        tokens.push({ type: "asterisk", index, value: "*" });
        index = nextIndex;
        continue;
      }

      if (ch === "+" || ch === "?") {
        tokens.push({ type: "other-modifier", index, value: ch });
        index = nextIndex;
        continue;
      }

      if (ch === "\\") {
        if (index === len - 1) {
          pushError(index, ch, nextIndex);
          continue;
        }
        const escaped = nextCodePoint(input, nextIndex);
        tokens.push({ type: "escaped-char", index, value: escaped.ch });
        index = escaped.nextIndex;
        continue;
      }

      if (ch === "{") {
        tokens.push({ type: "open", index, value: "{" });
        index = nextIndex;
        continue;
      }

      if (ch === "}") {
        tokens.push({ type: "close", index, value: "}" });
        index = nextIndex;
        continue;
      }

      if (ch === ":") {
        let namePos = nextIndex;
        let name = "";
        while (namePos < len) {
          const nc = nextCodePoint(input, namePos);
          const first = namePos === nextIndex;
          if (!isValidNameCodePoint(nc.ch, first)) break;
          name += nc.ch;
          namePos = nc.nextIndex;
        }
        if (name.length === 0) {
          pushError(index, ch, nextIndex);
          continue;
        }
        tokens.push({ type: "name", index, value: name });
        index = namePos;
        continue;
      }

      if (ch === "(") {
        let parenDepth = 1;
        let regexpPos = nextIndex;
        let pattern = "";
        let error = false;

        while (regexpPos < len) {
          const rc = nextCodePoint(input, regexpPos);
          if (rc.cp > 0x7f) {
            pushError(index, ch, nextIndex);
            error = true;
            break;
          }
          if (regexpPos === nextIndex && rc.ch === "?") {
            pushError(index, ch, nextIndex);
            error = true;
            break;
          }
          if (rc.ch === "\\") {
            if (regexpPos === len - 1) {
              pushError(index, ch, nextIndex);
              error = true;
              break;
            }
            const esc = nextCodePoint(input, rc.nextIndex);
            if (esc.cp > 0x7f) {
              pushError(index, ch, nextIndex);
              error = true;
              break;
            }
            pattern += "\\" + esc.ch;
            regexpPos = esc.nextIndex;
            continue;
          }
          if (rc.ch === ")") {
            parenDepth--;
            if (parenDepth === 0) {
              regexpPos = rc.nextIndex;
              break;
            }
            pattern += ")";
            regexpPos = rc.nextIndex;
            continue;
          }
          if (rc.ch === "(") {
            parenDepth++;
            if (rc.nextIndex === len) {
              pushError(index, ch, nextIndex);
              error = true;
              break;
            }
            const peek = nextCodePoint(input, rc.nextIndex);
            if (peek.ch !== "?") {
              pushError(index, ch, nextIndex);
              error = true;
              break;
            }
          }
          pattern += rc.ch;
          regexpPos = rc.nextIndex;
        }

        if (error) continue;
        if (parenDepth !== 0 || pattern.length === 0) {
          pushError(index, ch, nextIndex);
          continue;
        }
        tokens.push({ type: "regexp", index, value: pattern });
        index = regexpPos;
        continue;
      }

      // Note: unmatched invalid surrogate or normal char
      tokens.push({ type: "char", index, value: ch });
      index = nextIndex;
    }

    tokens.push({ type: "end", index: len, value: "" });
    return tokens;
  }

  // ---------------------------------------------------------------------------
  // Canonicalization Callbacks (WHATWG URLPattern Section 1.5)
  // ---------------------------------------------------------------------------
  function dummyURL(str) {
    return new URLCtor(str);
  }

  function canonicalizeProtocol(value) {
    if (value === "") return "";
    // Strip a single optional trailing ":" when passed via init object.
    const clean = value.endsWith(":") ? value.slice(0, -1) : value;
    if (!/^[a-zA-Z][a-zA-Z0-9+.-]*$/.test(clean)) {
      throw makeCodedError("ERR_INVALID_URL_PATTERN", "Failed to construct URLPattern");
    }
    return clean.toLowerCase();
  }

  function canonicalizeUsername(value) {
    if (value === "") return "";
    const url = dummyURL("https://dummy.test");
    url.username = value;
    return url.username;
  }

  function canonicalizePassword(value) {
    if (value === "") return "";
    const url = dummyURL("https://dummy.test");
    url.password = value;
    return url.password;
  }

  function canonicalizeIPv6Hostname(value) {
    let result = "";
    for (let i = 0; i < value.length; i++) {
      const ch = value[i];
      if (!isASCIIHexDigit(ch) && ch !== "[" && ch !== "]" && ch !== ":") {
        throw makeCodedError("ERR_INVALID_URL_PATTERN", "Failed to construct URLPattern");
      }
      result += ch.toLowerCase();
    }
    return result;
  }

  function canonicalizeHostname(value) {
    if (value === "") return "";
    if (value.includes("[") || value.includes("]")) {
      return canonicalizeIPv6Hostname(value);
    }
    try {
      const url = new URLCtor("https://dummy.test");
      // Setting hostname on invalid inputs leaves it unchanged in WHATWG URL,
      // so verify by parsing `https://${value}` directly when possible or sentinel comparison.
      // Reject URL delimiters like '/', '?', '#', '@', ':' in hostname fixed-text.
      if (/[/?#@:\\]/.test(value)) {
        throw new Error("invalid host char");
      }
      const parsed = new URLCtor(`https://${value}`);
      if (parsed.pathname !== "/" || parsed.port !== "" || parsed.username !== "") {
        throw new Error("invalid host");
      }
      return parsed.hostname;
    } catch {
      throw makeCodedError("ERR_INVALID_URL_PATTERN", "Failed to construct URLPattern");
    }
  }

  function canonicalizePort(value, protocolValue) {
    if (value === "") return "";
    let proto = protocolValue ? canonicalizeProtocol(protocolValue) : "dummy";
    if (!SPECIAL_SCHEMES.has(proto)) {
      proto = "dummy";
    }
    let url;
    try {
      url = new URLCtor(`${proto}://dummy.test`);
    } catch {
      url = new URLCtor("dummy://dummy.test");
    }
    url.port = "2";
    url.port = value;
    // In WHATWG URL spec, invalid port leaves url.port unchanged ("2").
    if (value !== "2" && url.port === "2") {
      throw makeCodedError("ERR_INVALID_URL_PATTERN", "Failed to construct URLPattern");
    }
    // Also reject non-ASCII-digit characters if default port stripped or trailing junk?
    // Wait: WHATWG port state allows trailing garbage via setter ONLY up to EOF or invalid.
    // Check Node behavior tested above: "80abc" -> "80", "65536" -> throw.
    return url.port;
  }

  function canonicalizePathname(value) {
    if (value === "") return "";
    const leadingSlash = value[0] === "/";
    const modified = leadingSlash ? value : "/-" + value;
    const url = new URLCtor("https://dummy.test");
    url.pathname = modified;
    let out = url.pathname;
    if (!leadingSlash) {
      out = out.slice(2);
    }
    return out;
  }

  function canonicalizeOpaquePathname(value) {
    if (value === "") return "";
    // WHATWG cannot-be-a-base-URL path state: percent-encode C0 controls and >= 0x7F UTF-8 bytes.
    let out = "";
    for (let i = 0; i < value.length; ) {
      const { cp, ch, nextIndex } = nextCodePoint(value, i);
      if (cp <= 0x1f || cp === 0x7f) {
        out += "%" + cp.toString(16).toUpperCase().padStart(2, "0");
      } else if (cp >= 0x80) {
        try {
          out += encodeURIComponent(ch);
        } catch {
          out += "%EF%BF%BD";
        }
      } else {
        out += ch;
      }
      i = nextIndex;
    }
    return out;
  }

  function canonicalizeSearch(value) {
    if (value === "") return "";
    const url = new URLCtor("https://dummy.test");
    url.search = value;
    return url.search.startsWith("?") ? url.search.slice(1) : url.search;
  }

  function canonicalizeHash(value) {
    if (value === "") return "";
    const url = new URLCtor("https://dummy.test");
    url.hash = value;
    return url.hash.startsWith("#") ? url.hash.slice(1) : url.hash;
  }

  function isProtocolPatternSpecialScheme(protocolPattern) {
    return (
      protocolPattern.partList.length === 1 &&
      protocolPattern.partList[0].type === "fixed-text" &&
      SPECIAL_SCHEMES.has(protocolPattern.partList[0].value)
    ) || (
      // Match simple regex/optional like `http{s}?`? WHATWG actually uses
      // `protocolComponent.matches("")`? No: tests component against SPECIAL_SCHEMES.
      Array.from(SPECIAL_SCHEMES).some((s) => protocolPattern.regexp.test(s))
    );
  }

  // ---------------------------------------------------------------------------
  // Constructor String Parser (WHATWG URLPattern Section 1.3)
  // ---------------------------------------------------------------------------
  function parseConstructorString(input) {
    const tokenList = tokenize(input, "lenient");
    const result = {};
    let state = "init";
    let tokenIndex = 0;
    let componentStart = 0;
    let tokenIncrement = 1;
    let groupDepth = 0;
    let hostnameIPv6BracketDepth = 0;
    let protocolMatchesSpecialSchemeFlag = false;

    function getSafeToken(idx) {
      return idx < tokenList.length ? tokenList[idx] : tokenList[tokenList.length - 1];
    }

    function isNonSpecialPatternChar(idx, value) {
      const tok = getSafeToken(idx);
      if (tok.value !== value) return false;
      return tok.type === "char" || tok.type === "escaped-char" || tok.type === "invalid-char";
    }

    function isProtocolSuffix() {
      return isNonSpecialPatternChar(tokenIndex, ":");
    }

    function isIdentityTerminator() {
      return isNonSpecialPatternChar(tokenIndex, "@");
    }

    function isPasswordPrefix() {
      return isNonSpecialPatternChar(tokenIndex, ":");
    }

    function isPortPrefix() {
      return isNonSpecialPatternChar(tokenIndex, ":");
    }

    function isPathnameStart() {
      return isNonSpecialPatternChar(tokenIndex, "/");
    }

    function isSearchPrefix() {
      if (isNonSpecialPatternChar(tokenIndex, "?")) return true;
      const tok = getSafeToken(tokenIndex);
      if (tok.value !== "?") return false;
      if (tokenIndex === 0) return true;
      const prev = getSafeToken(tokenIndex - 1);
      return (
        prev.type !== "name" &&
        prev.type !== "regexp" &&
        prev.type !== "close" &&
        prev.type !== "asterisk"
      );
    }

    function isHashPrefix() {
      return isNonSpecialPatternChar(tokenIndex, "#");
    }

    function isAuthoritySlashesNext() {
      if (!isNonSpecialPatternChar(tokenIndex + 1, "/")) return false;
      if (!isNonSpecialPatternChar(tokenIndex + 2, "/")) return false;
      return true;
    }

    function makeComponentString() {
      const tok = getSafeToken(tokenIndex);
      const endIndex = tok.type === "end" ? input.length : tok.index;
      return input.slice(componentStart, endIndex);
    }

    function computeProtocolMatchesSpecialSchemeFlag() {
      try {
        const comp = compileComponent(makeComponentString(), encodeStringNoop, DEFAULT_OPTIONS);
        protocolMatchesSpecialSchemeFlag = false;
        for (const scheme of SPECIAL_SCHEMES) {
          if (comp.regexp.test(scheme)) {
            protocolMatchesSpecialSchemeFlag = true;
            break;
          }
        }
      } catch {
        protocolMatchesSpecialSchemeFlag = false;
      }
    }

    function rewind() {
      tokenIndex = componentStartTokenIndex;
      tokenIncrement = 0;
    }

    function changeState(newState, skip) {
      if (state !== "init" && state !== "authority" && state !== "done") {
        result[state] = makeComponentString();
      }
      if (state !== "init" && newState !== "done") {
        if (
          (state === "protocol" || state === "authority" || state === "username" || state === "password") &&
          newState === "port" &&
          result.hostname === undefined
        ) {
          result.hostname = "";
        }
        if (
          (state === "protocol" || state === "authority" || state === "username" || state === "password" || state === "hostname" || state === "port") &&
          newState !== "authority" &&
          newState !== "username" &&
          newState !== "password" &&
          newState !== "hostname" &&
          newState !== "port" &&
          protocolMatchesSpecialSchemeFlag
        ) {
          if (result.hostname === undefined) result.hostname = "";
          if (result.port === undefined) result.port = "";
        }
        if (
          (state === "protocol" || state === "authority" || state === "username" || state === "password" || state === "hostname" || state === "port") &&
          (newState === "search" || newState === "hash") &&
          result.pathname === undefined
        ) {
          result.pathname = protocolMatchesSpecialSchemeFlag ? "/" : "";
        }
        if (newState === "hash" && result.search === undefined) {
          result.search = "";
        }
      }
      state = newState;
      if (state !== "done") {
        const skipTok = getSafeToken(tokenIndex + skip);
        componentStart = skipTok.index;
        tokenIndex += skip;
        componentStartTokenIndex = tokenIndex;
        tokenIncrement = 0;
      }
    }

    let componentStartTokenIndex = 0;

    while (tokenIndex < tokenList.length) {
      const tok = tokenList[tokenIndex];
      tokenIncrement = 1;

      if (tok.type === "end") {
        if (state === "init") {
          tokenIndex = 0;
          componentStart = 0;
          componentStartTokenIndex = 0;
          tokenIncrement = 0;
          if (isNonSpecialPatternChar(0, "#")) {
            state = "hash";
            componentStart = getSafeToken(1).index;
            tokenIndex = 1;
            componentStartTokenIndex = 1;
          } else if (isSearchPrefix()) {
            state = "search";
            componentStart = getSafeToken(1).index;
            tokenIndex = 1;
            componentStartTokenIndex = 1;
          } else {
            state = "pathname";
          }
          continue;
        }
        if (state === "authority") {
          tokenIndex = componentStartTokenIndex;
          tokenIncrement = 0;
          state = "hostname";
          continue;
        }
        changeState("done", 0);
        break;
      }

      if (tok.type === "open") {
        groupDepth++;
        tokenIndex += tokenIncrement;
        continue;
      }
      if (groupDepth > 0) {
        if (tok.type === "close") {
          groupDepth--;
        }
        tokenIndex += tokenIncrement;
        continue;
      }

      switch (state) {
        case "init": {
          if (isProtocolSuffix()) {
            tokenIndex = 0;
            componentStart = 0;
            componentStartTokenIndex = 0;
            tokenIncrement = 0;
            state = "protocol";
          }
          break;
        }
        case "protocol": {
          if (isProtocolSuffix()) {
            computeProtocolMatchesSpecialSchemeFlag();
            let nextState = "pathname";
            let skip = 1;
            if (isAuthoritySlashesNext()) {
              nextState = "authority";
              skip = 3;
            } else if (protocolMatchesSpecialSchemeFlag) {
              nextState = "authority";
              skip = 1;
            }
            if (nextState === "pathname") {
              // Opaque / non-special without //
              result.protocol = makeComponentString();
              if (!protocolMatchesSpecialSchemeFlag) {
                result.hostname = "";
                result.port = "";
              }
              state = "pathname";
              const skipTok = getSafeToken(tokenIndex + skip);
              componentStart = skipTok.index;
              tokenIndex += skip;
              componentStartTokenIndex = tokenIndex;
              tokenIncrement = 0;
            } else {
              changeState(nextState, skip);
              // Consume optional extra leading slashes for special schemes (`http:///` etc.)
              if (protocolMatchesSpecialSchemeFlag && nextState === "authority") {
                while (isPathnameStart()) {
                  tokenIndex++;
                  componentStart = getSafeToken(tokenIndex).index;
                  componentStartTokenIndex = tokenIndex;
                }
              }
            }
          }
          break;
        }
        case "authority": {
          if (isIdentityTerminator()) {
            tokenIndex = componentStartTokenIndex;
            tokenIncrement = 0;
            state = "username";
          } else if (isPathnameStart() || isSearchPrefix() || isHashPrefix()) {
            tokenIndex = componentStartTokenIndex;
            tokenIncrement = 0;
            state = "hostname";
          }
          break;
        }
        case "username": {
          if (isPasswordPrefix()) {
            changeState("password", 1);
          } else if (isIdentityTerminator()) {
            changeState("hostname", 1);
          }
          break;
        }
        case "password": {
          if (isIdentityTerminator()) {
            changeState("hostname", 1);
          }
          break;
        }
        case "hostname": {
          if (tok.value === "[" && (tok.type === "char" || tok.type === "escaped-char")) {
            hostnameIPv6BracketDepth++;
          } else if (tok.value === "]" && (tok.type === "char" || tok.type === "escaped-char")) {
            hostnameIPv6BracketDepth--;
          } else if (isPortPrefix() && hostnameIPv6BracketDepth === 0) {
            changeState("port", 1);
          } else if (isPathnameStart()) {
            changeState("pathname", 0);
          } else if (isSearchPrefix()) {
            changeState("search", 1);
          } else if (isHashPrefix()) {
            changeState("hash", 1);
          }
          break;
        }
        case "port": {
          if (isPathnameStart()) {
            changeState("pathname", 0);
          } else if (isSearchPrefix()) {
            changeState("search", 1);
          } else if (isHashPrefix()) {
            changeState("hash", 1);
          }
          break;
        }
        case "pathname": {
          if (isSearchPrefix()) {
            changeState("search", 1);
          } else if (isHashPrefix()) {
            changeState("hash", 1);
          }
          break;
        }
        case "search": {
          if (isHashPrefix()) {
            changeState("hash", 1);
          }
          break;
        }
        case "hash": {
          break;
        }
      }

      tokenIndex += tokenIncrement;
    }

    if (result.hostname !== undefined && result.port === undefined) {
      result.port = "";
    }
    return result;
  }

  // ---------------------------------------------------------------------------
  // Pattern Parser & RegExp Compiler (WHATWG URLPattern Section 2.2 - 2.4)
  // ---------------------------------------------------------------------------
  function encodeStringNoop(v) {
    return v;
  }

  function parsePatternString(input, options, encodeCallback) {
    const tokenList = tokenize(input, "strict");
    const partList = [];
    let pendingFixed = "";
    let index = 0;
    let nextUnnamedGroupKey = 0;

    function getSafeToken(idx) {
      return idx < tokenList.length ? tokenList[idx] : tokenList[tokenList.length - 1];
    }

    function tryConsumeToken(type) {
      if (index < tokenList.length && tokenList[index].type === type) {
        return tokenList[index++];
      }
      return null;
    }

    function tryConsumeRegexpOrWildcardToken(nameTok) {
      const tok = tryConsumeToken("regexp");
      if (tok) return tok;
      if (!nameTok) {
        return tryConsumeToken("asterisk");
      }
      return null;
    }

    function tryConsumeModifierToken() {
      return tryConsumeToken("other-modifier") || tryConsumeToken("asterisk");
    }

    function consumeRequiredToken(type) {
      const tok = tryConsumeToken(type);
      if (!tok) {
        throw makeCodedError("ERR_INVALID_URL_PATTERN", "Failed to construct URLPattern");
      }
      return tok;
    }

    function consumeText() {
      let result = "";
      while (index < tokenList.length) {
        const tok = tokenList[index];
        if (tok.type === "char" || tok.type === "escaped-char") {
          result += tok.value;
          index++;
        } else {
          break;
        }
      }
      return result;
    }

    function maybeAddPartFromPendingFixedValue() {
      if (pendingFixed === "") return;
      const encoded = encodeCallback(pendingFixed);
      pendingFixed = "";
      if (encoded === "") return;
      partList.push({
        type: "fixed-text",
        value: encoded,
        modifier: "none",
      });
    }

    function addPart(prefix, nameTok, regexpOrWildcardTok, suffix, modifierTok) {
      let modifier = "none";
      if (modifierTok) {
        if (modifierTok.value === "?") modifier = "optional";
        else if (modifierTok.value === "*") modifier = "zero-or-more";
        else if (modifierTok.value === "+") modifier = "one-or-more";
      }

      if (!nameTok && !regexpOrWildcardTok && modifier === "none") {
        pendingFixed += prefix;
        return;
      }

      maybeAddPartFromPendingFixedValue();

      if (!nameTok && !regexpOrWildcardTok) {
        if (prefix === "") return;
        const encoded = encodeCallback(prefix);
        partList.push({
          type: "fixed-text",
          value: encoded,
          modifier,
        });
        return;
      }

      let regexpValue = "";
      if (!regexpOrWildcardTok) {
        regexpValue = "";
      } else if (regexpOrWildcardTok.type === "asterisk") {
        regexpValue = "*";
      } else {
        regexpValue = regexpOrWildcardTok.value;
      }

      let partType = "regexp";
      if (regexpValue === "") {
        partType = "segment-wildcard";
      } else if (regexpValue === "*") {
        partType = "full-wildcard";
        regexpValue = "";
      }

      let name = "";
      if (nameTok) {
        name = nameTok.value;
      } else if (regexpOrWildcardTok) {
        name = String(nextUnnamedGroupKey++);
      }

      for (const existing of partList) {
        if (existing.name === name) {
          throw makeCodedError("ERR_INVALID_URL_PATTERN", "Failed to construct URLPattern");
        }
      }

      const encodedPrefix = encodeCallback(prefix);
      const encodedSuffix = encodeCallback(suffix);

      partList.push({
        type: partType,
        value: regexpValue,
        modifier,
        name,
        prefix: encodedPrefix,
        suffix: encodedSuffix,
      });
    }

    while (index < tokenList.length) {
      const charOrEscaped = tryConsumeToken("char") || tryConsumeToken("escaped-char");
      let nameTok = tryConsumeToken("name");
      let regexpOrWildcardTok = tryConsumeRegexpOrWildcardToken(nameTok);

      if (nameTok || regexpOrWildcardTok) {
        let prefix = charOrEscaped ? charOrEscaped.value : "";
        if (!options.prefix || prefix !== options.prefix) {
          pendingFixed += prefix;
          prefix = "";
        }
        maybeAddPartFromPendingFixedValue();
        const modifierTok = tryConsumeModifierToken();
        addPart(prefix, nameTok, regexpOrWildcardTok, "", modifierTok);
        continue;
      }

      if (charOrEscaped) {
        pendingFixed += charOrEscaped.value;
        continue;
      }

      const openTok = getSafeToken(index);
      if (openTok.type === "open") {
        index++;
        const prefix = consumeText();
        const gNameTok = tryConsumeToken("name");
        const gRegexpOrWildcard = tryConsumeRegexpOrWildcardToken(gNameTok);
        const suffix = consumeText();
        consumeRequiredToken("close");
        const modifierTok = tryConsumeModifierToken();
        addPart(prefix, gNameTok, gRegexpOrWildcard, suffix, modifierTok);
        continue;
      }

      maybeAddPartFromPendingFixedValue();
      consumeRequiredToken("end");
      break;
    }

    return partList;
  }

  function convertPartListToRegExpAndGroups(partList, options) {
    let result = "^";
    const groupNames = [];
    let hasRegExpGroups = false;
    const segmentWildcardRegexp = options.delimiter
      ? `[^${escapeRegExpString(options.delimiter)}]+?`
      : "[\\s\\S]+?";
    const fullWildcardRegexp = ".*";
    const flags = options.ignoreCase ? "vi" : "v";

    for (const part of partList) {
      if (part.type === "fixed-text") {
        if (part.modifier === "none") {
          result += escapeRegExpString(part.value);
        } else {
          result += `(?:${escapeRegExpString(part.value)})${
            part.modifier === "optional" ? "?" : part.modifier === "zero-or-more" ? "*" : "+"
          }`;
        }
        continue;
      }

      if (part.type === "regexp") {
        hasRegExpGroups = true;
      }

      groupNames.push(part.name);
      let regexpValue;
      if (part.type === "segment-wildcard") {
        regexpValue = segmentWildcardRegexp;
      } else if (part.type === "full-wildcard") {
        regexpValue = fullWildcardRegexp;
      } else {
        regexpValue = part.value;
      }

      // Validate inner user regexp syntax
      if (part.type === "regexp") {
        try {
          new RegExp(regexpValue, flags);
        } catch {
          throw makeCodedError("ERR_INVALID_URL_PATTERN", "Failed to construct URLPattern");
        }
      }

      if (!part.prefix && !part.suffix) {
        if (part.modifier === "none" || part.modifier === "optional") {
          result += `(${regexpValue})${part.modifier === "optional" ? "?" : ""}`;
        } else if (part.modifier === "zero-or-more") {
          result += `((?:${regexpValue})*)`;
        } else {
          result += `((?:${regexpValue})+)`;
        }
        continue;
      }

      if (part.modifier === "none" || part.modifier === "optional") {
        const inner = `${escapeRegExpString(part.prefix)}(${regexpValue})${escapeRegExpString(part.suffix)}`;
        result += part.modifier === "none" ? inner : `(?:${inner})?`;
        continue;
      }

      // zero-or-more or one-or-more with prefix/suffix
      const escapedPrefix = escapeRegExpString(part.prefix);
      const escapedSuffix = escapeRegExpString(part.suffix);
      let inner = `(?:${escapedPrefix})((?:${regexpValue})(?:${escapedSuffix}${escapedPrefix}(?:${regexpValue}))*)(?:${escapedSuffix})`;
      if (part.modifier === "zero-or-more") {
        inner = `(?:${inner})?`;
      }
      result += inner;
    }

    result += "$";
    let regexp;
    try {
      regexp = new RegExp(result, flags);
    } catch {
      throw makeCodedError("ERR_INVALID_URL_PATTERN", "Failed to construct URLPattern");
    }

    return { regexp, groupNames, hasRegExpGroups };
  }

  function generatePatternString(partList, options) {
    let result = "";
    for (let i = 0; i < partList.length; i++) {
      const part = partList[i];
      const previous = i > 0 ? partList[i - 1] : null;
      const next = i + 1 < partList.length ? partList[i + 1] : null;
      const mod =
        part.modifier === "none"
          ? ""
          : part.modifier === "optional"
          ? "?"
          : part.modifier === "zero-or-more"
          ? "*"
          : "+";

      if (part.type === "fixed-text") {
        if (part.modifier === "none") {
          result += escapePatternString(part.value);
        } else {
          result += `{${escapePatternString(part.value)}}${mod}`;
        }
        continue;
      }

      const customName = !/^\d+$/.test(part.name);
      let needsGrouping = Boolean(
        part.suffix || (part.prefix && part.prefix !== options.prefix)
      );
      if (!needsGrouping && customName && part.type === "segment-wildcard" && part.modifier === "none" && next && next.type === "fixed-text" && next.modifier === "none") {
        const firstNext = nextCodePoint(next.value, 0);
        if (isValidNameCodePoint(firstNext.ch, false)) {
          needsGrouping = true;
        }
      }
      if (!needsGrouping && !part.prefix && previous && previous.type === "fixed-text" && previous.modifier === "none" && previous.value.slice(-1) === options.prefix) {
        needsGrouping = true;
      }

      if (needsGrouping) {
        result += "{";
      }
      result += escapePatternString(part.prefix);
      if (customName) {
        result += `:${part.name}`;
      }
      if (part.type === "regexp") {
        result += `(${part.value})`;
      } else if (part.type === "full-wildcard") {
        if (!customName && (!previous || previous.type === "fixed-text" || previous.modifier !== "none" || needsGrouping || part.prefix !== "")) {
          result += "*";
        } else {
          result += "(.*)";
        }
      }
      if (part.type === "segment-wildcard" && !customName) {
        // Unnamed segment-wildcard cannot occur directly without name/regexp except inside edge
      }
      result += escapePatternString(part.suffix);
      if (needsGrouping) {
        result += "}";
      }
      result += mod;
    }
    return result;
  }

  function compileComponent(input, encodeCallback, options) {
    const partList = parsePatternString(input, options, encodeCallback);
    const { regexp, groupNames, hasRegExpGroups } = convertPartListToRegExpAndGroups(
      partList,
      options
    );
    const patternString = generatePatternString(partList, options);
    return {
      patternString,
      partList,
      regexp,
      groupNames,
      hasRegExpGroups,
    };
  }

  // ---------------------------------------------------------------------------
  // URLPatternInit Processing (WHATWG URLPattern Section 1.2)
  // ---------------------------------------------------------------------------
  function readInitComponent(obj, key, skipSymbolCheck) {
    const val = obj[key];
    if (val === undefined) return undefined;
    if (skipSymbolCheck && typeof val !== "string") {
      // Match Node Ada behavior? Check WebIDL USVString:
      // Note: WebIDL converts numbers/null/booleans to string, throws on Symbol.
    }
    if (typeof val === "symbol") {
      throw new TypeError("Cannot convert a Symbol value to a string");
    }
    return String(val);
  }

  function processURLPatternInit(initObj, type, baseURL) {
    const result = {};
    for (const key of COMPONENT_KEYS) {
      result[key] = undefined;
    }

    let baseURLObj = null;
    if (initObj.baseURL !== undefined) {
      baseURL = readInitComponent(initObj, "baseURL");
    }
    if (baseURL !== undefined) {
      if (typeof baseURL === "symbol") {
        throw new TypeError("Cannot convert a Symbol value to a string");
      }
      try {
        baseURLObj = new URLCtor(String(baseURL));
      } catch {
        throw makeCodedError("ERR_INVALID_URL_PATTERN", "Failed to construct URLPattern");
      }
    }

    // Read explicit init fields (excluding baseURL already read)
    if (initObj.protocol !== undefined) {
      let v = readInitComponent(initObj, "protocol");
      if (v.endsWith(":")) v = v.slice(0, -1);
      result.protocol = type === "url" ? canonicalizeProtocol(v) : v;
    }
    if (initObj.username !== undefined) {
      const v = readInitComponent(initObj, "username");
      result.username = type === "url" ? canonicalizeUsername(v) : v;
    }
    if (initObj.password !== undefined) {
      const v = readInitComponent(initObj, "password");
      result.password = type === "url" ? canonicalizePassword(v) : v;
    }
    if (initObj.hostname !== undefined) {
      const v = readInitComponent(initObj, "hostname");
      result.hostname = type === "url" ? canonicalizeHostname(v) : v;
    }
    if (initObj.port !== undefined) {
      const v = readInitComponent(initObj, "port");
      result.port = type === "url" ? canonicalizePort(v, result.protocol) : v;
    }
    if (initObj.pathname !== undefined) {
      result.pathname = readInitComponent(initObj, "pathname");
    }
    if (initObj.search !== undefined) {
      let v = readInitComponent(initObj, "search");
      if (v.startsWith("?")) v = v.slice(1);
      result.search = type === "url" ? canonicalizeSearch(v) : v;
    }
    if (initObj.hash !== undefined) {
      let v = readInitComponent(initObj, "hash");
      if (v.startsWith("#")) v = v.slice(1);
      result.hash = type === "url" ? canonicalizeHash(v) : v;
    }

    function escapedOrRaw(val) {
      return type === "pattern" ? escapePatternString(val) : val;
    }

    if (baseURLObj) {
      const hasProtocol = result.protocol !== undefined;
      const hasUsername = result.username !== undefined;
      const hasPassword = result.password !== undefined;
      const hasHostname = result.hostname !== undefined;
      const hasPort = result.port !== undefined;
      const hasPathname = result.pathname !== undefined;
      const hasSearch = result.search !== undefined;
      const hasHash = result.hash !== undefined;

      if (!hasProtocol) {
        result.protocol = escapedOrRaw(baseURLObj.protocol.slice(0, -1));
      }
      if (type === "url" && !hasProtocol && !hasUsername) {
        result.username = baseURLObj.username;
      }
      if (type === "url" && !hasProtocol && !hasUsername && !hasPassword) {
        result.password = baseURLObj.password;
      }

      if (!hasProtocol && !hasUsername && !hasPassword && !hasHostname) {
        result.hostname = escapedOrRaw(baseURLObj.hostname);
      }

      if (!hasProtocol && !hasUsername && !hasPassword && !hasHostname && !hasPort) {
        result.port = escapedOrRaw(baseURLObj.port);
      }

      if (
        !hasProtocol &&
        !hasUsername &&
        !hasPassword &&
        !hasHostname &&
        !hasPort &&
        !hasPathname
      ) {
        result.pathname = escapedOrRaw(baseURLObj.pathname);
      }

      // Relative pathname resolution against baseURL when hierarchical
      if (
        hasPathname &&
        !hasProtocol &&
        !hasUsername &&
        !hasPassword &&
        !hasHostname &&
        !hasPort &&
        result.pathname[0] !== "/"
      ) {
        const basePath = baseURLObj.pathname;
        const slashIdx = basePath.lastIndexOf("/");
        if (slashIdx >= 0) {
          const prefix = escapedOrRaw(basePath.slice(0, slashIdx + 1));
          result.pathname = prefix + result.pathname;
        }
      }

      if (
        !hasProtocol &&
        !hasUsername &&
        !hasPassword &&
        !hasHostname &&
        !hasPort &&
        !hasPathname &&
        !hasSearch
      ) {
        const s = baseURLObj.search;
        result.search = escapedOrRaw(s.startsWith("?") ? s.slice(1) : s);
      }

      if (
        !hasProtocol &&
        !hasUsername &&
        !hasPassword &&
        !hasHostname &&
        !hasPort &&
        !hasPathname &&
        !hasSearch &&
        !hasHash
      ) {
        const h = baseURLObj.hash;
        result.hash = escapedOrRaw(h.startsWith("#") ? h.slice(1) : h);
      }
    }

    // Apply defaults for omitted components
    if (type === "pattern") {
      for (const key of COMPONENT_KEYS) {
        if (result[key] === undefined) result[key] = "*";
      }
    } else {
      for (const key of COMPONENT_KEYS) {
        if (result[key] === undefined) result[key] = "";
      }
      result.pathname = canonicalizePathname(result.pathname);
    }

    return result;
  }

  // ---------------------------------------------------------------------------
  // Private State Map for URLPattern Instances
  // ---------------------------------------------------------------------------
  const INTERNAL = new WeakMap();

  function buildComponentResult(component, value) {
    const execResult = component.regexp.exec(value);
    if (!execResult) return null;
    const groups = {};
    for (let i = 0; i < component.groupNames.length; i++) {
      groups[component.groupNames[i]] = execResult[i + 1];
    }
    return {
      input: value,
      groups,
    };
  }

  function matchURLPattern(internal, input, baseURL) {
    let protocol = "";
    let username = "";
    let password = "";
    let hostname = "";
    let port = "";
    let pathname = "";
    let search = "";
    let hash = "";
    let inputs;

    if (typeof input === "string") {
      inputs = baseURL !== undefined ? [input, String(baseURL)] : [input];
      let url;
      try {
        url = baseURL !== undefined ? new URLCtor(input, baseURL) : new URLCtor(input);
      } catch {
        return null;
      }
      protocol = url.protocol.slice(0, -1);
      username = url.username;
      password = url.password;
      hostname = url.hostname;
      port = url.port;
      pathname = url.pathname;
      search = url.search.startsWith("?") ? url.search.slice(1) : url.search;
      hash = url.hash.startsWith("#") ? url.hash.slice(1) : url.hash;
    } else {
      if (baseURL !== undefined) {
        throw makeCodedError("ERR_OPERATION_FAILED", "Failed to exec URLPattern");
      }
      let processed;
      try {
        processed = processURLPatternInit(input, "url", undefined);
      } catch {
        return null;
      }
      const snapshot = {};
      for (const key of COMPONENT_KEYS) {
        if (input[key] !== undefined) {
          snapshot[key] = String(input[key]);
        }
      }
      inputs = [snapshot];
      protocol = processed.protocol;
      username = processed.username;
      password = processed.password;
      hostname = processed.hostname;
      port = processed.port;
      pathname = processed.pathname;
      search = processed.search;
      hash = processed.hash;
    }

    const rProtocol = buildComponentResult(internal.protocol, protocol);
    if (!rProtocol) return null;
    const rUsername = buildComponentResult(internal.username, username);
    if (!rUsername) return null;
    const rPassword = buildComponentResult(internal.password, password);
    if (!rPassword) return null;
    const rHostname = buildComponentResult(internal.hostname, hostname);
    if (!rHostname) return null;
    const rPort = buildComponentResult(internal.port, port);
    if (!rPort) return null;
    const rPathname = buildComponentResult(internal.pathname, pathname);
    if (!rPathname) return null;
    const rSearch = buildComponentResult(internal.search, search);
    if (!rSearch) return null;
    const rHash = buildComponentResult(internal.hash, hash);
    if (!rHash) return null;

    return {
      inputs,
      protocol: rProtocol,
      username: rUsername,
      password: rPassword,
      hostname: rHostname,
      port: rPort,
      pathname: rPathname,
      search: rSearch,
      hash: rHash,
    };
  }

  class URLPattern {
    constructor(input = {}, baseURLOrOptions = undefined, options = undefined) {
      let baseURL;
      let opts;

      if (arguments.length === 0 || input === undefined) {
        input = {};
      }

      if (typeof input === "string") {
        if (baseURLOrOptions !== undefined) {
          if (typeof baseURLOrOptions === "string") {
            baseURL = baseURLOrOptions;
            if (options !== undefined) {
              if (options === null || typeof options !== "object") {
                throw makeCodedError("ERR_INVALID_ARG_TYPE", "options must be an object");
              }
              opts = options;
            }
          } else if (baseURLOrOptions !== null && typeof baseURLOrOptions === "object" && !Array.isArray(baseURLOrOptions)) {
            if (arguments.length >= 3 && options !== undefined) {
              throw makeCodedError("ERR_INVALID_ARG_TYPE", "second argument must be a string or object");
            }
            opts = baseURLOrOptions;
          } else {
            throw makeCodedError("ERR_INVALID_ARG_TYPE", "second argument must be a string or object");
          }
        } else if (arguments.length >= 3) {
          throw makeCodedError("ERR_INVALID_ARG_TYPE", "second argument must be a string or object");
        }
        input = parseConstructorString(input);
        if (input.protocol === undefined && baseURL === undefined) {
          throw makeCodedError("ERR_INVALID_URL_PATTERN", "Failed to construct URLPattern");
        }
      } else if (input !== null && typeof input === "object" && !Array.isArray(input)) {
        if (typeof baseURLOrOptions === "string") {
          throw makeCodedError("ERR_INVALID_URL_PATTERN", "Failed to construct URLPattern");
        }
        if (baseURLOrOptions !== undefined) {
          if (baseURLOrOptions === null || Array.isArray(baseURLOrOptions) || typeof baseURLOrOptions !== "object") {
            throw makeCodedError("ERR_INVALID_ARG_TYPE", "second argument must be a string or object");
          }
          opts = baseURLOrOptions;
        }
      } else {
        throw makeCodedError("ERR_INVALID_ARG_TYPE", "Input must be an object or a string");
      }

      const ignoreCase = opts ? Boolean(opts.ignoreCase) : false;
      const processed = processURLPatternInit(input, "pattern", baseURL);

      const protocolComponent = compileComponent(
        processed.protocol,
        canonicalizeProtocol,
        DEFAULT_OPTIONS
      );
      const usernameComponent = compileComponent(
        processed.username,
        canonicalizeUsername,
        DEFAULT_OPTIONS
      );
      const passwordComponent = compileComponent(
        processed.password,
        canonicalizePassword,
        DEFAULT_OPTIONS
      );
      const hostnameComponent = compileComponent(
        processed.hostname,
        canonicalizeHostname,
        HOSTNAME_OPTIONS
      );
      const portComponent = compileComponent(
        processed.port,
        (v) => canonicalizePort(v, processed.protocol === "*" ? "" : processed.protocol),
        DEFAULT_OPTIONS
      );

      let pathnameOptions = PATHNAME_OPTIONS;
      if (!isProtocolPatternSpecialScheme(protocolComponent)) {
        pathnameOptions = DEFAULT_OPTIONS;
      }
      if (ignoreCase) {
        pathnameOptions = { ...pathnameOptions, ignoreCase: true };
      }

      const pathnameEncode = isProtocolPatternSpecialScheme(protocolComponent)
        ? canonicalizePathname
        : canonicalizeOpaquePathname;

      const pathnameComponent = compileComponent(
        processed.pathname,
        pathnameEncode,
        pathnameOptions
      );

      const searchOptions = ignoreCase
        ? { ...DEFAULT_OPTIONS, ignoreCase: true }
        : DEFAULT_OPTIONS;
      const searchComponent = compileComponent(
        processed.search,
        canonicalizeSearch,
        searchOptions
      );

      const hashOptions = ignoreCase
        ? { ...DEFAULT_OPTIONS, ignoreCase: true }
        : DEFAULT_OPTIONS;
      const hashComponent = compileComponent(
        processed.hash,
        canonicalizeHash,
        hashOptions
      );

      INTERNAL.set(this, {
        protocol: protocolComponent,
        username: usernameComponent,
        password: passwordComponent,
        hostname: hostnameComponent,
        port: portComponent,
        pathname: pathnameComponent,
        search: searchComponent,
        hash: hashComponent,
        hasRegExpGroups:
          protocolComponent.hasRegExpGroups ||
          usernameComponent.hasRegExpGroups ||
          passwordComponent.hasRegExpGroups ||
          hostnameComponent.hasRegExpGroups ||
          portComponent.hasRegExpGroups ||
          pathnameComponent.hasRegExpGroups ||
          searchComponent.hasRegExpGroups ||
          hashComponent.hasRegExpGroups,
      });
    }

    get protocol() {
      const state = INTERNAL.get(this);
      if (!state) throw new TypeError("Illegal invocation");
      return state.protocol.patternString;
    }

    get username() {
      const state = INTERNAL.get(this);
      if (!state) throw new TypeError("Illegal invocation");
      return state.username.patternString;
    }

    get password() {
      const state = INTERNAL.get(this);
      if (!state) throw new TypeError("Illegal invocation");
      return state.password.patternString;
    }

    get hostname() {
      const state = INTERNAL.get(this);
      if (!state) throw new TypeError("Illegal invocation");
      return state.hostname.patternString;
    }

    get port() {
      const state = INTERNAL.get(this);
      if (!state) throw new TypeError("Illegal invocation");
      return state.port.patternString;
    }

    get pathname() {
      const state = INTERNAL.get(this);
      if (!state) throw new TypeError("Illegal invocation");
      return state.pathname.patternString;
    }

    get search() {
      const state = INTERNAL.get(this);
      if (!state) throw new TypeError("Illegal invocation");
      return state.search.patternString;
    }

    get hash() {
      const state = INTERNAL.get(this);
      if (!state) throw new TypeError("Illegal invocation");
      return state.hash.patternString;
    }

    get hasRegExpGroups() {
      const state = INTERNAL.get(this);
      if (!state) throw new TypeError("Illegal invocation");
      return state.hasRegExpGroups;
    }

    exec(input = {}, baseURL = undefined) {
      const state = INTERNAL.get(this);
      if (!state) throw new TypeError("Illegal invocation");
      if (input === undefined) input = {};
      if (typeof input !== "string" && (input === null || typeof input !== "object" || Array.isArray(input))) {
        throw makeCodedError("ERR_INVALID_ARG_TYPE", "URLPattern input needs to be a string or an object");
      }
      if (baseURL !== undefined && typeof baseURL !== "string") {
        if (typeof baseURL === "symbol") {
          throw new TypeError("Cannot convert a Symbol value to a string");
        }
        baseURL = String(baseURL);
      }
      return matchURLPattern(state, input, baseURL);
    }

    test(input = {}, baseURL = undefined) {
      return this.exec(input, baseURL) !== null;
    }
  }

  Object.defineProperty(globalObj, "URLPattern", {
    value: URLPattern,
    enumerable: false,
    configurable: true,
    writable: true,
  });
})
