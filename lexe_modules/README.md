# Lexe Modules

Lexe Modules is a meta-module of [rquickjs](https://github.com/DelSkayn/rquickjs) modules that can be used independently of Lexe. They aim to bring to [quickjs](https://bellard.org/quickjs/) APIs from [Node.js](https://nodejs.org/) and [WinterTC](https://wintertc.org/). You can use this meta-module, but each module is also a unique crate.

Lexe is a lightweight JavaScript runtime built on QuickJS and maintained for the Lexe project.

## Usage

The package is not available in the crate registry yet, but you can clone the repo and import it as a local path.

Use this script to set everything up:

```bash
cd your_project_dir
git clone https://github.com/Ray-D-Song/lexe.git

cd lexe
npm i
make js
```

Each module has a feature flag, they are all enabled by default but if you prefer to can decide which one you need.
Check the [Compability matrix](#compatibility-matrix) for the full list.

```toml
[dependencies]
lexe_modules = { path = "lexe/lexe_modules", default-features = true } # load from local path
rquickjs = { version = "0.11", features = ["full-async"] }
tokio = { version = "1", features = ["full"] }

```

Once you have enable a module, you can import it in your runtime.

> [!NOTE]
> Some modules currently require that you call an `init` function **before** they evaluated.

```rust
use lexe_modules::buffer;
use rquickjs::{async_with, context::EvalOptions, AsyncContext, AsyncRuntime, Error, Module};


#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let runtime = AsyncRuntime::new()?;
    let context = AsyncContext::full(&runtime).await?;

    async_with!(context => |ctx| {
        buffer::init(&ctx)?;
        let (_module, module_eval) = Module::evaluate_def::<buffer::BufferModule,_>(ctx.clone(), "buffer")?;
        module_eval.into_future::<()>().await?;

        let mut options = EvalOptions::default();
        options.global = false;
        if let Err(Error::Exception) = ctx.eval_with_options::<(), _>(
            r#"
            import { Buffer } from "node:buffer";
            Buffer.alloc(10);
            "#,
            options
        ){
            println!("{:#?}", ctx.catch());
        };

        Ok::<_, Error>(())
    })
    .await?;

    Ok(())
}
```

Using ModuleBuilder makes it even simpler.

```rust
use lexe_modules::module_builder::ModuleBuilder;
use rquickjs::{async_with, context::EvalOptions, AsyncContext, AsyncRuntime, Error, Module};


#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let runtime = AsyncRuntime::new()?;

    let module_builder = ModuleBuilder::default();
    let (module_resolver, module_loader, global_attachment) = module_builder.build();
    runtime.set_loader((module_resolver,), (module_loader,)).await;

    let context = AsyncContext::full(&runtime).await?;

    async_with!(context => |ctx| {
        global_attachment.attach(&ctx)?;

        let mut options = EvalOptions::default();
        options.global = false;
        if let Err(Error::Exception) = ctx.eval_with_options::<(), _>(
            r#"
            import { Buffer } from "node:buffer";
            Buffer.alloc(10);
            "#,
            options
        ){
            println!("{:#?}", ctx.catch());
        };

        Ok::<_, Error>(())
    })
    .await?;

    Ok(())
}

```

## Compatibility matrix

> [!NOTE]
> Only a fraction of the Node.js APIs are supported. Below is a high level overview of partially supported APIs and modules.

| Module / API | Node.js | lexe Modules | Feature | Crate / implementation |
| ------------ | ------- | ---------------------- | ------- | ---------------------- |
| abort | ✔︎ | ✔︎ | `abort` | `lexe_abort` |
| assert / assert/strict | ✔︎ | ⚠️ | `assert` | `lexe_assert` |
| async_hooks | ✔︎ | ⚠️ | `async-hooks` | `lexe_async_hooks` |
| buffer | ✔︎ | ⚠️ | `buffer` | `lexe_buffer` |
| child_process | ✔︎ | ⚠️ | `child-process` | `lexe_child_process` |
| console | ✔︎ | ⚠️ | `console` | `lexe_console` |
| constants | ✔︎ | ⚠️ | `constants` | `lexe_constants` |
| crypto | ✔︎ | ⚠️ | `crypto` | `lexe_crypto` |
| dgram | ✔︎ | ⚠️ | `dgram` | `lexe_dgram` |
| diagnostics_channel | ✔︎ | ⚠️ | `diagnostics-channel` | `lexe_diagnostics_channel` |
| dns / dns/promises | ✔︎ | ⚠️ | `dns` | `lexe_dns` |
| events | ✔︎ | ⚠️ | `events` | `lexe_events` |
| exceptions | ✔︎ | ⚠️ | `exceptions` | `lexe_exceptions` |
| fetch | ✔︎ | ⚠️ | `fetch` | `lexe_fetch` |
| fs / fs/promises | ✔︎ | ⚠️ | `fs` | `lexe_fs` |
| http | ✔︎ | ⚠️ | `http` | `lexe_http` |
| http2 | ✔︎ | ✔︎ | N/A | native `lexe_http2`; embedded JS fallback for browser |
| https | ✔︎ | ⚠️ | `https` | `lexe_http` |
| inspector | ✔︎ | ⚠️ | `inspector` | `lexe_inspector` |
| intl | ✔︎ | ⚠️ | `intl` | `lexe_intl` |
| module | ✔︎ | ⚠️ | N/A | built-in CommonJS loader facade |
| navigator | ✔︎ | ⚠️ | `navigator` | `lexe_navigator` |
| net | ✔︎ | ⚠️ | `net` | `lexe_net` |
| os | ✔︎ | ⚠️ | `os` | `lexe_os` |
| path | ✔︎ | ⚠️ | `path` | `lexe_path` |
| perf_hooks | ✔︎ | ⚠️ | `perf-hooks` | `lexe_perf_hooks` |
| process | ✔︎ | ⚠️ | `process` | `lexe_process` |
| querystring | ✔︎ | ⚠️ | `querystring` | `lexe_querystring` |
| readline / readline/promises | ✔︎ | ✔︎ | N/A | embedded JS |
| sqlite | ✔︎ | ⚠️ | `sqlite` | `lexe_sqlite` (Node 24.3 / SQLite 3.50.1) |
| stream / stream/promises | ✔︎ | ✔︎ | N/A | `lexe_stream` / embedded JS |
| stream/web | ✔︎ | ⚠️ | `stream-web` | `lexe_stream_web` |
| string_decoder | ✔︎ | ✔︎ | `string-decoder` | `lexe_string_decoder` |
| test | ✔︎ | ✔︎ | N/A | embedded JS (`test.js`) |
| temporal | ✔︎ | ⚠️ | `temporal` | `lexe_temporal` |
| timers / timers/promises | ✔︎ | ⚠️ | `timers` | `lexe_timers` |
| tls | ✔︎ | ⚠️ | `tls` | `lexe_tls` |
| tty | ✔︎ | ⚠️ | `tty` | `lexe_tty` |
| url | ✔︎ | ⚠️ | `url` | `lexe_url` |
| util / util/types | ✔︎ | ⚠️ | `util` | `lexe_util` |
| v8 | ✔︎ | ⚠️ | `v8` | `lexe_v8` |
| vm | ✔︎ | ⚠️ | `vm` | `lexe_vm` |
| wasi | ✔︎ | ⚠️ | `webassembly` | `lexe_webassembly` (WASI Preview 1 via `wasmz`) |
| WebAssembly | ✔︎ | ⚠️ | `webassembly` | `lexe_webassembly` (backed by `wasmz`) |
| worker_threads | ✔︎ | ⚠️ | N/A | OS-thread isolates; structured-clone postMessage; SAB hooks |
| zlib | ✔︎ | ⚠️ | `zlib` | `lexe_zlib` |
| Other modules | ✔︎ | ✘ | N/A | N/A |

_⚠️ = partially supported_

## License

This module is licensed under the Apache-2.0 License.
