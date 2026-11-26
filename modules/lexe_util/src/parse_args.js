(function () {
  function makeCodeError(code, message) {
    const err = new TypeError(message);
    err.code = code;
    return err;
  }

  function validateBoolean(val, name) {
    if (typeof val !== "boolean") {
      throw makeCodeError(
        "ERR_INVALID_ARG_TYPE",
        `The "${name}" property must be of type boolean. Received type ${typeof val}`,
      );
    }
  }

  function validateOptionType(optName, optCfg) {
    if (typeof optCfg !== "object" || optCfg === null || Array.isArray(optCfg)) {
      throw makeCodeError(
        "ERR_INVALID_ARG_TYPE",
        `The "options.${optName}" property must be an instance of Object.`,
      );
    }
    const { type, short, multiple } = optCfg;
    if (type !== "string" && type !== "boolean") {
      throw makeCodeError(
        "ERR_INVALID_ARG_TYPE",
        `The "options.${optName}.type" property must be ('string|boolean'). Received type ${typeof type} (${String(type)})`,
      );
    }
    if (short !== undefined) {
      if (typeof short !== "string") {
        throw makeCodeError(
          "ERR_INVALID_ARG_TYPE",
          `The "options.${optName}.short" property must be of type string. Received type ${typeof short}`,
        );
      }
      if (short.length !== 1) {
        throw makeCodeError(
          "ERR_INVALID_ARG_VALUE",
          `The property 'options.${optName}.short' must be a single character. Received '${short}'`,
        );
      }
    }
    if (multiple !== undefined) {
      validateBoolean(multiple, `options.${optName}.multiple`);
    }
    if (Object.prototype.hasOwnProperty.call(optCfg, "default") && optCfg.default !== undefined) {
      const def = optCfg.default;
      const expected = type === "boolean" ? "boolean" : "string";
      if (multiple) {
        if (!Array.isArray(def)) {
          throw makeCodeError(
            "ERR_INVALID_ARG_TYPE",
            `The "options.${optName}.default" property must be an instance of Array.`,
          );
        }
        for (let i = 0; i < def.length; i++) {
          if (typeof def[i] !== expected) {
            throw makeCodeError(
              "ERR_INVALID_ARG_TYPE",
              `The "options.${optName}.default[${i}]" property must be of type ${expected}. Received type ${typeof def[i]} (${String(def[i])})`,
            );
          }
        }
      } else if (typeof def !== expected) {
        throw makeCodeError(
          "ERR_INVALID_ARG_TYPE",
          `The "options.${optName}.default" property must be of type ${expected}. Received type ${typeof def} (${String(def)})`,
        );
      }
    }
  }

  function looksLikeOption(arg) {
    return typeof arg === "string" && arg.length > 1 && arg[0] === "-";
  }

  function cloneDefault(val) {
    return Array.isArray(val) ? val.slice() : val;
  }

  function parseArgs(config) {
    if (config === null) {
      throw new TypeError("Cannot convert undefined or null to object");
    }
    const cfg = config === undefined ? Object.create(null) : Object(config);

    const hasArgs = Object.prototype.hasOwnProperty.call(cfg, "args") && cfg.args !== undefined;
    let args;
    if (hasArgs) {
      if (!Array.isArray(cfg.args)) {
        throw makeCodeError(
          "ERR_INVALID_ARG_TYPE",
          `The "args" property must be an instance of Array.`,
        );
      }
      args = cfg.args;
    } else {
      const argv = globalThis.process && Array.isArray(globalThis.process.argv)
        ? globalThis.process.argv
        : [];
      args = argv.slice(2);
    }

    const strict = cfg.strict === undefined ? true : cfg.strict;
    validateBoolean(strict, "strict");

    const allowPositionals =
      cfg.allowPositionals === undefined ? !strict : cfg.allowPositionals;
    validateBoolean(allowPositionals, "allowPositionals");

    const allowNegative = cfg.allowNegative === undefined ? false : cfg.allowNegative;
    validateBoolean(allowNegative, "allowNegative");

    const returnTokens = cfg.tokens === undefined ? false : cfg.tokens;
    validateBoolean(returnTokens, "tokens");

    const rawOptions =
      cfg.options === undefined ? Object.create(null) : cfg.options;
    if (typeof rawOptions !== "object" || rawOptions === null || Array.isArray(rawOptions)) {
      throw makeCodeError(
        "ERR_INVALID_ARG_TYPE",
        `The "options" property must be an instance of Object.`,
      );
    }

    const optionDefs = Object.create(null);
    const shortMap = Object.create(null);
    const optionKeys = Object.keys(rawOptions);
    for (let i = 0; i < optionKeys.length; i++) {
      const longName = optionKeys[i];
      const optCfg = rawOptions[longName];
      validateOptionType(longName, optCfg);
      optionDefs[longName] = optCfg;
      if (optCfg.short !== undefined) {
        shortMap[optCfg.short] = longName;
      }
    }

    const values = Object.create(null);
    const positionals = [];
    const tokens = returnTokens ? [] : null;

    function storeOption(name, rawName, value, inlineValue, index, isBoolean) {
      const def = optionDefs[name];
      const multiple = def ? !!def.multiple : false;
      if (multiple) {
        if (!Array.isArray(values[name])) {
          values[name] = [];
        }
        values[name].push(value);
      } else {
        values[name] = value;
      }
      if (tokens) {
        tokens.push({
          kind: "option",
          name,
          rawName,
          index,
          value: isBoolean ? undefined : value,
          inlineValue: isBoolean ? undefined : inlineValue,
        });
      }
    }

    function pushPositional(val, index) {
      if (!allowPositionals) {
        throw makeCodeError(
          "ERR_PARSE_ARGS_UNEXPECTED_POSITIONAL",
          `Unexpected argument '${val}'. This command does not take positional arguments`,
        );
      }
      positionals.push(val);
      if (tokens) {
        tokens.push({ kind: "positional", index, value: val });
      }
    }

    function takeNextStringValue(rawName, index, optName, shortChar) {
      if (index + 1 >= args.length) {
        throw makeCodeError(
          "ERR_PARSE_ARGS_INVALID_OPTION_VALUE",
          `Option '${rawName} <value>' argument missing`,
        );
      }
      const next = args[index + 1];
      if (strict && looksLikeOption(next)) {
        const hint = shortChar
          ? `To specify an option argument starting with a dash use '--${shortChar}=-XYZ' or '-${shortChar}-XYZ'.`
          : `To specify an option argument starting with a dash use '--${optName}=-XYZ'.`;
        throw makeCodeError(
          "ERR_PARSE_ARGS_INVALID_OPTION_VALUE",
          `Option '${rawName}' argument is ambiguous.\nDid you forget to specify the option argument for '${rawName}'?\n${hint}`,
        );
      }
      return next;
    }

    for (let i = 0; i < args.length; i++) {
      const arg = args[i];

      if (arg === "--") {
        if (tokens) {
          tokens.push({ kind: "option-terminator", index: i });
        }
        for (let j = i + 1; j < args.length; j++) {
          pushPositional(args[j], j);
        }
        break;
      }

      if (typeof arg !== "string" || arg === "-" || arg[0] !== "-") {
        pushPositional(arg, i);
        continue;
      }

      if (arg.startsWith("--")) {
        const eqIdx = arg.indexOf("=");
        const rawName = eqIdx === -1 ? arg : arg.slice(0, eqIdx);
        const hasInline = eqIdx !== -1;
        const inlineVal = hasInline ? arg.slice(eqIdx + 1) : undefined;
        const optName = rawName.slice(2);

        let resolvedName = optName;
        let def = Object.prototype.hasOwnProperty.call(optionDefs, optName)
          ? optionDefs[optName]
          : undefined;
        let negated = false;

        if (!def && allowNegative && optName.startsWith("no-")) {
          const baseName = optName.slice(3);
          const baseDef = Object.prototype.hasOwnProperty.call(optionDefs, baseName)
            ? optionDefs[baseName]
            : undefined;
          if (baseDef) {
            if (baseDef.type !== "boolean") {
              if (strict) {
                throw makeCodeError(
                  "ERR_PARSE_ARGS_UNKNOWN_OPTION",
                  `Unknown option '${rawName}'`,
                );
              }
            } else {
              resolvedName = baseName;
              def = baseDef;
              negated = true;
            }
          } else if (!strict) {
            resolvedName = baseName;
            negated = true;
          }
        }

        if (!def && strict) {
          throw makeCodeError(
            "ERR_PARSE_ARGS_UNKNOWN_OPTION",
            `Unknown option '${rawName}'`,
          );
        }

        const optType = def ? def.type : hasInline ? "string" : "boolean";
        if (optType === "boolean") {
          if (hasInline) {
            const label = negated ? `--${resolvedName}` : rawName;
            throw makeCodeError(
              "ERR_PARSE_ARGS_INVALID_OPTION_VALUE",
              `Option '${label}' does not take an argument`,
            );
          }
          storeOption(resolvedName, rawName, !negated, undefined, i, true);
        } else {
          if (hasInline) {
            storeOption(resolvedName, rawName, inlineVal, true, i, false);
          } else {
            const nextVal = takeNextStringValue(rawName, i, resolvedName, null);
            i += 1;
            storeOption(resolvedName, rawName, nextVal, false, i - 1, false);
          }
        }
        continue;
      }

      // Short option or cluster: `-f`, `-abc`, `-oval`, `-abval`
      for (let k = 1; k < arg.length; k++) {
        const ch = arg[k];
        const rawName = "-" + ch;
        const hasKnown = Object.prototype.hasOwnProperty.call(shortMap, ch);
        if (!hasKnown && strict) {
          throw makeCodeError(
            "ERR_PARSE_ARGS_UNKNOWN_OPTION",
            `Unknown option '${rawName}'`,
          );
        }
        const resolvedName = hasKnown ? shortMap[ch] : ch;
        const def = hasKnown ? optionDefs[resolvedName] : undefined;
        const optType = def ? def.type : "boolean";

        if (optType === "boolean") {
          storeOption(resolvedName, rawName, true, undefined, i, true);
        } else {
          if (k + 1 < arg.length) {
            const rest = arg.slice(k + 1);
            storeOption(resolvedName, rawName, rest, true, i, false);
            break;
          }
          const nextVal = takeNextStringValue(rawName, i, resolvedName, ch);
          i += 1;
          storeOption(resolvedName, rawName, nextVal, false, i - 1, false);
          break;
        }
      }
    }

    for (let i = 0; i < optionKeys.length; i++) {
      const longName = optionKeys[i];
      const def = optionDefs[longName];
      if (
        Object.prototype.hasOwnProperty.call(def, "default") &&
        def.default !== undefined &&
        !Object.prototype.hasOwnProperty.call(values, longName)
      ) {
        values[longName] = cloneDefault(def.default);
      }
    }

    const result = { values, positionals };
    if (tokens) {
      result.tokens = tokens;
    }
    return result;
  }

  return parseArgs;
})()
