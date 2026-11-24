// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use rquickjs::{Ctx, Function, Object, Result, Value};

/// Normalize `tls.connect()` overloads into a single options object and optional callback.
pub fn normalize_connect_args<'js>(
    ctx: &Ctx<'js>,
    mut args: Vec<Value<'js>>,
) -> Result<(Value<'js>, Option<Function<'js>>)> {
    let mut callback = None;

    if args.is_empty() {
        return Ok((Object::new(ctx.clone())?.into_value(), callback));
    }

    if args.len() == 1 {
        let first = args.remove(0);
        if first.is_function() {
            callback = first.into_function();
            return Ok((Object::new(ctx.clone())?.into_value(), callback));
        }
        return Ok((first, callback));
    }

    if let Some(port) = args.first().and_then(|v| v.as_int()) {
        args.remove(0);
        let mut host = String::from("localhost");
        let options = Object::new(ctx.clone())?;

        let next = args.first().cloned();
        if let Some(next) = next {
            if next.is_function() {
                callback = args.remove(0).into_function();
            } else if let Some(h) = next.as_string() {
                args.remove(0);
                host = h.to_string()?;
                if let Some(third) = args.first() {
                    if third.is_function() {
                        callback = args.remove(0).into_function();
                    } else if third.is_object() {
                        let extra = args.remove(0).into_object().unwrap();
                        for key in extra.keys::<String>() {
                            let key = key?;
                            let val = extra.get::<_, Value>(&key)?;
                            options.set(key.as_str(), val)?;
                        }
                        callback = args.first().and_then(|v| v.clone().into_function());
                    }
                }
            } else if next.is_object() {
                let extra = args.remove(0).into_object().unwrap();
                for key in extra.keys::<String>() {
                    let key = key?;
                    let val = extra.get::<_, Value>(&key)?;
                    options.set(key.as_str(), val)?;
                }
                callback = args.first().and_then(|v| v.clone().into_function());
            }
        }
        options.set("port", port)?;
        options.set("host", host)?;
        return Ok((options.into_value(), callback));
    }

    if let Some(last) = args.last() {
        if last.is_function() {
            callback = args.pop().and_then(|v| v.into_function());
        }
    }
    let options = args
        .into_iter()
        .next()
        .unwrap_or_else(|| Object::new(ctx.clone()).unwrap().into_value());
    Ok((options, callback))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rquickjs::{Context, IntoJs, Runtime};

    #[test]
    fn normalizes_port_host_options_callback() {
        let rt = Runtime::new().unwrap();
        let ctx = Context::full(&rt).unwrap();
        ctx.with(|ctx| {
            let opts = Object::new(ctx.clone()).unwrap();
            opts.set("minVersion", "TLSv1.2").unwrap();
            let cb =
                rquickjs::Function::new(ctx.clone(), |_: ()| Ok::<_, rquickjs::Error>(())).unwrap();
            let (value, callback) = normalize_connect_args(
                &ctx,
                vec![
                    3306i32.into_js(&ctx).unwrap(),
                    "db.local".into_js(&ctx).unwrap(),
                    opts.into_value(),
                    cb.into_value(),
                ],
            )
            .unwrap();
            let obj = value.as_object().unwrap();
            assert_eq!(obj.get::<_, u16>("port").unwrap(), 3306);
            assert_eq!(obj.get::<_, String>("host").unwrap(), "db.local");
            assert_eq!(obj.get::<_, String>("minVersion").unwrap(), "TLSv1.2");
            assert!(callback.is_some());
        });
    }

    #[test]
    fn positional_port_overrides_options_port() {
        let rt = Runtime::new().unwrap();
        let ctx = Context::full(&rt).unwrap();
        ctx.with(|ctx| {
            let opts = Object::new(ctx.clone()).unwrap();
            opts.set("port", 80i32).unwrap();
            let (value, _) = normalize_connect_args(
                &ctx,
                vec![443i32.into_js(&ctx).unwrap(), opts.into_value()],
            )
            .unwrap();
            let obj = value.as_object().unwrap();
            assert_eq!(obj.get::<_, u16>("port").unwrap(), 443);
        });
    }
}
