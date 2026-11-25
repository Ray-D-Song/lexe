use lexe_test::{call_test, test_async_with, ModuleEvaluator};
use rquickjs::{Ctx, Function};

use super::HttpModule;
use crate::HttpsModule;

fn init_test_runtime(ctx: &Ctx<'_>) {
    lexe_async_hooks::init(ctx).expect("async_hooks init");
}

async fn eval_http_module(ctx: Ctx<'_>) {
    init_test_runtime(&ctx);
    ModuleEvaluator::eval_rust::<HttpModule>(ctx.clone(), "http")
        .await
        .unwrap();
}

async fn eval_https_module(ctx: Ctx<'_>) {
    eval_http_module(ctx.clone()).await;
    ModuleEvaluator::eval_rust::<HttpsModule>(ctx, "https")
        .await
        .unwrap();
}

#[tokio::test]
async fn exposes_http_and_node_http() {
    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import http, { METHODS, STATUS_CODES } from 'http';
                export function test() {
                  http.validateHeaderName('x-test');
                  return typeof http.createServer === 'function'
                    && METHODS.includes('GET')
                    && STATUS_CODES[200] === 'OK'
                    && STATUS_CODES[418] === "I'm a teapot"
                    && !('Socket' in http)
                    && (() => { const s = http.createServer({ maxHeadersCount: 12, headersTimeout: 34, requestTimeout: 56, keepAliveTimeout: 78, maxRequestsPerSocket: 4, insecureHTTPParser: true }); return s.maxHeadersCount === 12 && s.headersTimeout === 34 && s.requestTimeout === 56 && s.keepAliveTimeout === 78 && s.maxRequestsPerSocket === 4 && s.insecureHTTPParser === true; })()
                    && METHODS.length === 35
                    && METHODS.includes('MKCALENDAR')
                    && http.maxHeaderSize === 16384
                    && typeof http.OutgoingMessage === 'function'
                    && typeof http.setMaxIdleHTTPParsers === 'function';
                }
            "#).await.unwrap();
            let ok: bool = call_test(&ctx, &module, ()).await;
            assert!(ok);
        })).await;
}

#[tokio::test]
async fn serves_a_request_and_streams_response_body() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer((req, res) => {
                    res.statusCode = 201;
                    res.setHeader('x-lexe', 'yes');
                    res.setHeader('trailer', 'x-trailer');
                    res.write('hello ');
                    res.addTrailers({ 'x-trailer': 'yes' });
                    res.end('world');
                  });
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"GET /hello HTTP/1.1\r\nHost: localhost\r\nTE: trailers\r\nConnection: close\r\n\r\n").await.unwrap();
            let mut response = Vec::new();
            socket.read_to_end(&mut response).await.unwrap();
            let response = String::from_utf8(response).unwrap();
            assert!(response.starts_with("HTTP/1.1 201"));
            assert!(response.contains("x-lexe: yes"));
            assert!(response.contains("hello "));
            assert!(response.contains("world"));
            assert!(response.contains("x-trailer: yes"), "{response}");
        })).await;
}

#[tokio::test]
async fn write_head_commits_headers_before_end() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer((_req, res) => res.writeHead(202, { 'x-head': 'yes' }).end(res.headersSent ? 'sent' : 'missing'));
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").await.unwrap();
            let mut response = Vec::new(); socket.read_to_end(&mut response).await.unwrap();
            let response = String::from_utf8(response).unwrap();
            assert!(response.starts_with("HTTP/1.1 202"));
            assert!(response.contains("x-head: yes"));
            assert!(response.contains("sent"));
        })).await;
}

#[tokio::test]
async fn writes_status_message() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer((_req, res) => { res.statusCode = 299; res.statusMessage = 'Lexe'; res.end(); });
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").await.unwrap();
            let mut response = Vec::new(); socket.read_to_end(&mut response).await.unwrap();
            assert!(String::from_utf8(response).unwrap().starts_with("HTTP/1.1 299 Lexe"));
        })).await;
}

#[tokio::test]
async fn write_head_accepts_status_message_and_headers() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer((_req, res) => res.writeHead(207, 'Multi', { 'x-head': 'yes' }).end());
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").await.unwrap();
            let mut response = Vec::new(); socket.read_to_end(&mut response).await.unwrap();
            let response = String::from_utf8(response).unwrap();
            assert!(response.starts_with("HTTP/1.1 207 Multi"));
            assert!(response.contains("x-head: yes"));
        })).await;
}

