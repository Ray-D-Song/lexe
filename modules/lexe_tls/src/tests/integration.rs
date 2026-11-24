// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use lexe_test_tls::MockServer;

use crate::test_support::{eval_tls_test, init_tls_modules, run_tls_test};

#[tokio::test]
async fn module_loads() {
    run_tls_test(|ctx| Box::pin(async move { init_tls_modules(&ctx).await })).await;
}

#[tokio::test]
async fn client_connects_to_mock_server() {
    run_tls_test(|ctx| {
        Box::pin(async move {
            init_tls_modules(&ctx).await;
            let server = MockServer::start().await.unwrap();
            let port = server.address().port();
            let ca = server.ca();

            let ok: bool = eval_tls_test(
                &ctx,
                r#"
                    import { connect } from 'tls';

                    export async function test(port, ca) {
                        return new Promise((resolve, reject) => {
                            const socket = connect({
                                port,
                                host: '127.0.0.1',
                                servername: 'localhost',
                                ca,
                                rejectUnauthorized: true,
                            });
                            socket.on('secureConnect', () => {
                                resolve(socket.authorized === true && socket.encrypted === true);
                            });
                            socket.on('error', reject);
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

#[tokio::test]
async fn create_secure_context_mysql2_shape() {
    run_tls_test(|ctx| {
        Box::pin(async move {
            init_tls_modules(&ctx).await;

            let ok: bool = eval_tls_test(
                &ctx,
                r#"
                    import tls from 'tls';

                    export function test() {
                        return tls.createSecureContext({
                            ciphers: undefined,
                            pfx: undefined,
                            session: undefined,
                            checkServerIdentity: (hostname, cert) => undefined,
                        }) != null;
                    }
                "#,
                (),
            )
            .await;

            assert!(ok);
        })
    })
    .await;
}

#[tokio::test]
async fn rejects_unknown_ca_when_strict() {
    run_tls_test(|ctx| {
        Box::pin(async move {
            init_tls_modules(&ctx).await;
            let server = MockServer::start().await.unwrap();
            let port = server.address().port();

            let result: String = eval_tls_test(
                &ctx,
                r#"
                    import { connect } from 'tls';

                    export async function test(port) {
                        return new Promise((resolve) => {
                            const socket = connect({
                                port,
                                host: '127.0.0.1',
                                servername: 'localhost',
                                rejectUnauthorized: true,
                            });
                            socket.on('secureConnect', () => resolve('connected'));
                            socket.on('error', () => resolve('error'));
                        });
                    }
                "#,
                (port,),
            )
            .await;

            assert_eq!(result, "error");
        })
    })
    .await;
}

#[tokio::test]
async fn connect_port_host_options_overload() {
    run_tls_test(|ctx| {
        Box::pin(async move {
            init_tls_modules(&ctx).await;
            let server = MockServer::start().await.unwrap();
            let port = server.address().port();
            let ca = server.ca();

            let ok: bool = eval_tls_test(
                &ctx,
                r#"
                    import { connect } from 'tls';

                    export async function test(port, ca) {
                        return new Promise((resolve, reject) => {
                            const socket = connect(port, '127.0.0.1', {
                                servername: 'localhost',
                                ca,
                                rejectUnauthorized: true,
                            }, () => {
                                resolve(socket.authorized === true);
                            });
                            socket.on('error', reject);
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

#[tokio::test]
async fn create_secure_context_and_get_ciphers() {
    run_tls_test(|ctx| {
        Box::pin(async move {
            init_tls_modules(&ctx).await;

            let ok: bool = eval_tls_test(
                &ctx,
                r#"
                    import tls from 'tls';

                    export function test() {
                        const ciphers = tls.getCiphers();
                        const ctx = tls.createSecureContext({ minVersion: 'TLSv1.2' });
                        return Array.isArray(ciphers) && ciphers.length > 0 && ctx != null;
                    }
                "#,
                (),
            )
            .await;

            assert!(ok);
        })
    })
    .await;
}

#[tokio::test]
async fn api_return_shapes() {
    run_tls_test(|ctx| {
        Box::pin(async move {
            init_tls_modules(&ctx).await;
            let server = MockServer::start().await.unwrap();
            let port = server.address().port();
            let ca = server.ca();

            let ok: bool = eval_tls_test(
                &ctx,
                r#"
                    import { connect } from 'tls';

                    export async function test(port, ca) {
                        return new Promise((resolve, reject) => {
                            const socket = connect({
                                port,
                                host: '127.0.0.1',
                                servername: 'localhost',
                                ca,
                                rejectUnauthorized: true,
                            });
                            socket.on('secureConnect', () => {
                                const protocol = socket.getProtocol();
                                const cipher = socket.getCipher();
                                const peer = socket.getPeerCertificate();
                                resolve(
                                    socket.alpnProtocol === false &&
                                    protocol != null &&
                                    cipher?.standardName != null &&
                                    typeof peer === 'object' &&
                                    peer != null &&
                                    Object.keys(peer).length > 0
                                );
                            });
                            socket.on('error', reject);
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

#[tokio::test]
async fn listen_port_callback_overload() {
    use lexe_test_tls::fixtures;

    run_tls_test(|ctx| {
        Box::pin(async move {
            init_tls_modules(&ctx).await;

            let ok: bool = eval_tls_test(
                &ctx,
                r#"
                    import tls from 'tls';

                    export async function test(cert, key) {
                        const server = tls.createServer({ cert, key });
                        const port = await new Promise((resolve, reject) => {
                            server.listen(0, (err) => {
                                if (err) reject(err);
                                else resolve(server.address().port);
                            });
                            server.on('error', reject);
                        });
                        server.close();
                        return port > 0;
                    }
                "#,
                (fixtures::SERVER_CERT, fixtures::SERVER_KEY),
            )
            .await;

            assert!(ok);
        })
    })
    .await;
}

#[tokio::test]
async fn double_listen_is_rejected() {
    use lexe_test_tls::fixtures;

    run_tls_test(|ctx| {
        Box::pin(async move {
            init_tls_modules(&ctx).await;

            let caught: bool = eval_tls_test(
                &ctx,
                r#"
                    import tls from 'tls';

                    export function test(cert, key) {
                        const server = tls.createServer({ cert, key });
                        server.listen(0, '127.0.0.1');
                        try {
                            server.listen(0, '127.0.0.1');
                            return false;
                        } catch (err) {
                            return String(err.message).includes('ERR_SERVER_ALREADY_LISTEN');
                        }
                    }
                "#,
                (fixtures::SERVER_CERT, fixtures::SERVER_KEY),
            )
            .await;

            assert!(caught);
        })
    })
    .await;
}

#[tokio::test]
async fn invalid_listen_then_valid_listen_succeeds() {
    use lexe_test_tls::fixtures;

    run_tls_test(|ctx| {
        Box::pin(async move {
            init_tls_modules(&ctx).await;

            let ok: bool = eval_tls_test(
                &ctx,
                r#"
                    import tls from 'tls';

                    export async function test(cert, key) {
                        const server = tls.createServer({ cert, key });
                        try {
                            server.listen(0, {});
                        } catch (_) {}

                        return new Promise((resolve, reject) => {
                            server.listen(0, '127.0.0.1', () => {
                                resolve(server.address().port > 0);
                            });
                            server.on('error', reject);
                        });
                    }
                "#,
                (fixtures::SERVER_CERT, fixtures::SERVER_KEY),
            )
            .await;

            assert!(ok);
        })
    })
    .await;
}

#[tokio::test]
async fn listen_in_close_callback_succeeds() {
    use lexe_test_tls::fixtures;

    run_tls_test(|ctx| {
        Box::pin(async move {
            init_tls_modules(&ctx).await;

            let ok: bool = eval_tls_test(
                &ctx,
                r#"
                    import tls from 'tls';

                    export async function test(cert, key) {
                        const server = tls.createServer({ cert, key });
                        return new Promise((resolve, reject) => {
                            server.listen(0, '127.0.0.1', () => {
                                server.close(() => {
                                    server.listen(0, '127.0.0.1', () => {
                                        const port = server.address().port;
                                        server.close();
                                        resolve(port > 0);
                                    });
                                });
                            });
                            server.on('error', reject);
                        });
                    }
                "#,
                (fixtures::SERVER_CERT, fixtures::SERVER_KEY),
            )
            .await;

            assert!(ok);
        })
    })
    .await;
}
