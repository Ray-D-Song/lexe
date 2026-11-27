// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use lexe_events::{Emitter, EventEmitter};
use lexe_net::{Server as NetServer, Socket as NetSocket};
use lexe_utils::module::{export_default, ModuleInfo};
use rquickjs::{
    module::{Declarations, Exports, ModuleDef},
    prelude::Func,
    Class, Ctx, Exception, Function, IntoJs, Object, Result, Undefined, Value,
};

use crate::alpn::convert_alpn_protocols_export;
use crate::backend::supported_ciphers;
use crate::client::connect;
use crate::identity::check_server_identity;
use crate::secure_context::{create_js_secure_context, JsSecureContext};
use crate::server::{create_server, Server};
use crate::tls_socket::TlsSocket;

const CLIENT_RENEG_LIMIT: i32 = 3;
const CLIENT_RENEG_WINDOW: i32 = 600;
const DEFAULT_ECDH_CURVE: &str = "auto";

/// JS implementation of `tls.TLSSocket`.
///
/// Extends the JS `net.Socket` class, so:
///   `s instanceof tls.TLSSocket` — true
///   `s instanceof net.Socket`    — true
///   `s instanceof stream.Duplex` — true
///
/// The native `TlsSocket` is only the backend (`_handle`), exactly like
/// `net.Socket`. `allowHalfOpen` is permanently true on the native side: TLS
/// ends one direction at a time (`close_notify`), so the native read loop must
/// never shut the write half. The user-visible behaviour is decided here, in
/// `writeAfterFIN`.
const TLS_SOCKET_GLUE_JS: &str = r#"(function () {
  "use strict";

  function build(createHandle, NetSocket) {
    class TLSSocket extends NetSocket {      constructor(socket, options) {
        const opts = options || {};
        // Callers that already own a native handle (`tls.connect` performs the
        // handshake itself) pass `_handleFactory`; otherwise this class creates
        // its own backend through the factory the module supplied.
        const supplied = opts._handleFactory;
        super({
          // Only forward a factory when the caller already owns a handle.
          // Otherwise `net.Socket` resolves `this.constructor._createHandle`,
          // which the TLSSocket class sets to the TLS backend factory.
          ...(supplied ? { _handleFactory: supplied } : {}),
          allowHalfOpen: opts.allowHalfOpen === true,
        });

        // STARTTLS: wrap an existing `net.Socket`. Its transport has to move to
        // this TLS handle, which is asynchronous, so record the source and let
        // `_starttlsHandoff` drive it once the caller is ready (Node does the
        // same through `TLSSocket` + `beginTlsHandoff`).
        this._tlsWrappedSocket =
          socket && typeof socket._handle === "object" && socket._handle
            ? socket
            : null;
        if (this._tlsWrappedSocket) {
          this.connecting = true;
          this.pending = false;
          this._starttlsOptions = opts;
          // `new tls.TLSSocket(socket, { isServer: true, ... })` (STARTTLS): the
          // accepted plain socket's transport moves to this TLS handle. Deferred
          // to keep the constructor synchronous, exactly like Node's
          // `beginTlsHandoff`.
          if (opts.isServer) {
            const source = this._tlsWrappedSocket;
            process.nextTick(() => {
              const target = this._handle;
              if (target && typeof target.starttlsServer === "function") {
                target.starttlsServer(source, opts);
              }
            });
          }
        }
        this._secureEstablished = false;
        this._controlReleased = false;
        this._servername = opts.servername || null;

        // Node installs `writeAfterFIN` when the peer ends a non-half-open
        // socket; `_onEof` applies it, since the native side now always keeps
        // the write half open (TLS ends one direction at a time).
        this.on("end", () => {
          if (!this.allowHalfOpen) this._onReadableStreamEnd();
        });

        // Wake the native reader when the consumer drains, same as net.Socket.
        this.on("resume", () => {
          const handle = this._handle;
          if (handle && typeof handle.resumeReading === "function") {
            handle.resumeReading();
          }
        });

        // TLS decrypts inbound records inside the native layer, so the byte
        // counter has to be fed where plaintext is handed to the readable
        // side. Wrapping `push` keeps that in JS and leaves the shared read
        // loop untouched.
        const push = this.push;
        this.push = (chunk, encoding) => {
          if (chunk !== null && chunk !== undefined) {
            this.bytesRead += chunk.length;
          }
          return push.call(this, chunk, encoding);
        };
      }

      _onReadableStreamEnd() {
        if (!this.allowHalfOpen) {
          // Node: a non-half-open socket ends its write side when the peer
          // ends, then turns later writes into EPIPE.
          this.write = writeAfterFIN;
          if (!this.writableEnded) this.end();
        }
      }

      // Native read loop reached EOF: end the readable side, then apply the
      // half-open policy for the write side.
      _onEof() {
        if (this._readableState && this._readableState.ended) return;
        this.push(null);
        if (this._readableState && this._readableState.flowing !== true) {
          this.resume();
        }
        this._onReadableStreamEnd();
      }

      // --- TLS-only surface -------------------------------------------------

      get _secureEstablishedFlag() {
        return this._secureEstablished;
      }

      get encrypted() {
        return true;
      }

      get secureConnecting() {
        if (this._secureEstablished) return false;
        const handle = this._handle;
        return handle ? handle.secureConnecting : true;
      }

      get servername() {
        const handle = this._handle;
        return (handle && handle.servername) || this._servername || null;
      }

      get alpnProtocol() {
        const handle = this._handle;
        return handle ? handle.alpnProtocol : false;
      }

      get authorized() {
        const handle = this._handle;
        return handle ? handle.authorized : false;
      }

      get authorizationError() {
        const handle = this._handle;
        return handle ? handle.authorizationError : null;
      }

      setSecureEstablished() {
        this._secureEstablished = true;
        this._controlReleased = true;
      }

      getPeerCertificate(detailed) {
        const handle = this._handle;
        if (!handle || typeof handle.getPeerCertificate !== "function") return {};
        return handle.getPeerCertificate(!!detailed);
      }

      getCertificate() {
        const handle = this._handle;
        if (!handle || typeof handle.getCertificate !== "function") return {};
        return handle.getCertificate();
      }

      getCipher() {
        const handle = this._handle;
        if (!handle || typeof handle.getCipher !== "function") return {};
        return handle.getCipher();
      }

      getProtocol() {
        const handle = this._handle;
        if (!handle || typeof handle.getProtocol !== "function") return null;
        return handle.getProtocol();
      }

      isSessionReused() {
        const handle = this._handle;
        if (!handle || typeof handle.isSessionReused !== "function") return false;
        return handle.isSessionReused();
      }

      getSession() {
        const handle = this._handle;
        if (!handle || typeof handle.getSession !== "function") return undefined;
        return handle.getSession();
      }

      getTLSTicket() {
        const handle = this._handle;
        if (!handle || typeof handle.getTLSTicket !== "function") return undefined;
        return handle.getTLSTicket();
      }

      // `setServername` before a handshake reconnects with SNI; after it, TLS
      // forbids renegotiation, so the request is ignored like Node.
      setServername(name) {
        if (this._secureEstablished) return this;
        this._servername = name;
        return this;
      }

      // A TLS socket has no half-open negotiation: renegotiation is a no-op
      // unless the handle supports it.
      renegotiate(options, cb) {
        const handle = this._handle;
        if (handle && typeof handle.renegotiate === "function") {
          return handle.renegotiate(options, cb);
        }
        if (typeof cb === "function") process.nextTick(cb, new Error("renegotiation not supported"));
        return false;
      }
    }

    // Node's `net.Socket` error for a write after the peer's FIN.
    function writeAfterFIN(chunk, encoding, cb) {
      if (!this.writableEnded) {
        return NetSocket.prototype.write.call(this, chunk, encoding, cb);
      }
      if (typeof encoding === "function") {
        cb = encoding;
        encoding = null;
      }
      const er = new Error("This socket has been ended by the other party");
      er.code = "EPIPE";
      if (typeof cb === "function") process.nextTick(cb, er);
      this.destroy(er);
      return false;
    }

    TLSSocket.prototype._writeAfterFIN = writeAfterFIN;
    // Every TLSSocket — plain `new TLSSocket()` included — must own a TLS
    // backend, not inherit net.Socket's. Subclasses of the net class read this
    // static through `this.constructor`.
    TLSSocket._createHandle = (opts) => createHandle(opts);

    return TLSSocket;
  }

  return { build };
})()"#;