#[tokio::test]
async fn exposes_the_connection_socket_on_request() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  let connected;
                  const server = createServer((req, res) => {
                    const usable = req.socket === connected
                      && !req.socket.pending
                      && req.socket.readyState === 'open'
                      && req.socket.remoteAddress === '127.0.0.1'
                      && typeof req.socket.remotePort === 'number';
                    res.end(usable ? 'same' : 'different');
                  });
                  server.once('connection', socket => { connected = socket; });
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").await.unwrap();
            let mut response = Vec::new(); socket.read_to_end(&mut response).await.unwrap();
            assert!(String::from_utf8(response).unwrap().contains("same"));
        })).await;
}

#[tokio::test]
async fn request_socket_destroy_closes_the_tcp_connection() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer(req => req.socket.destroy());
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n").await.unwrap();
            let mut response = [0; 1];
            let count = tokio::time::timeout(std::time::Duration::from_millis(100), socket.read(&mut response)).await.unwrap().unwrap();
            assert_eq!(count, 0, "destroy() must close the accepted TCP stream");
        })).await;
}

#[tokio::test]
async fn combines_duplicate_headers_and_preserves_raw_headers() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer((req, res) => {
                    const ok = req.headers['x-test'] === 'one, two'
                      && req.headers.cookie === 'a=1; b=2'
                      && req.headers['set-cookie'].join('|') === 'a=1|b=2'
                      && req.rawHeaders.filter(v => v === 'x-test').length === 2;
                    res.end(ok ? 'ok' : 'bad');
                  });
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nX-Test: one\r\nX-Test: two\r\nCookie: a=1\r\nCookie: b=2\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\nConnection: close\r\n\r\n").await.unwrap();
            let mut response = Vec::new(); socket.read_to_end(&mut response).await.unwrap();
            assert!(String::from_utf8(response).unwrap().contains("ok"));
        })).await;
}

#[tokio::test]
async fn request_data_events_deliver_buffers() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer((req, res) => {
                    let body = '';
                    req.on('data', chunk => body += chunk.toString());
                    req.on('end', () => res.end(body));
                  });
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbody").await.unwrap();
            let mut response = Vec::new(); socket.read_to_end(&mut response).await.unwrap();
            assert!(String::from_utf8(response).unwrap().contains("body"));
        })).await;
}

#[tokio::test]
async fn rejects_invalid_response_headers_at_api_boundary() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer((_req, res) => {
                    let rejected = false;
                    try { res.setHeader('x-test', 'good\r\nbad'); } catch { rejected = true; }
                    res.end(rejected ? 'rejected' : 'accepted');
                  });
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").await.unwrap();
            let mut response = Vec::new(); socket.read_to_end(&mut response).await.unwrap();
            assert!(String::from_utf8(response).unwrap().contains("rejected"));
        })).await;
}

#[cfg(unix)]
#[tokio::test]
async fn listens_on_a_unix_socket() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let path = std::env::temp_dir().join(format!("lexe-http-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let path_string = path.to_string_lossy().to_string();
    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test(path) {
                  const server = createServer((req, res) => res.end('unix'));
                  server.listen({ path });
                  return new Promise(resolve => server.once('listening', () => resolve(server.address())));
                }
            "#).await.unwrap();
            let address: String = call_test(&ctx, &module, (path_string.clone(),)).await;
            assert_eq!(address, path_string);
            let mut socket = tokio::net::UnixStream::connect(&path_string).await.unwrap();
            socket.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").await.unwrap();
            let mut response = Vec::new(); socket.read_to_end(&mut response).await.unwrap();
            assert!(String::from_utf8(response).unwrap().contains("unix"));
        })).await;
    std::fs::remove_file(path).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn listens_on_a_unix_socket_path_argument() {
    let path = std::env::temp_dir().join(format!("lexe-http-path-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let path_string = path.to_string_lossy().to_string();
    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test(path) {
                  const server = createServer();
                  server.listen(path);
                  return new Promise(resolve => server.once('listening', () => resolve(server.address())));
                }
            "#).await.unwrap();
            let address: String = call_test(&ctx, &module, (path_string.clone(),)).await;
            assert_eq!(address, path_string);
        })).await;
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn enforces_max_headers_count() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer((req, res) => res.end('unexpected'));
                  server.maxHeadersCount = 1;
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nX-Test: one\r\nConnection: close\r\n\r\n").await.unwrap();
            let mut response = Vec::new(); socket.read_to_end(&mut response).await.unwrap();
            assert!(String::from_utf8(response).unwrap().starts_with("HTTP/1.1 431"));
        })).await;
}

