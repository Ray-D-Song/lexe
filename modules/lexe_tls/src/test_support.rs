// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Shared helpers for TLS integration tests.

use lexe_buffer as buffer;
use lexe_events::EventsModule;
use lexe_net::NetModule;
use lexe_test::{call_test, test_async_with, ModuleEvaluator};
use rquickjs::{module::Evaluated, Ctx, Module};

use crate::TlsModule;

pub async fn init_tls_modules(ctx: &Ctx<'_>) {
    buffer::init(ctx).expect("buffer");
    ModuleEvaluator::eval_rust::<EventsModule>(ctx.clone(), "events")
        .await
        .expect("events");
    ModuleEvaluator::eval_rust::<NetModule>(ctx.clone(), "net")
        .await
        .expect("net");
    ModuleEvaluator::eval_rust::<TlsModule>(ctx.clone(), "tls")
        .await
        .expect("tls");
}

pub async fn eval_tls_test<'js, T, A>(ctx: &Ctx<'js>, source: &str, args: A) -> T
where
    T: rquickjs::FromJs<'js>,
    A: rquickjs::function::IntoArgs<'js>,
{
    let module: Module<'js, Evaluated> = ModuleEvaluator::eval_js(ctx.clone(), "test", source)
        .await
        .expect("eval test module");
    call_test(ctx, &module, args).await
}

pub async fn run_tls_test<F>(f: F)
where
    F: for<'js> FnOnce(Ctx<'js>) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + 'js>>
        + Send,
{
    test_async_with(f).await;
}