/// Resolve the JS `net.Socket` class emitted by the net module.
///
/// The tls module can be evaluated before `node:net` is imported, so pull it in
/// on demand; the net module publishes the class on the global it always sets.
fn require_net_socket_class<'js>(ctx: &Ctx<'js>) -> Result<Value<'js>> {
    let existing: Value = ctx.globals().get("__lexeNetSocketClass")?;
    if existing.is_function() {
        return Ok(existing);
    }
    ctx.eval::<(), _>(r#"require("node:net")"#)?;
    ctx.globals().get("__lexeNetSocketClass")
}

fn link_prototypes<'js>(ctx: &Ctx<'js>, _default: &Object<'js>) -> Result<()> {
    if let (Some(tls_proto), Some(net_proto)) = (
        Class::<TlsSocket>::prototype(ctx)?,
        Class::<NetSocket>::prototype(ctx)?,
    ) {
        tls_proto.set_prototype(Some(&net_proto))?;
    }
    if let (Some(tls_proto), Some(net_proto)) = (
        Class::<Server>::prototype(ctx)?,
        Class::<NetServer>::prototype(ctx)?,
    ) {
        tls_proto.set_prototype(Some(&net_proto))?;
    }
    Ok(())
}

fn default_ciphers() -> String {
    supported_ciphers().join(":")
}

fn check_server_identity_export<'js>(
    ctx: Ctx<'js>,
    hostname: String,
    cert: Object<'js>,
) -> Result<Value<'js>> {
    let cert_obj = crate::certificate::cert_object_from_js(&ctx, &cert)
        .map_err(|e| Exception::throw_message(&ctx, &e))?;
    match check_server_identity(&ctx, &hostname, &cert_obj)? {
        Some(err) => Ok(err),
        None => Undefined.into_js(&ctx),
    }
}