#[tokio::test]
async fn enforces_max_header_size() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer({ maxHeaderSize: 8192 }, (_req, res) => res.end('unexpected'));
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            let header = "x".repeat(9_000);
            socket.write_all(format!("GET / HTTP/1.1\r\nHost: localhost\r\nX-Large: {header}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
            let mut response = Vec::new(); let _ = socket.read_to_end(&mut response).await;
            let response = String::from_utf8(response).unwrap();
            assert!(!response.contains("unexpected"));
        })).await;
}

#[tokio::test]
async fn closes_slow_headers_after_headers_timeout() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer((req, res) => res.end('unexpected'));
                  server.headersTimeout = 10;
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"GET / HTTP/1.1\r\nHost: localhost").await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            let mut response = Vec::new();
            socket.read_to_end(&mut response).await.unwrap();
            assert!(response.is_empty());
        })).await;
}

#[tokio::test]
async fn enforces_request_body_timeout() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer({ requestTimeout: 10 }, (req, res) => req.on('data', () => {}));
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 4\r\nConnection: close\r\n\r\n").await.unwrap();
            let mut response = Vec::new(); socket.read_to_end(&mut response).await.unwrap();
            assert!(String::from_utf8(response).unwrap().starts_with("HTTP/1.1 408"));
        })).await;
}

#[tokio::test]
async fn does_not_block_completed_response_on_unread_body() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer({ requestTimeout: 1000 }, (_req, res) => res.end('early'));
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 100\r\n\r\n").await.unwrap();
            let mut response = [0; 1024];
            let count = tokio::time::timeout(std::time::Duration::from_millis(100), socket.read(&mut response)).await.unwrap().unwrap();
            assert!(String::from_utf8_lossy(&response[..count]).contains("early"));
        })).await;
}

#[tokio::test]
async fn rejects_upgrade_and_connect_without_listeners() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer(() => { throw new Error('request must not run'); });
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            for request in [
                b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: test\r\n\r\n".as_slice(),
                b"CONNECT example.test:443 HTTP/1.1\r\nHost: example.test:443\r\n\r\n".as_slice(),
            ] {
                let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
                socket.write_all(request).await.unwrap();
                let mut response = Vec::new(); let _ = socket.read_to_end(&mut response).await;
                let response = String::from_utf8(response).unwrap();
                assert!(!response.starts_with("HTTP/1.1 101") && !response.starts_with("HTTP/1.1 200"), "{response}");
            }
        })).await;
}

#[tokio::test]
async fn limits_buffered_request_body_size() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer({ maxBodySize: 4 }, () => {});
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\nConnection: close\r\n\r\n12345").await.unwrap();
            let mut response = Vec::new(); socket.read_to_end(&mut response).await.unwrap();
            assert!(String::from_utf8(response).unwrap().starts_with("HTTP/1.1 413"));
        })).await;
}

#[tokio::test]
async fn closes_idle_keep_alive_connections() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer({ keepAliveTimeout: 10 }, (_req, res) => res.end('ok'));
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n").await.unwrap();
            let mut response = [0; 1024];
            let count = socket.read(&mut response).await.unwrap();
            assert!(String::from_utf8_lossy(&response[..count]).starts_with("HTTP/1.1 200"));
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            assert_eq!(socket.read(&mut response).await.unwrap(), 0);
        })).await;
}

#[tokio::test]
async fn reports_and_closes_connections() {
    use tokio::io::AsyncReadExt;

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  globalThis.server = createServer(() => {});
                  globalThis.server.listen(0, '127.0.0.1');
                  return new Promise(resolve => globalThis.server.once('listening', () => resolve(globalThis.server.address().port)));
                }
                export function connectionCount() { let count = -1; globalThis.server.getConnections((_error, value) => { count = value; }); return count; }
                export function closeAll() { globalThis.server.closeAllConnections(); }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let connection_count: Function = module.get("connectionCount").unwrap();
            let count: i32 = connection_count.call(()).unwrap();
            assert_eq!(count, 1);
            let close_all: Function = module.get("closeAll").unwrap();
            close_all.call::<_, ()>(()).unwrap();
            let mut buffer = [0; 1];
            assert_eq!(socket.read(&mut buffer).await.unwrap(), 0);
        })).await;
}

