// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use lexe_async_hooks::init as init_async_hooks;
use lexe_test::{call_test, test_async_with, ModuleEvaluator};

use crate::Http2Module;

async fn eval_http2_module(ctx: rquickjs::Ctx<'_>) {
    init_async_hooks(&ctx).unwrap();
    ModuleEvaluator::eval_rust::<Http2Module>(ctx.clone(), "http2")
        .await
        .unwrap();
}

#[tokio::test]
async fn http2_create_server_h2c() {
    test_async_with(|ctx| {
        Box::pin(async move {
            eval_http2_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(
                ctx.clone(),
                "test",
                r#"
                import { createServer } from 'http2';
                export async function test() {
                  const server = createServer((req, res) => {
                    res.statusCode = 200;
                    res.end('h2-ok');
                  });
                  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
                  return server.address().port;
                }
            "#,
            )
            .await
            .unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;

            use tokio::net::TcpStream;
            let tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            let (mut client, connection) = h2::client::handshake(tcp).await.unwrap();
            tokio::spawn(async move {
                let _ = connection.await;
            });
            let request = http::Request::builder()
                .method("GET")
                .uri(format!("http://127.0.0.1:{port}/"))
                .body(())
                .unwrap();
            let (response, mut send) = client.send_request(request, false).unwrap();
            send.send_data(bytes::Bytes::from_static(b""), true)
                .unwrap();
            let response = response.await.unwrap();
            assert_eq!(response.status(), http::StatusCode::OK);
            let mut body = response.into_body();
            let mut data = Vec::new();
            while let Some(chunk) = body.data().await {
                data.extend_from_slice(&chunk.unwrap());
            }
            assert_eq!(String::from_utf8_lossy(&data), "h2-ok");
        })
    })
    .await;
}

#[tokio::test]
async fn http2_connect_h2c() {
    test_async_with(|ctx| {
        Box::pin(async move {
            eval_http2_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(
                ctx.clone(),
                "test",
                r#"
                import { createServer, connect, constants } from 'http2';
                export async function test() {
                  const server = createServer((req, res) => {
                    res.end('via-connect');
                  });
                  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
                  const port = server.address().port;
                  const authority = 'http://127.0.0.1:' + port;
                  const body = await new Promise((resolve, reject) => {
                    const client = connect(authority);
                    client.on('error', reject);
                    client.on('connect', () => {
                      const req = client.request({
                        [constants.HTTP2_HEADER_METHOD]: constants.HTTP2_METHOD_GET,
                        [constants.HTTP2_HEADER_PATH]: '/',
                      });
                      const chunks = [];
                      req.on('response', () => {});
                      req.on('data', (c) => chunks.push(c));
                      req.on('end', () => {
                        client.close();
                        server.close();
                        resolve(Buffer.concat(chunks).toString());
                      });
                    });
                  });
                  return body === 'via-connect';
                }
            "#,
            )
            .await
            .unwrap();
            let ok: bool = call_test(&ctx, &module, ()).await;
            assert!(ok);
        })
    })
    .await;
}
