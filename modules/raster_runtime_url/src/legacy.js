// Legacy `node:url` API: `Url`, `parse`, `resolve`, `resolveObject`, the
// string/object forms of `format`, and `fileURLToPathBuffer`.
//
// A port of refs/node/lib/url.js and the relevant parts of
// refs/node/lib/internal/url.js, internal/querystring.js and
// internal/data_url.js. Evaluated once per `node:url` instance.
(function (natives) {
  "use strict";

  const { toASCII, domainToUnicode } = natives;

  function makeError(Base, code, message, extra) {
    const error = new Base(message);
    Object.defineProperty(error, "code", {
      value: code,
      enumerable: true,
      configurable: true,
      writable: true,
    });
    if (extra) Object.assign(error, extra);
    return error;
  }

  function describeReceived(value) {
    if (value === undefined) return "undefined";
    if (value === null) return "null";
    if (typeof value === "function") {
      return value.name ? `function ${value.name}` : "function ";
    }
    if (typeof value === "object") {
      const name = value.constructor && value.constructor.name;
      return name ? `an instance of ${name}` : "[Object: null prototype]";
    }
    let shown = String(value);
    if (typeof value === "string") {
      if (shown.length > 28) shown = `${shown.slice(0, 25)}...`;
      shown = `'${shown}'`;
    }
    return `type ${typeof value} (${shown})`;
  }

  function invalidArgType(name, expected, value) {
    return makeError(
      TypeError,
      "ERR_INVALID_ARG_TYPE",
      `The "${name}" argument must be ${expected}. Received ${describeReceived(value)}`
    );
  }

  function invalidUrl(input) {
    return makeError(TypeError, "ERR_INVALID_URL", "Invalid URL", { input });
  }

  function validateString(value, name) {
    if (typeof value !== "string") throw invalidArgType(name, "of type string", value);
  }

  function validateObject(value, name) {
    if (value === null || Array.isArray(value) || typeof value !== "object") {
      throw invalidArgType(name, "of type object", value);
    }
  }

  const unsafeProtocol = new Set(["javascript", "javascript:"]);
  const hostlessProtocol = new Set(["javascript", "javascript:"]);
  const slashedProtocol = new Set([
    "http",
    "http:",
    "https",
    "https:",
    "ftp",
    "ftp:",
    "gopher",
    "gopher:",
    "file",
    "file:",
    "ws",
    "ws:",
    "wss",
    "wss:",
  ]);

  const querystring = () => globalThis.require("querystring");

  const CHAR_SPACE = 32;
  const CHAR_TAB = 9;
  const CHAR_CARRIAGE_RETURN = 13;
  const CHAR_LINE_FEED = 10;
  const CHAR_NO_BREAK_SPACE = 160;
  const CHAR_ZERO_WIDTH_NOBREAK_SPACE = 65279;
  const CHAR_HASH = 35;
  const CHAR_FORWARD_SLASH = 47;
  const CHAR_LEFT_SQUARE_BRACKET = 91;
  const CHAR_RIGHT_SQUARE_BRACKET = 93;
  const CHAR_LEFT_ANGLE_BRACKET = 60;
  const CHAR_RIGHT_ANGLE_BRACKET = 62;
  const CHAR_LEFT_CURLY_BRACKET = 123;
  const CHAR_RIGHT_CURLY_BRACKET = 125;
  const CHAR_QUESTION_MARK = 63;
  const CHAR_DOUBLE_QUOTE = 34;
  const CHAR_SINGLE_QUOTE = 39;
  const CHAR_PERCENT = 37;
  const CHAR_SEMICOLON = 59;
  const CHAR_BACKWARD_SLASH = 92;
  const CHAR_CIRCUMFLEX_ACCENT = 94;
  const CHAR_GRAVE_ACCENT = 96;
  const CHAR_VERTICAL_LINE = 124;
  const CHAR_AT = 64;
  const CHAR_COLON = 58;

  function spliceOne(list, index) {
    for (; index + 1 < list.length; index++) list[index] = list[index + 1];
    list.pop();
  }

  const hexTable = new Array(256);
  for (let i = 0; i < 256; ++i) {
    hexTable[i] = "%" + (i < 16 ? "0" : "") + i.toString(16).toUpperCase();
  }

  function encodeStr(str, noEscapeTable, hexTable) {
    const len = str.length;
    if (len === 0) return "";

    let out = "";
    let lastPos = 0;
    let i = 0;

    outer: for (; i < len; i++) {
      let c = str.charCodeAt(i);

      while (c < 0x80) {
        if (noEscapeTable[c] !== 1) {
          if (lastPos < i) out += str.slice(lastPos, i);
          lastPos = i + 1;
          out += hexTable[c];
        }

        if (++i === len) break outer;

        c = str.charCodeAt(i);
      }

      if (lastPos < i) out += str.slice(lastPos, i);

      if (c < 0x800) {
        lastPos = i + 1;
        out += hexTable[0xc0 | (c >> 6)] + hexTable[0x80 | (c & 0x3f)];
        continue;
      }
      if (c < 0xd800 || c >= 0xe000) {
        lastPos = i + 1;
        out +=
          hexTable[0xe0 | (c >> 12)] +
          hexTable[0x80 | ((c >> 6) & 0x3f)] +
          hexTable[0x80 | (c & 0x3f)];
        continue;
      }
      ++i;

      if (i >= len) throw makeError(URIError, "ERR_INVALID_URI", "URI malformed");

      const c2 = str.charCodeAt(i) & 0x3ff;

      lastPos = i + 1;
      c = 0x10000 + (((c & 0x3ff) << 10) | c2);
      out +=
        hexTable[0xf0 | (c >> 18)] +
        hexTable[0x80 | ((c >> 12) & 0x3f)] +
        hexTable[0x80 | ((c >> 6) & 0x3f)] +
        hexTable[0x80 | (c & 0x3f)];
    }
    if (lastPos === 0) return str;
    if (lastPos < len) return out + str.slice(lastPos);
    return out;
  }

  // ---------------------------------------------------------------------- Url

  function Url() {
    this.protocol = null;
    this.slashes = null;
    this.auth = null;
    this.host = null;
    this.port = null;
    this.hostname = null;
    this.hash = null;
    this.search = null;
    this.query = null;
    this.pathname = null;
    this.path = null;
    this.href = null;
  }

  const protocolPattern = /^[a-z0-9.+-]+:/i;
  const portPattern = /:[0-9]*$/;
  const hostPattern = /^\/\/[^@/]+@[^@/]+/;
  const simplePathPattern = /^(\/\/?(?!\/)[^?\s]*)(\?[^\s]*)?$/;
  const hostnameMaxLen = 255;

  function insideNodeModules() {
    const stack = new Error().stack;
    return typeof stack === "string" && /[\\/]node_modules[\\/]/.test(stack);
  }

  let urlParseWarned = false;

  function urlParse(url, parseQueryString, slashesDenoteHost) {
    if (!urlParseWarned && !insideNodeModules()) {
      urlParseWarned = true;
      if (typeof process === "object" && typeof process.emitWarning === "function") {
        process.emitWarning(
          "`url.parse()` behavior is not standardized and prone to " +
            "errors that have security implications. Use the WHATWG URL API " +
            "instead. CVEs are not issued for `url.parse()` vulnerabilities.",
          "DeprecationWarning",
          "DEP0169"
        );
      }
    }

    if (url instanceof Url) return url;

    const urlObject = new Url();
    urlObject.parse(url, parseQueryString, slashesDenoteHost);
    return urlObject;
  }

  function isIpv6Hostname(hostname) {
    return (
      hostname.charCodeAt(0) === CHAR_LEFT_SQUARE_BRACKET &&
      hostname.charCodeAt(hostname.length - 1) === CHAR_RIGHT_SQUARE_BRACKET
    );
  }

  const forbiddenHostChars = /[\0\t\n\r #%/:<>?@[\\\]^|]/;
  const forbiddenHostCharsIpv6 = /[\0\t\n\r #%/<>?@\\^|]/;

  Url.prototype.parse = function parse(url, parseQueryString, slashesDenoteHost) {
    validateString(url, "url");

    let hasHash = false;
    let hasAt = false;
    let start = -1;
    let end = -1;
    let rest = "";
    let lastPos = 0;
    for (let i = 0, inWs = false, split = false; i < url.length; ++i) {
      const code = url.charCodeAt(i);

      const isWs = code < 33 || code === CHAR_NO_BREAK_SPACE || code === CHAR_ZERO_WIDTH_NOBREAK_SPACE;
      if (start === -1) {
        if (isWs) continue;
        lastPos = start = i;
      } else if (inWs) {
        if (!isWs) {
          end = -1;
          inWs = false;
        }
      } else if (isWs) {
        end = i;
        inWs = true;
      }

      if (!split) {
        switch (code) {
          case CHAR_AT:
            hasAt = true;
            break;
          case CHAR_HASH:
            hasHash = true;
          // Fall through
          case CHAR_QUESTION_MARK:
            split = true;
            break;
          case CHAR_BACKWARD_SLASH:
            if (i - lastPos > 0) rest += url.slice(lastPos, i);
            rest += "/";
            lastPos = i + 1;
            break;
        }
      } else if (!hasHash && code === CHAR_HASH) {
        hasHash = true;
      }
    }

    if (start !== -1) {
      if (lastPos === start) {
        if (end === -1) {
          if (start === 0) rest = url;
          else rest = url.slice(start);
        } else {
          rest = url.slice(start, end);
        }
      } else if (end === -1 && lastPos < url.length) {
        rest += url.slice(lastPos);
      } else if (end !== -1 && lastPos < end) {
        rest += url.slice(lastPos, end);
      }
    }

    if (!slashesDenoteHost && !hasHash && !hasAt) {
      const simplePath = simplePathPattern.exec(rest);
      if (simplePath) {
        this.path = rest;
        this.href = rest;
        this.pathname = simplePath[1];
        if (simplePath[2]) {
          this.search = simplePath[2];
          if (parseQueryString) {
            this.query = querystring().parse(this.search.slice(1));
          } else {
            this.query = this.search.slice(1);
          }
        } else if (parseQueryString) {
          this.search = null;
          this.query = { __proto__: null };
        }
        return this;
      }
    }

    let proto = protocolPattern.exec(rest);
    let lowerProto;
    if (proto) {
      proto = proto[0];
      lowerProto = proto.toLowerCase();
      this.protocol = lowerProto;
      rest = rest.slice(proto.length);
    }

    let slashes;
    if (slashesDenoteHost || proto || hostPattern.test(rest)) {
      slashes = rest.charCodeAt(0) === CHAR_FORWARD_SLASH && rest.charCodeAt(1) === CHAR_FORWARD_SLASH;
      if (slashes && !(proto && hostlessProtocol.has(lowerProto))) {
        rest = rest.slice(2);
        this.slashes = true;
      }
    }

    if (!hostlessProtocol.has(lowerProto) && (slashes || (proto && !slashedProtocol.has(proto)))) {
      let hostEnd = -1;
      let atSign = -1;
      let nonHost = -1;
      for (let i = 0; i < rest.length; ++i) {
        switch (rest.charCodeAt(i)) {
          case CHAR_TAB:
          case CHAR_LINE_FEED:
          case CHAR_CARRIAGE_RETURN:
            rest = rest.slice(0, i) + rest.slice(i + 1);
            i -= 1;
            break;
          case CHAR_SPACE:
          case CHAR_DOUBLE_QUOTE:
          case CHAR_PERCENT:
          case CHAR_SINGLE_QUOTE:
          case CHAR_SEMICOLON:
          case CHAR_LEFT_ANGLE_BRACKET:
          case CHAR_RIGHT_ANGLE_BRACKET:
          case CHAR_BACKWARD_SLASH:
          case CHAR_CIRCUMFLEX_ACCENT:
          case CHAR_GRAVE_ACCENT:
          case CHAR_LEFT_CURLY_BRACKET:
          case CHAR_VERTICAL_LINE:
          case CHAR_RIGHT_CURLY_BRACKET:
            if (nonHost === -1) nonHost = i;
            break;
          case CHAR_HASH:
          case CHAR_FORWARD_SLASH:
          case CHAR_QUESTION_MARK:
            if (nonHost === -1) nonHost = i;
            hostEnd = i;
            break;
          case CHAR_AT:
            atSign = i;
            nonHost = -1;
            break;
        }
        if (hostEnd !== -1) break;
      }
      start = 0;
      if (atSign !== -1) {
        this.auth = decodeURIComponent(rest.slice(0, atSign));
        start = atSign + 1;
      }
      if (nonHost === -1) {
        this.host = rest.slice(start);
        rest = "";
      } else {
        this.host = rest.slice(start, nonHost);
        rest = rest.slice(nonHost);
      }

      this.parseHost();

      if (typeof this.hostname !== "string") this.hostname = "";

      const hostname = this.hostname;
      const ipv6Hostname = isIpv6Hostname(hostname);

      if (!ipv6Hostname) {
        rest = getHostname(this, rest, hostname, url);
      }

      if (this.hostname.length > hostnameMaxLen) {
        this.hostname = "";
      } else {
        this.hostname = this.hostname.toLowerCase();
      }

      if (this.hostname !== "") {
        if (ipv6Hostname) {
          if (forbiddenHostCharsIpv6.test(this.hostname)) {
            throw invalidUrl(url);
          }
        } else {
          this.hostname = toASCII(this.hostname);

          if (this.hostname === "" || forbiddenHostChars.test(this.hostname)) {
            throw invalidUrl(url);
          }
        }
      }

      const p = this.port ? ":" + this.port : "";
      const h = this.hostname || "";
      this.host = h + p;

      if (ipv6Hostname) {
        this.hostname = this.hostname.slice(1, -1);
        if (rest[0] !== "/") {
          rest = "/" + rest;
        }
      }
    }

    if (!unsafeProtocol.has(lowerProto)) {
      rest = autoEscapeStr(rest);
    }

    let questionIdx = -1;
    let hashIdx = -1;
    for (let i = 0; i < rest.length; ++i) {
      const code = rest.charCodeAt(i);
      if (code === CHAR_HASH) {
        this.hash = rest.slice(i);
        hashIdx = i;
        break;
      } else if (code === CHAR_QUESTION_MARK && questionIdx === -1) {
        questionIdx = i;
      }
    }

    if (questionIdx !== -1) {
      if (hashIdx === -1) {
        this.search = rest.slice(questionIdx);
        this.query = rest.slice(questionIdx + 1);
      } else {
        this.search = rest.slice(questionIdx, hashIdx);
        this.query = rest.slice(questionIdx + 1, hashIdx);
      }
      if (parseQueryString) {
        this.query = querystring().parse(this.query);
      }
    } else if (parseQueryString) {
      this.search = null;
      this.query = { __proto__: null };
    }

    const useQuestionIdx = questionIdx !== -1 && (hashIdx === -1 || questionIdx < hashIdx);
    const firstIdx = useQuestionIdx ? questionIdx : hashIdx;
    if (firstIdx === -1) {
      if (rest.length > 0) this.pathname = rest;
    } else if (firstIdx > 0) {
      this.pathname = rest.slice(0, firstIdx);
    }
    if (slashedProtocol.has(lowerProto) && this.hostname && !this.pathname) {
      this.pathname = "/";
    }

    if (this.pathname || this.search) {
      const p = this.pathname || "";
      const s = this.search || "";
      this.path = p + s;
    }

    this.href = this.format();
    return this;
  };

  let warnInvalidPort = true;
  function getHostname(self, rest, hostname, url) {
    for (let i = 0; i < hostname.length; ++i) {
      const code = hostname.charCodeAt(i);
      const isValid =
        code !== CHAR_FORWARD_SLASH &&
        code !== CHAR_BACKWARD_SLASH &&
        code !== CHAR_HASH &&
        code !== CHAR_QUESTION_MARK &&
        code !== CHAR_COLON;

      if (!isValid) {
        if (warnInvalidPort && code === CHAR_COLON) {
          const detail = `The URL ${url} is invalid. Future versions of Node.js will throw an error.`;
          if (typeof process === "object" && typeof process.emitWarning === "function") {
            process.emitWarning(detail, "DeprecationWarning", "DEP0170");
          }
          warnInvalidPort = false;
        }
        self.hostname = hostname.slice(0, i);
        return `/${hostname.slice(i)}${rest}`;
      }
    }
    return rest;
  }

  const escapedCodes = [
    /* 0 - 9 */ "", "", "", "", "", "", "", "", "", "%09",
    /* 10 - 19 */ "%0A", "", "", "%0D", "", "", "", "", "", "",
    /* 20 - 29 */ "", "", "", "", "", "", "", "", "", "",
    /* 30 - 39 */ "", "", "%20", "", "%22", "", "", "", "", "%27",
    /* 40 - 49 */ "", "", "", "", "", "", "", "", "", "",
    /* 50 - 59 */ "", "", "", "", "", "", "", "", "", "",
    /* 60 - 69 */ "%3C", "", "%3E", "", "", "", "", "", "", "",
    /* 70 - 79 */ "", "", "", "", "", "", "", "", "", "",
    /* 80 - 89 */ "", "", "", "", "", "", "", "", "", "",
    /* 90 - 99 */ "", "", "%5C", "", "%5E", "", "%60", "", "", "",
    /* 100 - 109 */ "", "", "", "", "", "", "", "", "", "",
    /* 110 - 119 */ "", "", "", "", "", "", "", "", "", "",
    /* 120 - 125 */ "", "", "", "%7B", "%7C", "%7D",
  ];

  function autoEscapeStr(rest) {
    let escaped = "";
    let lastEscapedPos = 0;
    for (let i = 0; i < rest.length; ++i) {
      const escapedChar = escapedCodes[rest.charCodeAt(i)];
      if (escapedChar) {
        if (i > lastEscapedPos) escaped += rest.slice(lastEscapedPos, i);
        escaped += escapedChar;
        lastEscapedPos = i + 1;
      }
    }
    if (lastEscapedPos === 0) return rest;

    if (lastEscapedPos < rest.length) escaped += rest.slice(lastEscapedPos);

    return escaped;
  }

  function urlFormat(urlObject, options) {
    if (typeof urlObject === "string") {
      urlObject = urlParse(urlObject);
    } else if (typeof urlObject !== "object" || urlObject === null) {
      throw invalidArgType("urlObject", "one of type object or string", urlObject);
    } else if (urlObject instanceof URL) {
      return formatURLObject(urlObject, options);
    }

    return Url.prototype.format.call(urlObject);
  }

  // `bindingUrl.format()`: serialize `href` with parts left out or the host shown as Unicode.
  function formatURLObject(url, options) {
    let fragment = true;
    let unicode = false;
    let search = true;
    let auth = true;

    if (options) {
      validateObject(options, "options");
      if (options.fragment != null) fragment = Boolean(options.fragment);
      if (options.unicode != null) unicode = Boolean(options.unicode);
      if (options.search != null) search = Boolean(options.search);
      if (options.auth != null) auth = Boolean(options.auth);
    }

    const copy = new URL(url.href);
    if (!fragment) copy.hash = "";
    if (!search) copy.search = "";
    if (!auth) {
      copy.username = "";
      copy.password = "";
    }
    let href = copy.href;
    if (unicode && copy.hostname !== "") {
      const hostStart = href.indexOf(copy.host, copy.protocol.length);
      href = href.slice(0, hostStart) + domainToUnicode(copy.hostname) + href.slice(hostStart + copy.hostname.length);
    }
    return href;
  }

  const noEscapeAuth = new Int8Array([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 1, 0, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 1, 1, 0,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0,
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 1,
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 1, 0,
  ]);

  Url.prototype.format = function format() {
    let auth = this.auth || "";
    if (auth) {
      auth = encodeStr(auth, noEscapeAuth, hexTable);
      auth += "@";
    }

    let protocol = this.protocol || "";
    if (protocol && protocol.charCodeAt(protocol.length - 1) !== 58 /* : */) {
      protocol += ":";
    }

    let pathname = this.pathname || "";
    let hash = this.hash || "";
    let host = "";
    let query = "";

    if (this.host) {
      host = auth + this.host;
    } else if (this.hostname) {
      host =
        auth +
        (this.hostname.indexOf(":") !== -1 && !isIpv6Hostname(this.hostname)
          ? "[" + this.hostname + "]"
          : this.hostname);
      if (this.port) {
        host += ":" + this.port;
      }
    }

    if (this.query !== null && typeof this.query === "object") {
      query = querystring().stringify(this.query);
    }
    let search = this.search || (query && "?" + query) || "";

    if (pathname.indexOf("#") !== -1 || pathname.indexOf("?") !== -1) {
      let newPathname = "";
      let lastPos = 0;
      const len = pathname.length;
      for (let i = 0; i < len; i++) {
        const code = pathname.charCodeAt(i);
        if (code === CHAR_HASH || code === CHAR_QUESTION_MARK) {
          if (i > lastPos) {
            newPathname += pathname.slice(lastPos, i);
          }
          newPathname += code === CHAR_HASH ? "%23" : "%3F";
          lastPos = i + 1;
        }
      }
      if (lastPos < len) {
        newPathname += pathname.slice(lastPos);
      }
      pathname = newPathname;
    }

    if (this.slashes || slashedProtocol.has(protocol)) {
      if (this.slashes || host) {
        if (pathname && pathname.charCodeAt(0) !== CHAR_FORWARD_SLASH) pathname = "/" + pathname;
        host = "//" + host;
      } else if (
        protocol.length >= 4 &&
        protocol.charCodeAt(0) === 102 /* f */ &&
        protocol.charCodeAt(1) === 105 /* i */ &&
        protocol.charCodeAt(2) === 108 /* l */ &&
        protocol.charCodeAt(3) === 101 /* e */
      ) {
        host = "//";
      }
    }

    if (search.indexOf("#") !== -1) {
      search = search.replaceAll("#", "%23");
    }

    if (hash && hash.charCodeAt(0) !== CHAR_HASH) {
      hash = "#" + hash;
    }
    if (search && search.charCodeAt(0) !== CHAR_QUESTION_MARK) {
      search = "?" + search;
    }

    return protocol + host + pathname + search + hash;
  };

  function urlResolve(source, relative) {
    return urlParse(source, false, true).resolve(relative);
  }

  Url.prototype.resolve = function resolve(relative) {
    return this.resolveObject(urlParse(relative, false, true)).format();
  };

  function urlResolveObject(source, relative) {
    if (!source) return relative;
    return urlParse(source, false, true).resolveObject(relative);
  }

  Url.prototype.resolveObject = function resolveObject(relative) {
    if (typeof relative === "string") {
      const rel = new Url();
      rel.parse(relative, false, true);
      relative = rel;
    }

    const result = new Url();
    Object.assign(result, this);

    result.hash = relative.hash;

    if (relative.href === "") {
      result.href = result.format();
      return result;
    }

    if (relative.slashes && !relative.protocol) {
      const relativeWithoutProtocol = Object.keys(relative).reduce((acc, key) => {
        if (key !== "protocol") {
          acc[key] = relative[key];
        }
        return acc;
      }, {});
      Object.assign(result, relativeWithoutProtocol);

      if (slashedProtocol.has(result.protocol) && result.hostname && !result.pathname) {
        result.path = result.pathname = "/";
      }

      result.href = result.format();
      return result;
    }

    if (relative.protocol && relative.protocol !== result.protocol) {
      if (!slashedProtocol.has(relative.protocol)) {
        Object.assign(result, relative);
        result.href = result.format();
        return result;
      }

      result.protocol = relative.protocol;
      if (!relative.host && !/^file:?$/.test(relative.protocol) && !hostlessProtocol.has(relative.protocol)) {
        const relPath = (relative.pathname || "").split("/");
        while (relPath.length && !(relative.host = relPath.shift()));
        relative.host ||= "";
        relative.hostname ||= "";
        if (relPath[0] !== "") relPath.unshift("");
        if (relPath.length < 2) relPath.unshift("");
        result.pathname = relPath.join("/");
      } else {
        result.pathname = relative.pathname;
      }
      result.search = relative.search;
      result.query = relative.query;
      result.host = relative.host || "";
      result.auth = relative.auth;
      result.hostname = relative.hostname || relative.host;
      result.port = relative.port;
      if (result.pathname || result.search) {
        const p = result.pathname || "";
        const s = result.search || "";
        result.path = p + s;
      }
      result.slashes ||= relative.slashes;
      result.href = result.format();
      return result;
    }

    const isSourceAbs = result.pathname && result.pathname.charAt(0) === "/";
    const isRelAbs = relative.host || (relative.pathname && relative.pathname.charAt(0) === "/");
    let mustEndAbs = isRelAbs || isSourceAbs || (result.host && relative.pathname);
    const removeAllDots = mustEndAbs;
    let srcPath = (result.pathname && result.pathname.split("/")) || [];
    const relPath = (relative.pathname && relative.pathname.split("/")) || [];
    const noLeadingSlashes = result.protocol && !slashedProtocol.has(result.protocol);

    if (noLeadingSlashes) {
      result.hostname = "";
      result.port = null;
      if (result.host) {
        if (srcPath[0] === "") srcPath[0] = result.host;
        else srcPath.unshift(result.host);
      }
      result.host = "";
      if (relative.protocol) {
        relative.hostname = null;
        relative.port = null;
        result.auth = null;
        if (relative.host) {
          if (relPath[0] === "") relPath[0] = relative.host;
          else relPath.unshift(relative.host);
        }
        relative.host = null;
      }
      mustEndAbs &&= relPath[0] === "" || srcPath[0] === "";
    }

    if (isRelAbs) {
      if (relative.host || relative.host === "") {
        if (result.host !== relative.host) result.auth = null;
        result.host = relative.host;
        result.port = relative.port;
      }
      if (relative.hostname || relative.hostname === "") {
        if (result.hostname !== relative.hostname) result.auth = null;
        result.hostname = relative.hostname;
      }
      result.search = relative.search;
      result.query = relative.query;
      srcPath = relPath;
    } else if (relPath.length) {
      srcPath ||= [];
      srcPath.pop();
      srcPath = srcPath.concat(relPath);
      result.search = relative.search;
      result.query = relative.query;
    } else if (relative.search !== null && relative.search !== undefined) {
      if (noLeadingSlashes) {
        result.hostname = result.host = srcPath.shift();
        const authInHost = result.host && result.host.indexOf("@") > 0 && result.host.split("@");
        if (authInHost) {
          result.auth = authInHost.shift();
          result.host = result.hostname = authInHost.shift();
        }
      }
      result.search = relative.search;
      result.query = relative.query;
      if (result.pathname !== null || result.search !== null) {
        result.path = (result.pathname ? result.pathname : "") + (result.search ? result.search : "");
      }
      result.href = result.format();
      return result;
    }

    if (!srcPath.length) {
      result.pathname = null;
      if (result.search) {
        result.path = "/" + result.search;
      } else {
        result.path = null;
      }
      result.href = result.format();
      return result;
    }

    let last = srcPath.slice(-1)[0];
    const hasTrailingSlash =
      ((result.host || relative.host || srcPath.length > 1) && (last === "." || last === "..")) || last === "";

    let up = 0;
    for (let i = srcPath.length - 1; i >= 0; i--) {
      last = srcPath[i];
      if (last === ".") {
        spliceOne(srcPath, i);
      } else if (last === "..") {
        spliceOne(srcPath, i);
        up++;
      } else if (up) {
        spliceOne(srcPath, i);
        up--;
      }
    }

    if (!mustEndAbs && !removeAllDots) {
      while (up--) {
        srcPath.unshift("..");
      }
    }

    if (mustEndAbs && srcPath[0] !== "" && (!srcPath[0] || srcPath[0].charAt(0) !== "/")) {
      srcPath.unshift("");
    }

    if (hasTrailingSlash && srcPath.join("/").slice(-1) !== "/") {
      srcPath.push("");
    }

    const isAbsolute = srcPath[0] === "" || (srcPath[0] && srcPath[0].charAt(0) === "/");

    if (noLeadingSlashes) {
      result.hostname = result.host = isAbsolute ? "" : srcPath.length ? srcPath.shift() : "";
      const authInHost = result.host && result.host.indexOf("@") > 0 ? result.host.split("@") : false;
      if (authInHost) {
        result.auth = authInHost.shift();
        result.host = result.hostname = authInHost.shift();
      }
    }

    mustEndAbs ||= result.host && srcPath.length;

    if (mustEndAbs && !isAbsolute) {
      srcPath.unshift("");
    }

    if (!srcPath.length) {
      result.pathname = null;
      result.path = null;
    } else {
      result.pathname = srcPath.join("/");
    }

    if (result.pathname !== null || result.search !== null) {
      result.path = (result.pathname ? result.pathname : "") + (result.search ? result.search : "");
    }
    result.auth = relative.auth || result.auth;
    result.slashes ||= relative.slashes;
    result.href = result.format();
    return result;
  };

  Url.prototype.parseHost = function parseHost() {
    let host = this.host;
    let port = portPattern.exec(host);
    if (port) {
      port = port[0];
      if (port !== ":") {
        this.port = port.slice(1);
      }
      host = host.slice(0, host.length - port.length);
    }
    if (host) this.hostname = host;
  };

  // ------------------------------------------------------ fileURLToPathBuffer

  const isHexByte = (b) => (b >= 0x30 && b <= 0x39) || (b >= 0x41 && b <= 0x46) || (b >= 0x61 && b <= 0x66);
  const hexValue = (b) => (b <= 0x39 ? b - 0x30 : (b | 0x20) - 0x61 + 10);

  // https://url.spec.whatwg.org/#percent-decode
  function percentDecode(input) {
    const length = input.length;
    const output = new Uint8Array(length);
    let j = 0;
    for (let i = 0; i < length; ++i) {
      const byte = input[i];
      if (byte !== 0x25) {
        output[j++] = byte;
      } else if (!(isHexByte(input[i + 1]) && isHexByte(input[i + 2]))) {
        output[j++] = 0x25;
      } else {
        output[j++] = (hexValue(input[i + 1]) << 4) | hexValue(input[i + 2]);
        i += 2;
      }
    }
    return length === j ? output : output.subarray(0, j);
  }

  function toBuffer(bytes) {
    return Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  }

  function getPathBufferFromURLWin32(url) {
    const hostname = url.hostname;
    let pathname = url.pathname;
    pathname = pathname.replace(/\//g, "\\");
    const decodedPathname = toBuffer(percentDecode(Buffer.from(pathname, "utf8")));
    if (hostname !== "") {
      return Buffer.concat([Buffer.from("\\\\", "ascii"), Buffer.from(domainToUnicode(hostname), "utf8"), decodedPathname]);
    }
    const letter = decodedPathname[1] | 0x20;
    const sep = decodedPathname[2];
    if (letter < 0x61 || letter > 0x7a || sep !== 0x3a) {
      throw makeError(TypeError, "ERR_INVALID_FILE_URL_PATH", "File URL path must be absolute", { input: url.href });
    }
    return decodedPathname.subarray(1);
  }

  function getPathBufferFromURLPosix(url) {
    if (url.hostname !== "") {
      throw makeError(TypeError, "ERR_INVALID_FILE_URL_HOST", 'File URL host must be "localhost" or empty on linux');
    }
    return toBuffer(percentDecode(Buffer.from(url.pathname, "utf8")));
  }

  function fileURLToPathBuffer(path, options = {}) {
    const windows = options?.windows;
    let href = path;
    if (typeof path === "string") {
      try {
        path = new URL(path);
      } catch (error) {
        throw invalidUrl(href);
      }
    } else if (!(path instanceof URL)) {
      throw invalidArgType("path", "of type string or an instance of URL", path);
    }
    if (path.protocol !== "file:") {
      throw makeError(TypeError, "ERR_INVALID_URL_SCHEME", "The URL must be of scheme file");
    }
    return (windows ?? process.platform === "win32")
      ? getPathBufferFromURLWin32(path)
      : getPathBufferFromURLPosix(path);
  }

  return { Url, parse: urlParse, resolve: urlResolve, resolveObject: urlResolveObject, format: urlFormat, fileURLToPathBuffer };
})