#[tokio::test]
async fn closes_idle_connections_on_demand() {
    use tokio::io::AsyncReadExt;

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  globalThis.server = createServer(() => {});
                  globalThis.server.listen(0, '127.0.0.1');
                  return new Promise(resolve => globalThis.server.once('listening', () => resolve(globalThis.server.address().port)));
                }
                export function closeIdle() { globalThis.server.closeIdleConnections(); }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let close_idle: Function = module.get("closeIdle").unwrap();
            close_idle.call::<_, ()>(()).unwrap();
            let mut buffer = [0; 1];
            assert_eq!(socket.read(&mut buffer).await.unwrap(), 0);
        })).await;
}

#[tokio::test]
async fn serves_http1_pipelined_requests_in_order() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer((req, res) => res.end(req.url));
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"GET /first HTTP/1.1\r\nHost: localhost\r\n\r\nGET /second HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").await.unwrap();
            let mut response = Vec::new(); socket.read_to_end(&mut response).await.unwrap();
            let response = String::from_utf8(response).unwrap();
            let first = response.find("/first").unwrap();
            let second = response.find("/second").unwrap();
            assert!(first < second, "{response}");
        })).await;
}

#[tokio::test]
async fn suppresses_response_body_for_head_requests() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer((_req, res) => res.end('must-not-appear'));
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"HEAD / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").await.unwrap();
            let mut response = Vec::new(); socket.read_to_end(&mut response).await.unwrap();
            assert!(!String::from_utf8(response).unwrap().contains("must-not-appear"));
        })).await;
}

#[tokio::test]
async fn suppresses_response_body_for_204_and_304() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer((req, res) => { res.statusCode = req.url === '/204' ? 204 : 304; res.end('must-not-appear'); });
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            for path in ["/204", "/304"] {
                let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
                socket.write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
                let mut response = Vec::new(); socket.read_to_end(&mut response).await.unwrap();
                assert!(!String::from_utf8(response).unwrap().contains("must-not-appear"));
            }
        })).await;
}

#[tokio::test]
async fn handles_expect_continue_like_node() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer((req, res) => { req.on('data', () => {}); req.on('end', () => res.end('ok')); });
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"POST / HTTP/1.1\r\nHost: localhost\r\nExpect: 100-continue\r\nContent-Length: 2\r\nConnection: close\r\n\r\n").await.unwrap();
            let mut interim = [0; 25]; socket.read_exact(&mut interim).await.unwrap();
            assert_eq!(&interim, b"HTTP/1.1 100 Continue\r\n\r\n");
            socket.write_all(b"ok").await.unwrap();
            let mut response = Vec::new(); socket.read_to_end(&mut response).await.unwrap();
            assert!(String::from_utf8(response).unwrap().starts_with("HTTP/1.1 200"));
        })).await;
}

#[tokio::test]
async fn check_continue_can_reject_without_sending_100() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer(() => { throw new Error('request must not run'); });
                  server.on('checkContinue', (_req, res) => { res.statusCode = 417; res.end('no'); });
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"POST / HTTP/1.1\r\nHost: localhost\r\nExpect: 100-continue\r\nContent-Length: 2\r\nConnection: close\r\n\r\n").await.unwrap();
            let mut response = Vec::new(); socket.read_to_end(&mut response).await.unwrap();
            let response = String::from_utf8(response).unwrap();
            assert!(response.starts_with("HTTP/1.1 417"), "{response}");
            assert!(!response.contains("100 Continue"), "{response}");
        })).await;
}

#[tokio::test]
async fn emits_client_error_for_malformed_requests() {
    use tokio::io::AsyncWriteExt;

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer(() => {});
                  server.on('clientError', error => { globalThis.clientError = error.code; });
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
                export function errorCode() { return globalThis.clientError; }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"NOT HTTP\r\n\r\n").await.unwrap();
            drop(socket);
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let error_code: Function = module.get("errorCode").unwrap();
            let code: String = error_code.call(()).unwrap();
            assert_eq!(code, "HPE_INVALID_REQUEST");
        })).await;
}

#[tokio::test]
async fn close_waits_for_listener_shutdown() {
    test_async_with(|ctx| {
        Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(
                ctx.clone(),
                "test",
                r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer(() => {});
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => {
                    server.close(() => resolve(!server.listening));
                  }));
                }
            "#,
            )
            .await
            .unwrap();
            let closed: bool = call_test(&ctx, &module, ()).await;
            assert!(closed);
        })
    })
    .await;
}

