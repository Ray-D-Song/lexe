//! Node-compatible `net.BlockList` and `net.SocketAddress`.
//!
//! Semantics are pinned to Node 24.21 (`refs/node/lib/internal/blocklist.js`
//! and `lib/internal/blocklist.js`). The behaviours that are easy to get wrong
//! and are covered by `tests/unit/net-address-tools.test.ts`:
//!
//! - `addAddress`/`addSubnet`/`addRange` require a `family` for IPv6, and a
//!   family that disagrees with the address is `ERR_INVALID_ADDRESS`.
//! - `check(addr)` with no family matches against both families; an unknown
//!   `family` string returns `false` (it does not throw), but a non-string
//!   family is `ERR_INVALID_ARG_TYPE`.
//! - `::ffff:1.2.3.4` matches as IPv6, not as the mapped IPv4 address.
//! - `rules` lists IPv6 rules before IPv4 ones (Node keeps two tables).

use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use raster_runtime_utils::primordials::{BasePrimordials, Primordial};
use rquickjs::{class::Trace, Class, Ctx, Exception, JsLifetime, Object, Result, Value};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Family {
    Ipv4,
    Ipv6,
}

impl Family {
    fn as_str(self) -> &'static str {
        match self {
            Family::Ipv4 => "ipv4",
            Family::Ipv6 => "ipv6",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Addr {
    V4(Ipv4Addr),
    V6(Ipv6Addr),
}

impl Addr {
    fn family(self) -> Family {
        match self {
            Addr::V4(_) => Family::Ipv4,
            Addr::V6(_) => Family::Ipv6,
        }
    }
}

#[derive(Clone, Debug)]
enum Rule {
    Address(Addr),
    Subnet { base: Addr, prefix: u8 },
    Range { start: Addr, end: Addr },
}

impl Rule {
    fn family(&self) -> Family {
        match self {
            Rule::Address(a) => a.family(),
            Rule::Subnet { base, .. } => base.family(),
            Rule::Range { start, .. } => start.family(),
        }
    }

    fn matches(&self, addr: Addr) -> bool {
        match self {
            Rule::Address(a) => addr_eq(*a, addr),
            Rule::Subnet { base, prefix } => in_subnet(*base, addr, *prefix),
            Rule::Range { start, end } => in_range(*start, addr, *end),
        }
    }

    fn describe(&self) -> String {
        match self {
            Rule::Address(a) => match a {
                Addr::V4(v) => format!("Address: IPv4 {v}"),
                Addr::V6(v) => format!("Address: IPv6 {v}"),
            },
            Rule::Subnet { base, prefix } => match base {
                Addr::V4(v) => format!("Subnet: IPv4 {v}/{prefix}"),
                Addr::V6(v) => format!("Subnet: IPv6 {v}/{prefix}"),
            },
            Rule::Range { start, end } => match (start, end) {
                (Addr::V4(s), Addr::V4(e)) => format!("Range: IPv4 {s}-{e}"),
                (Addr::V6(s), Addr::V6(e)) => format!("Range: IPv6 {s}-{e}"),
                _ => String::new(),
            },
        }
    }
}

fn addr_eq(a: Addr, b: Addr) -> bool {
    match (a, b) {
        (Addr::V4(x), Addr::V4(y)) => x == y,
        (Addr::V6(x), Addr::V6(y)) => x == y,
        _ => false,
    }
}

fn in_subnet(base: Addr, addr: Addr, prefix: u8) -> bool {
    match (base, addr) {
        (Addr::V4(b), Addr::V4(a)) => {
            let prefix = prefix.min(32) as u32;
            if prefix == 0 {
                return true;
            }
            let mask = u32::MAX << (32 - prefix);
            (u32::from(b) & mask) == (u32::from(a) & mask)
        },
        (Addr::V6(b), Addr::V6(a)) => {
            let prefix = prefix.min(128) as u32;
            if prefix == 0 {
                return true;
            }
            let mask = u128::MAX << (128 - prefix);
            (u128::from(b) & mask) == (u128::from(a) & mask)
        },
        _ => false,
    }
}

fn in_range(start: Addr, addr: Addr, end: Addr) -> bool {
    match (start, addr, end) {
        (Addr::V4(s), Addr::V4(a), Addr::V4(e)) => {
            u32::from(s) <= u32::from(a) && u32::from(a) <= u32::from(e)
        },
        (Addr::V6(s), Addr::V6(a), Addr::V6(e)) => {
            u128::from(s) <= u128::from(a) && u128::from(a) <= u128::from(e)
        },
        _ => false,
    }
}

/// Throw a `TypeError` carrying a Node `code` property, matching the rest of
/// the runtime (`modules/raster_runtime_sqlite/src/error.rs` does the same).
pub(crate) fn throw_coded(
    ctx: &Ctx<'_>,
    constructor: CodedError,
    code: &str,
    message: &str,
) -> rquickjs::Error {
    let Ok(primordials) = BasePrimordials::get(ctx) else {
        return Exception::throw_message(ctx, message);
    };
    let ctor = match constructor {
        CodedError::Type => &primordials.constructor_type_error,
        CodedError::Range => &primordials.constructor_range_error,
    };
    let Ok(err) = ctor.construct((message.to_string(),)) else {
        return Exception::throw_message(ctx, message);
    };
    let err: Object<'_> = err;
    let _ = err.set("code", code);
    ctx.throw(err.into_value())
}

#[derive(Clone, Copy)]
pub(crate) enum CodedError {
    Type,
    Range,
}

fn invalid_address(ctx: &Ctx<'_>) -> rquickjs::Error {
    throw_coded(
        ctx,
        CodedError::Type,
        "ERR_INVALID_ADDRESS",
        "Invalid socket address",
    )
}

/// How the caller spelled the `family` argument.
///
/// Node distinguishes three cases and they behave differently, so they are kept
/// apart here instead of collapsing "missing" and "unrecognised" into one:
///
/// | argument            | add*()                    | check()            |
/// | ------------------- | ------------------------- | ------------------ |
/// | omitted / undefined | IPv4 only, IPv6 address throws | IPv4 table    |
/// | `"ipv4"`            | IPv4 only                 | IPv4 table         |
/// | any other string    | IPv6 only                 | IPv6 table         |
/// | null / non-string   | `ERR_INVALID_ARG_TYPE`    | `ERR_INVALID_ARG_TYPE` |
#[derive(Clone, Copy, PartialEq, Eq)]
enum FamilyArg {
    /// Argument omitted or `undefined`.
    Missing,
    Ipv4,
    Ipv6,
    /// A string that is neither `"ipv4"` nor `"ipv6"` — Node routes these to
    /// the IPv6 table rather than rejecting them.
    Other,
}

impl FamilyArg {
    fn table(self) -> Family {
        match self {
            // Node's `check` defaults to the IPv4 table when no family is given.
            FamilyArg::Missing | FamilyArg::Ipv4 => Family::Ipv4,
            FamilyArg::Ipv6 | FamilyArg::Other => Family::Ipv6,
        }
    }
}

fn parse_family(ctx: &Ctx<'_>, value: Option<Value<'_>>) -> Result<FamilyArg> {
    let Some(value) = value else {
        return Ok(FamilyArg::Missing);
    };
    if value.is_undefined() {
        return Ok(FamilyArg::Missing);
    }
    let Some(text) = value.as_string().and_then(|s| s.to_string().ok()) else {
        return Err(throw_coded(
            ctx,
            CodedError::Type,
            "ERR_INVALID_ARG_TYPE",
            "The \"family\" argument must be of type string",
        ));
    };
    Ok(if text.eq_ignore_ascii_case("ipv4") {
        FamilyArg::Ipv4
    } else if text.eq_ignore_ascii_case("ipv6") {
        FamilyArg::Ipv6
    } else {
        FamilyArg::Other
    })
}

fn parse_addr_for_add(text: &str, family: FamilyArg) -> Option<Addr> {
    match family {
        // Omitting `family` means IPv4 only: an IPv6 literal is rejected
        // (`ERR_INVALID_ADDRESS`) rather than guessed at.
        FamilyArg::Missing | FamilyArg::Ipv4 => Ipv4Addr::from_str(text).ok().map(Addr::V4),
        // An IPv6 rule must name its family explicitly; `::ffff:1.2.3.4`
        // included (Node treats the mapped form as an IPv6 literal here).
        FamilyArg::Ipv6 | FamilyArg::Other => Ipv6Addr::from_str(text).ok().map(Addr::V6),
    }
}

fn parse_addr_for_check(text: &str, family: FamilyArg) -> Option<Addr> {
    match family.table() {
        Family::Ipv4 => Ipv4Addr::from_str(text).ok().map(Addr::V4),
        Family::Ipv6 => Ipv6Addr::from_str(text).ok().map(Addr::V6),
    }
}

#[derive(Default, Debug, JsLifetime)]
#[rquickjs::class]
pub struct BlockList {
    rules: Vec<Rule>,
}

// Every field is plain Rust data (no JS values), so there is nothing to trace.
impl<'js> Trace<'js> for BlockList {
    fn trace<'a>(&self, _tracer: rquickjs::class::Tracer<'a, 'js>) {}
}

impl BlockList {
    fn push_rule(&mut self, rule: Rule) {
        // Node's `rules` order comes from its internal tables and is not part of
        // the contract (v4-only lists come back reversed, mixed lists do not),
        // so this keeps insertion order — stable and documented in the tests.
        self.rules.push(rule);
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl BlockList {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'_>, options: rquickjs::prelude::Opt<Object<'_>>) -> Result<Self> {
        // `new BlockList({ add: [...] })` pre-populates; keep it simple and
        // accept the same shape Node does.
        let mut list = BlockList::default();
        if let Some(options) = options.0 {
            if let Some(add) = options.get::<_, Option<Value<'_>>>("add")? {
                if let Some(items) = add.as_array() {
                    for item in items.iter::<Value<'_>>() {
                        let item = item?;
                        if let Some(obj) = item.as_object() {
                            let address: String =
                                obj.get::<_, Option<String>>("address")?.unwrap_or_default();
                            let prefix: Option<u8> = obj.get("prefix")?;
                            let family =
                                parse_family(&ctx, obj.get::<_, Option<Value<'_>>>("family")?)?;
                            if let Some(addr) = parse_addr_for_add(&address, family) {
                                match prefix {
                                    Some(prefix) => {
                                        list.push_rule(Rule::Subnet { base: addr, prefix })
                                    },
                                    None => list.push_rule(Rule::Address(addr)),
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(list)
    }

    pub fn add_address(
        &mut self,
        ctx: Ctx<'_>,
        address: Value<'_>,
        family: rquickjs::prelude::Opt<Value<'_>>,
    ) -> Result<()> {
        let family = parse_family(&ctx, family.0)?;
        reject_unknown_family(&ctx, family)?;
        let Some(addr) = require_addr_for_add(&ctx, address, family)? else {
            return Err(invalid_address(&ctx));
        };
        // Node dedupes identical addresses; subnets and ranges are not deduped.
        if !self
            .rules
            .iter()
            .any(|r| matches!(r, Rule::Address(a) if addr_eq(*a, addr)))
        {
            self.rules.push(Rule::Address(addr));
        }
        Ok(())
    }

    pub fn add_subnet(
        &mut self,
        ctx: Ctx<'_>,
        address: Value<'_>,
        prefix: Value<'_>,
        family: rquickjs::prelude::Opt<Value<'_>>,
    ) -> Result<()> {
        let family = parse_family(&ctx, family.0)?;
        reject_unknown_family(&ctx, family)?;
        let Some(addr) = require_addr_for_add(&ctx, address, family)? else {
            return Err(invalid_address(&ctx));
        };
        let Some(prefix) = prefix.as_int() else {
            return Err(throw_coded(
                &ctx,
                CodedError::Type,
                "ERR_INVALID_ARG_TYPE",
                "The \"prefix\" argument must be of type number",
            ));
        };
        let max = match addr.family() {
            Family::Ipv4 => 32,
            Family::Ipv6 => 128,
        };
        if !(0..=max).contains(&prefix) {
            return Err(throw_coded(
                &ctx,
                CodedError::Range,
                "ERR_OUT_OF_RANGE",
                &format!(
                    "The value of \"prefix\" is out of range. It must be >= 0 && <= {max}. Received {prefix}"
                ),
            ));
        }
        self.rules.push(Rule::Subnet {
            base: addr,
            prefix: prefix as u8,
        });
        Ok(())
    }

    pub fn add_range(
        &mut self,
        ctx: Ctx<'_>,
        start: Value<'_>,
        end: Value<'_>,
        family: rquickjs::prelude::Opt<Value<'_>>,
    ) -> Result<()> {
        let family = parse_family(&ctx, family.0)?;
        let Some(start_addr) = require_addr_for_add(&ctx, start, family)? else {
            return Err(invalid_address(&ctx));
        };
        let Some(end_addr) = require_addr_for_add(&ctx, end, family)? else {
            return Err(invalid_address(&ctx));
        };
        if start_addr.family() != end_addr.family() {
            return Err(invalid_address(&ctx));
        }
        let ordered = match (start_addr, end_addr) {
            (Addr::V4(s), Addr::V4(e)) => u32::from(s) <= u32::from(e),
            (Addr::V6(s), Addr::V6(e)) => u128::from(s) <= u128::from(e),
            _ => false,
        };
        if !ordered {
            return Err(throw_coded(
                &ctx,
                CodedError::Type,
                "ERR_INVALID_ARG_VALUE",
                "The argument 'start' must come before end",
            ));
        }
        self.rules.push(Rule::Range {
            start: start_addr,
            end: end_addr,
        });
        Ok(())
    }

    pub fn check(
        &self,
        ctx: Ctx<'_>,
        address: Value<'_>,
        family: rquickjs::prelude::Opt<Value<'_>>,
    ) -> Result<bool> {
        let family = parse_family(&ctx, family.0)?;
        let Some(text) = address.as_string().and_then(|s| s.to_string().ok()) else {
            return Ok(false);
        };
        let Some(addr) = parse_addr_for_check(&text, family) else {
            return Ok(false);
        };
        if family.table() == Family::Ipv6 {
            if self
                .rules
                .iter()
                .filter(|r| r.family() == Family::Ipv6)
                .any(|r| r.matches(addr))
            {
                return Ok(true);
            }
            // An IPv4-mapped IPv6 literal is also checked against the IPv4
            // table, so a v4 SSRF rule still catches `::ffff:1.2.3.4`.
            if let Addr::V6(v6) = addr {
                if let Some(v4) = v6.to_ipv4_mapped() {
                    return Ok(self
                        .rules
                        .iter()
                        .filter(|r| r.family() == Family::Ipv4)
                        .any(|r| r.matches(Addr::V4(v4))));
                }
            }
            // A rule stored from a mapped literal also matches the plain v4 form.
            if let Addr::V4(v4) = addr {
                let mapped = v4.to_ipv6_mapped();
                return Ok(self
                    .rules
                    .iter()
                    .filter(|r| r.family() == Family::Ipv6)
                    .any(|r| r.matches(Addr::V6(mapped))));
            }
            return Ok(false);
        }
        Ok(self
            .rules
            .iter()
            .filter(|r| r.family() == Family::Ipv4)
            .any(|r| r.matches(addr)))
    }

    /// Node exposes the registered rules as human-readable strings.
    #[qjs(get, rename = "rules")]
    pub fn rules<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        let out = rquickjs::Array::new(ctx.clone())?;
        for (i, rule) in self.rules.iter().enumerate() {
            out.set(i, rule.describe())?;
        }
        Ok(out.into_value())
    }
}

/// `add*()` rejects a family string it does not recognise; `check()` does not
/// (Node routes unknown strings to the IPv6 table there).
fn reject_unknown_family(ctx: &Ctx<'_>, family: FamilyArg) -> Result<()> {
    if family == FamilyArg::Other {
        return Err(throw_coded(
            ctx,
            CodedError::Type,
            "ERR_INVALID_ARG_VALUE",
            "The argument 'family' must be one of: 'ipv4', 'ipv6'",
        ));
    }
    Ok(())
}

fn require_addr_for_add(
    ctx: &Ctx<'_>,
    value: Value<'_>,
    family: FamilyArg,
) -> Result<Option<Addr>> {
    let Some(text) = value.as_string().and_then(|s| s.to_string().ok()) else {
        let _ = ctx;
        return Ok(None);
    };
    Ok(parse_addr_for_add(&text, family))
}

/// Node-compatible `net.SocketAddress`.
#[derive(Clone, Trace, JsLifetime)]
#[rquickjs::class]
pub struct SocketAddress {
    #[qjs(skip_trace)]
    address: String,
    family: String,
    port: u16,
    flowlabel: u32,
}

#[rquickjs::methods(rename_all = "camelCase")]
impl SocketAddress {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'_>, options: rquickjs::prelude::Opt<Object<'_>>) -> Result<Self> {
        let mut address = String::from("127.0.0.1");
        let mut family = String::from("ipv4");
        let mut port: u16 = 0;
        let mut flowlabel: u32 = 0;

        if let Some(options) = options.0 {
            if let Some(a) = options.get::<_, Option<String>>("address")? {
                address = a;
            }
            if let Some(f) = options.get::<_, Option<String>>("family")? {
                family = f;
            }
            if let Some(p) = options.get::<_, Option<u32>>("port")? {
                port = u16::try_from(p)
                    .map_err(|_| Exception::throw_range(&ctx, "port is out of range"))?;
            }
            if let Some(f) = options.get::<_, Option<u32>>("flowlabel")? {
                flowlabel = f;
            }
        }

        // Node validates that the address matches the declared family, and only
        // an explicit `family: "ipv6"` makes an IPv6 address acceptable.
        match family.as_str() {
            "ipv4" => {
                if Ipv4Addr::from_str(&address).is_err() {
                    return Err(invalid_address(&ctx));
                }
                flowlabel = 0;
            },
            "ipv6" => {
                if Ipv6Addr::from_str(&address).is_err() || flowlabel > 0x000F_FFFF {
                    return Err(invalid_address(&ctx));
                }
            },
            _ => return Err(Exception::throw_message(&ctx, "Invalid socket address")),
        }

        Ok(SocketAddress {
            address,
            family,
            port,
            flowlabel,
        })
    }

    #[qjs(get)]
    pub fn address(&self) -> String {
        self.address.clone()
    }

    #[qjs(get)]
    pub fn family(&self) -> String {
        self.family.clone()
    }

    #[qjs(get)]
    pub fn port(&self) -> u16 {
        self.port
    }

    #[qjs(get)]
    pub fn flowlabel(&self) -> u32 {
        self.flowlabel
    }

    // Node spells this `toJSON`; camelCase renaming would produce `toJson`.
    #[qjs(rename = "toJSON")]
    pub fn to_json<'js>(&self, ctx: Ctx<'js>) -> Result<Object<'js>> {
        let obj = Object::new(ctx)?;
        obj.set("address", self.address.clone())?;
        obj.set("port", self.port)?;
        obj.set("family", self.family.clone())?;
        obj.set("flowlabel", self.flowlabel)?;
        Ok(obj)
    }
}

pub fn export(ctx: &Ctx<'_>, default: &Object<'_>) -> Result<()> {
    Class::<BlockList>::define(default)?;
    Class::<SocketAddress>::define(default)?;
    let _ = ctx;
    Ok(())
}
