// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use lexe_utils::result::ResultExt;
use lexe_utils::{any_of::AnyOf4, bytes::ObjectBytes, object::ObjectExt};
use rquickjs::{
    prelude::{Opt, This},
    Class, Ctx, Error, FromJs, Object, Result, Value,
};

use crate::HyperClient;

#[rquickjs::class]
#[derive(rquickjs::JsLifetime, rquickjs::class::Trace)]
pub struct Agent {
    #[qjs(skip_trace)]
    client: HyperClient,
    #[qjs(skip_trace)]
    keep_alive: bool,
    #[qjs(skip_trace)]
    keep_alive_msecs: u64,
    #[qjs(skip_trace)]
    max_sockets: f64,
    #[qjs(skip_trace)]
    max_free_sockets: usize,
    #[qjs(skip_trace)]
    max_total_sockets: f64,
}

impl Agent {
    pub fn client(&self) -> HyperClient {
        self.client.clone()
    }

    pub(crate) fn new_global<'js>(ctx: &Ctx<'js>) -> Result<Class<'js, Self>> {
        let opts = Object::new(ctx.clone())?;
        opts.set("keepAlive", true)?;
        Self::new(ctx.clone(), Opt(Some(opts.into_value())))
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl Agent {
    #[qjs(constructor)]
    pub fn new<'js>(ctx: Ctx<'js>, options_val: Opt<Value<'js>>) -> Result<Class<'js, Self>> {
        let mut reject_unauthorized = true;
        let mut ca = None;
        let mut keep_alive = false;
        let mut keep_alive_msecs = 1000u64;
        let mut max_sockets = f64::INFINITY;
        let mut max_free_sockets = 256usize;
        let mut max_total_sockets = f64::INFINITY;

        let raw_options = match options_val.0 {
            Some(v) if !v.is_undefined() && !v.is_null() => {
                let parsed = AgentOptions::from_js(&ctx, v.clone())?;
                if let Some(opt_reject) = parsed.reject_unauthorized {
                    reject_unauthorized = opt_reject;
                }
                if let Some(opt_ca) = parsed.ca {
                    ca = Some(opt_ca);
                }
                if let Some(opt_ka) = parsed.keep_alive {
                    keep_alive = opt_ka;
                }
                if let Some(opt_ms) = parsed.keep_alive_msecs {
                    keep_alive_msecs = opt_ms;
                }
                if let Some(opt_max) = parsed.max_sockets {
                    max_sockets = opt_max;
                }
                if let Some(opt_mfs) = parsed.max_free_sockets {
                    max_free_sockets = opt_mfs;
                }
                if let Some(opt_mts) = parsed.max_total_sockets {
                    max_total_sockets = opt_mts;
                }
                v.as_object()
                    .cloned()
                    .unwrap_or_else(|| Object::new(ctx.clone()).expect("object"))
            },
            _ => Object::new(ctx.clone())?,
        };

        let config = lexe_tls::build_client_config(lexe_tls::BuildClientConfigOptions {
            reject_unauthorized,
            ca,
        })
        .or_throw_msg(&ctx, "Failed to build TLS config")?;
        let client =
            crate::build_client(Some(config)).or_throw_msg(&ctx, "Failed to build HTTP client")?;

        let instance = Class::instance(
            ctx.clone(),
            Self {
                client,
                keep_alive,
                keep_alive_msecs,
                max_sockets,
                max_free_sockets,
                max_total_sockets,
            },
        )?;
        instance.set("options", raw_options)?;
        instance.set("sockets", Object::new(ctx.clone())?)?;
        instance.set("freeSockets", Object::new(ctx.clone())?)?;
        instance.set("requests", Object::new(ctx)?)?;
        Ok(instance)
    }

    #[qjs(get, enumerable)]
    fn protocol(&self) -> &'static str {
        "https:"
    }

    #[qjs(get, enumerable)]
    fn default_port(&self) -> u16 {
        443
    }

    #[qjs(get, enumerable)]
    fn keep_alive(&self) -> bool {
        self.keep_alive
    }

    #[qjs(set, rename = "keepAlive")]
    fn set_keep_alive(&mut self, value: bool) {
        self.keep_alive = value;
    }

    #[qjs(get, enumerable)]
    fn keep_alive_msecs(&self) -> u64 {
        self.keep_alive_msecs
    }

    #[qjs(get, enumerable)]
    fn max_sockets(&self) -> f64 {
        self.max_sockets
    }

    #[qjs(set, rename = "maxSockets")]
    fn set_max_sockets(&mut self, value: f64) {
        self.max_sockets = value;
    }

    #[qjs(get, enumerable)]
    fn max_free_sockets(&self) -> usize {
        self.max_free_sockets
    }

    #[qjs(set, rename = "maxFreeSockets")]
    fn set_max_free_sockets(&mut self, value: usize) {
        self.max_free_sockets = value;
    }

    #[qjs(get, enumerable)]
    fn max_total_sockets(&self) -> f64 {
        self.max_total_sockets
    }

    #[qjs(set, rename = "maxTotalSockets")]
    fn set_max_total_sockets(&mut self, value: f64) {
        self.max_total_sockets = value;
    }

    pub fn get_name(&self, options: Opt<Object<'_>>) -> Result<String> {
        let Some(opts) = options.0 else {
            return Ok("localhost:443:".to_string());
        };
        let host = opts
            .get_optional::<_, String>("host")?
            .or(opts.get_optional::<_, String>("hostname")?)
            .unwrap_or_else(|| "localhost".to_string());
        let port = if let Some(val) = opts.get_optional::<_, Value>("port")? {
            if let Some(n) = val.as_number() {
                format!("{}", n as u16)
            } else if let Some(s) = val.as_string() {
                s.to_string()?
            } else {
                String::new()
            }
        } else {
            String::new()
        };
        let local_addr = opts
            .get_optional::<_, String>("localAddress")?
            .unwrap_or_default();
        Ok(format!("{host}:{port}:{local_addr}"))
    }

    pub fn destroy<'js>(this: This<Class<'js, Self>>, ctx: Ctx<'js>) -> Result<()> {
        this.set("sockets", Object::new(ctx.clone())?)?;
        this.set("freeSockets", Object::new(ctx.clone())?)?;
        this.set("requests", Object::new(ctx)?)?;
        Ok(())
    }
}