#[tokio::test]
async fn close_waits_for_active_connections() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  globalThis.server = createServer((_req, res) => res.end('ok'));
                  globalThis.server.listen(0, '127.0.0.1');
                  return new Promise(resolve => globalThis.server.once('listening', () => resolve(globalThis.server.address().port)));
                }
                export function closeServer() { return new Promise(resolve => globalThis.server.close(() => resolve(true))); }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n").await.unwrap();
            let mut response = [0; 1024]; let _ = socket.read(&mut response).await.unwrap();
            let close_server: Function = module.get("closeServer").unwrap();
            let close = close_server.call::<_, rquickjs::promise::MaybePromise>(()).unwrap();
            drop(socket);
            let closed: bool = close.into_future().await.unwrap();
            assert!(closed);
        })).await;
}

#[tokio::test]
async fn upgrades_a_connection_to_a_net_socket() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer();
                  server.on('upgrade', (_req, socket) => { socket.write('upgraded'); socket.end(); });
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: test\r\n\r\n").await.unwrap();
            let mut response = Vec::new(); socket.read_to_end(&mut response).await.unwrap();
            let response = String::from_utf8(response).unwrap();
            assert!(response.starts_with("HTTP/1.1 101"));
            assert!(response.contains("upgraded"), "{response}");
        })).await;
}

#[tokio::test]
async fn upgraded_socket_exposes_incoming_bytes_to_read() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer();
                  server.on('upgrade', (_req, socket) => { globalThis.upgradedSocket = socket; });
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
                export function readUpgrade() { return globalThis.upgradedSocket.read().toString(); }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: test\r\n\r\n").await.unwrap();
            let mut headers = [0; 256];
            let count = tokio::time::timeout(std::time::Duration::from_millis(100), socket.read(&mut headers)).await.unwrap().unwrap();
            assert!(String::from_utf8_lossy(&headers[..count]).starts_with("HTTP/1.1 101"));
            socket.write_all(b"client-data").await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let read_upgrade: Function = module.get("readUpgrade").unwrap();
            let data: String = read_upgrade.call(()).unwrap();
            assert_eq!(data, "client-data");
        })).await;
}

#[tokio::test]
async fn upgraded_socket_emits_data_events() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer();
                  server.on('upgrade', (_req, socket) => {
                    globalThis.upgradeData = new Promise(resolve => socket.once('data', data => resolve(data.toString())));
                  });
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
                export function waitForUpgradeData() { return globalThis.upgradeData; }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: test\r\n\r\n").await.unwrap();
            let mut headers = [0; 256];
            let _ = tokio::time::timeout(std::time::Duration::from_millis(100), socket.read(&mut headers)).await.unwrap().unwrap();
            socket.write_all(b"event-data").await.unwrap();
            let wait_for_data: Function = module.get("waitForUpgradeData").unwrap();
            let promise = wait_for_data.call::<_, rquickjs::promise::MaybePromise>(()).unwrap();
            let data: String = promise.into_future().await.unwrap();
            assert_eq!(data, "event-data");
        })).await;
}

#[tokio::test]
async fn routes_connect_to_connect_event() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer(() => { throw new Error('request must not run'); });
                  server.on('connect', (_req, socket) => socket.end('tunnel'));
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"CONNECT example.test:443 HTTP/1.1\r\nHost: example.test:443\r\n\r\n").await.unwrap();
            let mut response = Vec::new(); socket.read_to_end(&mut response).await.unwrap();
            let response = String::from_utf8(response).unwrap();
            assert!(response.starts_with("HTTP/1.1 200"));
            assert!(response.contains("tunnel"), "{response}");
        })).await;
}

#[tokio::test]
async fn exposes_request_trailers_before_end() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(ctx.clone(), "test", r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer((req, res) => {
                    req.on('end', () => res.end(req.trailers['x-trailer']));
                  });
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#).await.unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            socket.write_all(b"POST / HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\ntest\r\n0\r\nX-Trailer: present\r\n\r\n").await.unwrap();
            let mut response = Vec::new(); socket.read_to_end(&mut response).await.unwrap();
            assert!(String::from_utf8(response).unwrap().contains("present"));
        })).await;
}