pub struct TlsModule;

impl ModuleDef for TlsModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("connect")?;
        declare.declare("createServer")?;
        declare.declare("createSecureContext")?;
        declare.declare("checkServerIdentity")?;
        declare.declare("getCiphers")?;
        declare.declare("getCACertificates")?;
        declare.declare("rootCertificates")?;
        declare.declare("convertALPNProtocols")?;
        declare.declare("CLIENT_RENEG_LIMIT")?;
        declare.declare("CLIENT_RENEG_WINDOW")?;
        declare.declare("DEFAULT_CIPHERS")?;
        declare.declare("DEFAULT_ECDH_CURVE")?;
        declare.declare("DEFAULT_MIN_VERSION")?;
        declare.declare("DEFAULT_MAX_VERSION")?;
        declare.declare("TLSSocket")?;
        declare.declare("Server")?;
        declare.declare("SecureContext")?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        export_default(ctx, exports, |default| {
            Class::<JsSecureContext>::define(default)?;
            Class::<TlsSocket>::define(default)?;
            Class::<Server>::define(default)?;

            if Class::<EventEmitter>::prototype(ctx)?.is_some() {
                TlsSocket::add_event_emitter_prototype(ctx)?;
                Server::add_event_emitter_prototype(ctx)?;
            }

            if Class::<NetSocket>::prototype(ctx)?.is_some() {
                link_prototypes(ctx, default)?;
            }

            // `tls.TLSSocket` is a JS class extending `net.Socket` so that
            // `instanceof tls.TLSSocket`, `instanceof net.Socket` and
            // `instanceof stream.Duplex` all hold. The native `TlsSocket` is
            // only its `_handle`; the Duplex owns flowing and backpressure.
            let tls_handle_ctor = Function::new(ctx.clone(), TlsSocket::handle_factory)?;
            let glue: Object = ctx.eval(TLS_SOCKET_GLUE_JS)?;
            let build = glue.get::<_, Function>("build")?;
            let net_socket_class: Value = require_net_socket_class(ctx)?;
            let tls_class: Function = build.call((tls_handle_ctor, net_socket_class))?;
            default.set("TLSSocket", tls_class.clone())?;
            ctx.globals()
                .set("__lexeTlsSocketClass", tls_class.clone())?;

            default.set("connect", Func::from(connect))?;
            default.set("createServer", Func::from(create_server))?;
            default.set(
                "createSecureContext",
                Func::from(|ctx: Ctx<'js>, options: Value<'js>| {
                    create_js_secure_context(&ctx, options)
                }),
            )?;
            default.set(
                "checkServerIdentity",
                Func::from(check_server_identity_export),
            )?;
            default.set(
                "getCiphers",
                Func::from(|_ctx: Ctx<'js>| supported_ciphers()),
            )?;
            default.set(
                "getCACertificates",
                Func::from(
                    |ctx: Ctx<'js>, kind: rquickjs::function::Opt<String>| -> Result<Vec<String>> {
                        let k = kind.0.as_deref().unwrap_or("default");
                        crate::root_ca::get_ca_certificates(k)
                            .map_err(|e| crate::error::throw_invalid_arg_value(&ctx, "type", &e))
                    },
                ),
            )?;
            let root_list = crate::root_ca::get_ca_certificates("default").unwrap_or_default();
            let root_arr = rquickjs::Array::new(ctx.clone())?;
            for (idx, pem) in root_list.into_iter().enumerate() {
                root_arr.set(idx, pem)?;
            }
            if let Ok(object_ctor) = ctx.globals().get::<_, Object>("Object") {
                if let Ok(freeze) = object_ctor.get::<_, Function>("freeze") {
                    let _ = freeze.call::<_, Value>((root_arr.clone(),));
                }
            }
            default.set("rootCertificates", root_arr)?;
            default.set(
                "convertALPNProtocols",
                Func::from(|ctx: Ctx<'js>, protocols: Value<'js>, out: Object<'js>| {
                    convert_alpn_protocols_export(&ctx, protocols, &out)
                }),
            )?;

            default.set("CLIENT_RENEG_LIMIT", CLIENT_RENEG_LIMIT)?;
            default.set("CLIENT_RENEG_WINDOW", CLIENT_RENEG_WINDOW)?;
            default.set("DEFAULT_CIPHERS", default_ciphers())?;
            default.set("DEFAULT_ECDH_CURVE", DEFAULT_ECDH_CURVE)?;
            default.set("DEFAULT_MIN_VERSION", "TLSv1.2")?;
            default.set("DEFAULT_MAX_VERSION", "TLSv1.3")?;

            Ok(())
        })?;
        Ok(())
    }
}

impl From<TlsModule> for ModuleInfo<TlsModule> {
    fn from(val: TlsModule) -> Self {
        ModuleInfo {
            name: "tls",
            module: val,
        }
    }
}
