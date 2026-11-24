// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#[cfg(test)]
mod starttls {
    use lexe_test_tls::{fixtures, MockServer};

    use crate::test_support::{eval_tls_test, init_tls_modules, run_tls_test};

    const STARTTLS_HELPERS: &str = r#"
        import tls from 'tls';
        import { connect as netConnect } from 'net';
        import { connect as tlsConnect } from 'tls';

        async function listenTlsServer(cert, key, ca) {
            const server = tls.createServer({ cert, key, ca, rejectUnauthorized: true });
            await new Promise((resolve, reject) => {
                server.listen(0, '127.0.0.1', resolve);
                server.on('error', reject);
            });
            return server.address().port;
        }

        async function upgrade(port, ca, removeListeners) {
            return new Promise((resolve, reject) => {
                const stream = netConnect({ port, host: '127.0.0.1' }, () => {
                        const startUpgrade = () => {
                            if (removeListeners) {
                                stream.on('data', () => {});
                                stream.removeAllListeners('data');
                            }
                            let closed = false;
                            stream.on('close', () => { closed = true; });
                            const tlsSocket = tlsConnect({
                                socket: stream,
                                servername: 'localhost',
                                ca,
                                rejectUnauthorized: true,
                            });
                            tlsSocket.on('secureConnect', () => {
                                resolve({ authorized: tlsSocket.authorized === true, closed });
                            });
                            tlsSocket.on('error', reject);
                        };
                        startUpgrade();
                });
                stream.on('error', reject);
            });
        }
    "#;

    #[tokio::test]
    async fn direct_tls_connect_to_server() {
        run_tls_test(|ctx| {
            Box::pin(async move {
                init_tls_modules(&ctx).await;
                let ok: bool = eval_tls_test(
                    &ctx,
                    &format!(
                        r#"{helpers}
                        export async function test(cert, key, ca) {{
                            const port = await listenTlsServer(cert, key, ca);
                            return new Promise((resolve, reject) => {{
                                const socket = tlsConnect({{
                                    port,
                                    host: '127.0.0.1',
                                    servername: 'localhost',
                                    ca,
                                    rejectUnauthorized: true,
                                }});
                                socket.on('secureConnect', () => resolve(socket.authorized === true));
                                socket.on('error', reject);
                            }});
                        }}"#,
                        helpers = STARTTLS_HELPERS
                    ),
                    (
                        fixtures::SERVER_CERT,
                        fixtures::SERVER_KEY,
                        fixtures::ROOT_CA,
                    ),
                )
                .await;
                assert!(ok);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn starttls_over_net_socket() {
        run_tls_test(|ctx| {
            Box::pin(async move {
                init_tls_modules(&ctx).await;
                let ok: bool = eval_tls_test(
                    &ctx,
                    &format!(
                        r#"{helpers}
                        export async function test(cert, key, ca) {{
                            const port = await listenTlsServer(cert, key, ca);
                            const result = await upgrade(port, ca, false);
                            return result.authorized;
                        }}"#,
                        helpers = STARTTLS_HELPERS
                    ),
                    (
                        fixtures::SERVER_CERT,
                        fixtures::SERVER_KEY,
                        fixtures::ROOT_CA,
                    ),
                )
                .await;
                assert!(ok);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn starttls_after_remove_all_listeners() {
        run_tls_test(|ctx| {
            Box::pin(async move {
                init_tls_modules(&ctx).await;
                let ok: bool = eval_tls_test(
                    &ctx,
                    &format!(
                        r#"{helpers}
                        export async function test(cert, key, ca) {{
                            const port = await listenTlsServer(cert, key, ca);
                            const result = await upgrade(port, ca, true);
                            return result.authorized;
                        }}"#,
                        helpers = STARTTLS_HELPERS
                    ),
                    (
                        fixtures::SERVER_CERT,
                        fixtures::SERVER_KEY,
                        fixtures::ROOT_CA,
                    ),
                )
                .await;
                assert!(ok);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn starttls_old_socket_does_not_emit_close() {
        run_tls_test(|ctx| {
            Box::pin(async move {
                init_tls_modules(&ctx).await;
                let ok: bool = eval_tls_test(
                    &ctx,
                    &format!(
                        r#"{helpers}
                        export async function test(cert, key, ca) {{
                            const port = await listenTlsServer(cert, key, ca);
                            const result = await upgrade(port, ca, true);
                            return !result.closed;
                        }}"#,
                        helpers = STARTTLS_HELPERS
                    ),
                    (
                        fixtures::SERVER_CERT,
                        fixtures::SERVER_KEY,
                        fixtures::ROOT_CA,
                    ),
                )
                .await;
                assert!(ok);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn starttls_race_repeated() {
        run_tls_test(|ctx| {
            Box::pin(async move {
                init_tls_modules(&ctx).await;
                let ok: bool = eval_tls_test(
                    &ctx,
                    &format!(
                        r#"{helpers}
                        export async function test(cert, key, ca, iterations) {{
                            const port = await listenTlsServer(cert, key, ca);
                            for (let i = 0; i < iterations; i++) {{
                                const result = await upgrade(port, ca, true);
                                if (!result.authorized) return false;
                            }}
                            return true;
                        }}"#,
                        helpers = STARTTLS_HELPERS
                    ),
                    (
                        fixtures::SERVER_CERT,
                        fixtures::SERVER_KEY,
                        fixtures::ROOT_CA,
                        50i32,
                    ),
                )
                .await;
                assert!(ok);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn mock_server_starttls_style() {
        run_tls_test(|ctx| {
            Box::pin(async move {
                init_tls_modules(&ctx).await;
                let server = MockServer::start().await.unwrap();
                let port = server.address().port();
                let ca = server.ca();

                let ok: bool = eval_tls_test(
                    &ctx,
                    r#"
                        import { connect as netConnect } from 'net';
                        import { connect as tlsConnect } from 'tls';

                        export async function test(port, ca) {
                            return new Promise((resolve, reject) => {
                                const stream = netConnect({ port, host: '127.0.0.1' }, () => {
                                    stream.removeAllListeners('data');
                                    const tlsSocket = tlsConnect({
                                        socket: stream,
                                        servername: 'localhost',
                                        ca,
                                        rejectUnauthorized: true,
                                    });
                                    tlsSocket.on('secureConnect', () => resolve(tlsSocket.authorized === true));
                                    tlsSocket.on('error', reject);
                                });
                                stream.on('error', reject);
                            });
                        }
                    "#,
                    (port, ca),
                )
                .await;

                assert!(ok);
            })
        })
        .await;
    }
}