#[tokio::test]
async fn http_get_fetches_local_server() {
    test_async_with(|ctx| {
        Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(
                ctx.clone(),
                "test",
                r#"
                import { createServer, get } from 'http';
                export async function test() {
                  const server = createServer((_req, res) => {
                    res.statusCode = 200;
                    res.end('pong');
                  });
                  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
                  const port = server.address().port;
                  const body = await new Promise((resolve, reject) => {
                    get(`http://127.0.0.1:${port}/`, (res) => {
                      const chunks = [];
                      res.on('data', (c) => chunks.push(c));
                      res.on('end', () => resolve(Buffer.concat(chunks).toString()));
                    }).on('error', reject);
                  });
                  server.close();
                  return body === 'pong';
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

fn js_string_literal(s: &str) -> String {
    format!("{s:?}")
}

#[tokio::test]
async fn https_create_server_serves_over_tls() {
    use lexe_test_tls::fixtures;

    test_async_with(|ctx| {
        Box::pin(async move {
            eval_https_module(ctx.clone()).await;
            let cert = js_string_literal(fixtures::SERVER_CERT);
            let key = js_string_literal(fixtures::SERVER_KEY);
            let module = ModuleEvaluator::eval_js(
                ctx.clone(),
                "test",
                &format!(
                    r#"
                import {{ createServer }} from 'https';
                export async function test() {{
                  const server = createServer({{ cert: {cert}, key: {key} }}, (_req, res) => {{
                    res.statusCode = 200;
                    res.end('tls-ok');
                  }});
                  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
                  return server.address().port;
                }}
            "#
                ),
            )
            .await
            .unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;

            use rustls::pki_types::ServerName;
            use rustls::RootCertStore;
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            use tokio_rustls::TlsConnector;

            let mut roots = RootCertStore::empty();
            let mut reader = std::io::Cursor::new(fixtures::ROOT_CA.as_bytes());
            for cert in rustls_pemfile::certs(&mut reader) {
                roots.add(cert.unwrap()).unwrap();
            }
            let config = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
            let connector = TlsConnector::from(std::sync::Arc::new(config));

            let tcp = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            let server_name = ServerName::try_from("localhost").unwrap();
            let mut tls = connector.connect(server_name, tcp).await.unwrap();
            tls.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            let mut response = Vec::new();
            tls.read_to_end(&mut response).await.unwrap();
            let response = String::from_utf8_lossy(&response);
            assert!(response.contains("tls-ok"), "{response}");
        })
    })
    .await;
}

#[tokio::test]
async fn lenient_parser_accepts_bare_lf_multi_space_ctl_headers_and_obs_fold() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| {
        Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(
                ctx.clone(),
                "test",
                r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer({ insecureHTTPParser: true }, (req, res) => {
                    const out = JSON.stringify({
                      method: req.method,
                      url: req.url,
                      ctl: req.headers['x-ctl'],
                      fold: req.headers['x-fold'],
                      rawLen: req.rawHeaders.length,
                    });
                    res.end(out);
                  });
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#,
            )
            .await
            .unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            socket
                .write_all(
                    b"GET  /lenient  HTTP/1.1\nHost: localhost\nX-Ctl: a\x01b\nX-Fold: one\n two\nConnection: close\n\n",
                )
                .await
                .unwrap();
            let mut response = Vec::new();
            socket.read_to_end(&mut response).await.unwrap();
            let response = String::from_utf8(response).unwrap();
            assert!(response.starts_with("HTTP/1.1 200"), "{response}");
            assert!(response.contains("\"url\":\"/lenient\""), "{response}");
            assert!(response.contains("\"ctl\":\"a\\u0001b\""), "{response}");
            assert!(response.contains("\"fold\":\"one two\""), "{response}");
        })
    })
    .await;
}

#[tokio::test]
async fn lenient_parser_rejects_space_before_colon_with_invalid_header_token() {
    use tokio::io::AsyncWriteExt;

    test_async_with(|ctx| {
        Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(
                ctx.clone(),
                "test",
                r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer({ insecureHTTPParser: true }, () => {});
                  server.on('clientError', error => { globalThis.lenientErrCode = error.code; });
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
                export function errorCode() { return globalThis.lenientErrCode; }
            "#,
            )
            .await
            .unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            socket
                .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nX-Bad : val\r\n\r\n")
                .await
                .unwrap();
            drop(socket);
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            let error_code: Function = module.get("errorCode").unwrap();
            let code: String = error_code.call(()).unwrap();
            assert_eq!(code, "HPE_INVALID_HEADER_TOKEN");
        })
    })
    .await;
}