pub struct AgentOptions {
    reject_unauthorized: Option<bool>,
    ca: Option<Vec<Vec<u8>>>,
    keep_alive: Option<bool>,
    keep_alive_msecs: Option<u64>,
    max_sockets: Option<f64>,
    max_free_sockets: Option<usize>,
    max_total_sockets: Option<f64>,
}

impl<'js> FromJs<'js> for AgentOptions {
    fn from_js(ctx: &Ctx<'js>, value: Value<'js>) -> Result<Self> {
        let ty_name = value.type_name();
        let obj = value
            .as_object()
            .ok_or(Error::new_from_js(ty_name, "Object"))?;

        let reject_unauthorized = obj.get_optional::<_, bool>("rejectUnauthorized")?;
        let keep_alive = obj.get_optional::<_, bool>("keepAlive")?;
        let keep_alive_msecs = obj
            .get_optional::<_, f64>("keepAliveMsecs")?
            .filter(|n| n.is_finite() && *n >= 0.0)
            .map(|n| n as u64);
        let max_sockets = obj.get_optional::<_, f64>("maxSockets")?;
        let max_free_sockets = obj
            .get_optional::<_, f64>("maxFreeSockets")?
            .filter(|n| n.is_finite() && *n >= 0.0)
            .map(|n| n as usize);
        let max_total_sockets = obj.get_optional::<_, f64>("maxTotalSockets")?;
        let ca = obj
            .get_optional::<_, AnyOf4<String, Vec<String>, ObjectBytes, Vec<ObjectBytes>>>("ca")?
            .map(|ca| {
                let ca = match ca {
                    AnyOf4::A(ca) => vec![ca.into_bytes()],
                    AnyOf4::B(ca) => ca.into_iter().map(|ca| ca.into_bytes()).collect(),
                    AnyOf4::C(ca) => vec![ca.into_bytes(ctx)?],
                    AnyOf4::D(ca) => ca
                        .into_iter()
                        .map(|ca| ca.into_bytes(ctx))
                        .collect::<Result<Vec<_>>>()?,
                };
                Ok::<_, Error>(ca)
            })
            .transpose()?;

        Ok(Self {
            reject_unauthorized,
            ca,
            keep_alive,
            keep_alive_msecs,
            max_sockets,
            max_free_sockets,
            max_total_sockets,
        })
    }
}
