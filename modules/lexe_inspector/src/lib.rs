// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//! Minimal Node `inspector` compatibility surface.
//!
//! Lexe does **not** implement the Node Inspector debugging protocol.
//! `url()` always returns `undefined`, and `Session` is exported for callers
//! like `@effect/platform` / profilers that instantiate `new inspector.Session()`:
//! `connect()` / `disconnect()` succeed as no-ops while `post()` invokes its
//! callback / rejects with `ERR_INSPECTOR_NOT_AVAILABLE`.

use lexe_utils::module::{export_default, ModuleInfo};
use rquickjs::{
    module::{Declarations, Exports, ModuleDef},
    prelude::Func,
    Ctx, Function, Result, Undefined, Value,
};

/// Always `undefined`: Lexe has not started a Node Inspector endpoint.
fn url(ctx: Ctx<'_>) -> Result<Value<'_>> {
    Ok(Undefined.into_value(ctx))
}

const SESSION_JS: &str = r#"(function () {
  let Base = class {};
  try {
    const events = require("node:events");
    if (events && typeof events.EventEmitter === "function") {
      Base = events.EventEmitter;
    }
  } catch {}
  class Session extends Base {
    #connected = false;

    connect() {
      this.#connected = true;
    }

    connectToMainThread() {
      this.#connected = true;
    }

    disconnect() {
      this.#connected = false;
    }

    post(method, params, callback) {
      if (typeof params === "function") {
        callback = params;
        params = undefined;
      }
      const err = new Error(
        this.#connected
          ? `Inspector protocol is not available in Lexe (method: ${method})`
          : "Session is not connected"
      );
      err.code = this.#connected ? "ERR_INSPECTOR_NOT_AVAILABLE" : "ERR_INSPECTOR_NOT_CONNECTED";
      if (typeof callback === "function") {
        queueMicrotask(() => callback(err));
        return;
      }
      return Promise.reject(err);
    }
  }
  return Session;
})()"#;

pub struct InspectorModule;

impl ModuleDef for InspectorModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("url")?;
        declare.declare("Session")?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let session_ctor: Function<'js> = ctx.eval(SESSION_JS)?;
        export_default(ctx, exports, |default| {
            default.set("url", Func::from(url))?;
            default.set("Session", session_ctor)?;
            Ok(())
        })?;
        Ok(())
    }
}

impl From<InspectorModule> for ModuleInfo<InspectorModule> {
    fn from(val: InspectorModule) -> Self {
        ModuleInfo {
            name: "inspector",
            module: val,
        }
    }
}

#[cfg(test)]
mod tests {
    use lexe_test::{call_test, test_async_with, ModuleEvaluator};

    use super::*;

    #[tokio::test]
    async fn test_inspector_url_and_session() {
        test_async_with(|ctx| {
            Box::pin(async move {
                ModuleEvaluator::eval_rust::<InspectorModule>(ctx.clone(), "inspector")
                    .await
                    .unwrap();

                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test_inspector",
                    r#"
                        import inspector, { url, Session } from 'inspector';
                        export async function test() {
                          if (typeof url !== 'function') throw new Error('url missing');
                          if (url() !== undefined) throw new Error('url should be undefined');
                          if (inspector.url !== url) throw new Error('default/named url mismatch');
                          if (typeof Session !== 'function' || inspector.Session !== Session) {
                            throw new Error('Session export missing');
                          }
                          const s = new Session();
                          s.connect();
                          let cbErr;
                          await new Promise((resolve) => {
                            s.post('Profiler.enable', (err) => {
                              cbErr = err;
                              resolve();
                            });
                          });
                          if (!cbErr || cbErr.code !== 'ERR_INSPECTOR_NOT_AVAILABLE') {
                            throw new Error('unexpected post callback error');
                          }
                          s.disconnect();
                          return true;
                        }
                    "#,
                )
                .await
                .unwrap();
                let result = call_test::<bool, _>(&ctx, &module, ()).await;
                assert!(result);
            })
        })
        .await;
    }
}