#[tokio::test]
async fn server_response_write_early_hints_and_processing() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| {
        Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(
                ctx.clone(),
                "test",
                r#"
                import { createServer } from 'http';
                export function test() {
                  const server = createServer((_req, res) => {
                    res.writeEarlyHints(
                      { link: ['</a.css>; rel=preload; as=style', '</b.js>; rel=preload; as=script'], 'x-hint': 'yes' },
                      () => {
                        res.writeProcessing(() => {
                          res.writeHead(200, { 'content-type': 'text/plain' });
                          res.end('final');
                        });
                      }
                    );
                  });
                  server.listen(0, '127.0.0.1');
                  return new Promise(resolve => server.once('listening', () => resolve(server.address().port)));
                }
            "#,
            )
            .await
            .unwrap();
            let port: u16 = call_test(&ctx, &module, ()).await;
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            socket
                .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            let mut response = Vec::new();
            socket.read_to_end(&mut response).await.unwrap();
            let response = String::from_utf8(response).unwrap();
            assert!(response.starts_with("HTTP/1.1 103 Early Hints\r\n"), "{response}");
            assert!(response.contains("Link: </a.css>; rel=preload; as=style, </b.js>; rel=preload; as=script"), "{response}");
            assert!(response.contains("x-hint: yes"), "{response}");
            assert!(response.contains("HTTP/1.1 102 Processing\r\n\r\n"), "{response}");
            assert!(response.contains("HTTP/1.1 200 OK"), "{response}");
            assert!(response.contains("final"), "{response}");
        })
    })
    .await;
}

#[tokio::test]
async fn server_check_expectation_and_max_requests_per_socket() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    test_async_with(|ctx| {
        Box::pin(async move {
            eval_http_module(ctx.clone()).await;
            let module = ModuleEvaluator::eval_js(
                ctx.clone(),
                "test",
                r#"
                import { createServer } from 'http';
                export function test() {
                  globalThis.droppedUrls = [];
                  const sExpectDefault = createServer((_req, res) => res.end('wrong'));
                  sExpectDefault.listen(0, '127.0.0.1');

                  const sMax = createServer({ maxRequestsPerSocket: 1 }, (req, res) => res.end('ok:' + req.url));
                  sMax.on('dropRequest', (req) => globalThis.droppedUrls.push(req.url));
                  sMax.listen(0, '127.0.0.1');

                  return new Promise(resolve => {
                    sExpectDefault.once('listening', () => {
                      sMax.once('listening', () => resolve([sExpectDefault.address().port, sMax.address().port]));
                    });
                  });
                }
                export function getDropped() { return globalThis.droppedUrls; }
            "#,
            )
            .await
            .unwrap();
            let ports: Vec<u16> = call_test(&ctx, &module, ()).await;

            // Default Expect -> 417
            let mut s1 = tokio::net::TcpStream::connect(("127.0.0.1", ports[0]))
                .await
                .unwrap();
            s1.write_all(b"POST / HTTP/1.1\r\nHost: localhost\r\nExpect: custom\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            let mut r1 = Vec::new();
            s1.read_to_end(&mut r1).await.unwrap();
            let r1 = String::from_utf8(r1).unwrap();
            assert!(r1.starts_with("HTTP/1.1 417"), "{r1}");

            // maxRequestsPerSocket = 1 -> first 200 with Connection: close, second -> 503 + dropRequest
            let mut s2 = tokio::net::TcpStream::connect(("127.0.0.1", ports[1]))
                .await
                .unwrap();
            s2.write_all(b"GET /one HTTP/1.1\r\nHost: localhost\r\n\r\nGET /two HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .await
                .unwrap();
            let mut r2 = Vec::new();
            s2.read_to_end(&mut r2).await.unwrap();
            let r2 = String::from_utf8(r2).unwrap();
            assert!(r2.contains("ok:/one"), "{r2}");
            assert!(r2.to_ascii_lowercase().contains("connection: close"), "{r2}");
            assert!(r2.contains("HTTP/1.1 503"), "{r2}");
            let get_dropped: Function = module.get("getDropped").unwrap();
            let dropped: Vec<String> = get_dropped.call(()).unwrap();
            assert_eq!(dropped, vec!["/two".to_string()]);
        })
    })
    .await;
}
