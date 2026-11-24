// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#[cfg(test)]
mod security {
    use lexe_test_tls::MockServer;

    use crate::test_support::{eval_tls_test, init_tls_modules, run_tls_test};

    #[tokio::test]
    async fn permissive_unknown_ca_authorized_false() {
        run_tls_test(|ctx| {
            Box::pin(async move {
                init_tls_modules(&ctx).await;
                let server = MockServer::start().await.unwrap();
                let port = server.address().port();

                let result: Vec<bool> = eval_tls_test(
                    &ctx,
                    r#"
                        import { connect } from 'tls';

                        export async function test(port) {
                            return new Promise((resolve) => {
                                const socket = connect({
                                    port,
                                    host: '127.0.0.1',
                                    servername: 'localhost',
                                    rejectUnauthorized: false,
                                });
                                socket.on('secureConnect', () => {
                                    resolve([
                                        socket.authorized,
                                        socket.encrypted,
                                        socket.authorizationError != null,
                                    ]);
                                });
                                socket.on('error', () => resolve([false, false, false]));
                            });
                        }
                    "#,
                    (port,),
                )
                .await;

                assert!(!result[0]);
                assert!(result[1]);
                assert!(result[2]);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn permissive_valid_ca_authorized_true() {
        run_tls_test(|ctx| {
            Box::pin(async move {
                init_tls_modules(&ctx).await;
                let server = MockServer::start().await.unwrap();
                let port = server.address().port();
                let ca = server.ca();

                let result: Vec<bool> = eval_tls_test(
                    &ctx,
                    r#"
                        import { connect } from 'tls';

                        export async function test(port, ca) {
                            return new Promise((resolve) => {
                                const socket = connect({
                                    port,
                                    host: '127.0.0.1',
                                    servername: 'localhost',
                                    ca,
                                    rejectUnauthorized: false,
                                });
                                socket.on('secureConnect', () => {
                                    resolve([socket.authorized, socket.encrypted]);
                                });
                                socket.on('error', () => resolve([false, false]));
                            });
                        }
                    "#,
                    (port, ca),
                )
                .await;

                assert!(result[0]);
                assert!(result[1]);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn strict_wrong_hostname_rejects() {
        run_tls_test(|ctx| {
            Box::pin(async move {
                init_tls_modules(&ctx).await;
                let server = MockServer::start().await.unwrap();
                let port = server.address().port();
                let ca = server.ca();

                let result: String = eval_tls_test(
                    &ctx,
                    r#"
                        import { connect } from 'tls';

                        export async function test(port, ca) {
                            return new Promise((resolve) => {
                                const socket = connect({
                                    port,
                                    host: '127.0.0.1',
                                    servername: 'wrong.example.com',
                                    ca,
                                    rejectUnauthorized: true,
                                });
                                socket.on('secureConnect', () => resolve('connected'));
                                socket.on('error', () => resolve('error'));
                            });
                        }
                    "#,
                    (port, ca),
                )
                .await;

                assert_eq!(result, "error");
            })
        })
        .await;
    }

    #[tokio::test]
    async fn permissive_wrong_hostname_authorized_false() {
        run_tls_test(|ctx| {
            Box::pin(async move {
                init_tls_modules(&ctx).await;
                let server = MockServer::start().await.unwrap();
                let port = server.address().port();
                let ca = server.ca();

                let result: Vec<bool> = eval_tls_test(
                    &ctx,
                    r#"
                        import { connect } from 'tls';

                        export async function test(port, ca) {
                            return new Promise((resolve) => {
                                const socket = connect({
                                    port,
                                    host: '127.0.0.1',
                                    servername: 'wrong.example.com',
                                    ca,
                                    rejectUnauthorized: false,
                                });
                                socket.on('secureConnect', () => {
                                    resolve([socket.authorized, socket.encrypted]);
                                });
                                socket.on('error', () => resolve([false, false]));
                            });
                        }
                    "#,
                    (port, ca),
                )
                .await;

                assert!(!result[0]);
                assert!(result[1]);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn ip_host_without_sni_connects() {
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
                                const socket = connect({ port, host: '127.0.0.1', ca, rejectUnauthorized: true });
                                socket.on('secureConnect', () => {
                                    resolve(socket.authorized === true && socket.servername == null);
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
    async fn custom_check_server_identity_failure_strict() {
        run_tls_test(|ctx| {
            Box::pin(async move {
                init_tls_modules(&ctx).await;
                let server = MockServer::start().await.unwrap();
                let port = server.address().port();
                let ca = server.ca();

                let result: String = eval_tls_test(
                    &ctx,
                    r#"
                        import { connect } from 'tls';

                        export async function test(port, ca) {
                            return new Promise((resolve) => {
                                const socket = connect({
                                    port,
                                    host: '127.0.0.1',
                                    servername: 'localhost',
                                    ca,
                                    rejectUnauthorized: true,
                                    checkServerIdentity: () => new Error('custom identity failure'),
                                });
                                socket.on('secureConnect', () => resolve('connected'));
                                socket.on('error', (err) => resolve(err.message));
                            });
                        }
                    "#,
                    (port, ca),
                )
                .await;

                assert_eq!(result, "custom identity failure");
            })
        })
        .await;
    }

    #[tokio::test]
    async fn ip_host_sends_no_clienthello_sni() {
        run_tls_test(|ctx| {
            Box::pin(async move {
                init_tls_modules(&ctx).await;

                let seen: Option<String> = eval_tls_test(
                    &ctx,
                    r#"
                        import tls, { connect } from 'tls';

                        export async function test(cert, key, ca) {
                            return new Promise((resolve, reject) => {
                                const server = tls.createServer({ cert, key, ca });
                                server.on('secureConnection', (socket) => {
                                    resolve(socket.servername ?? null);
                                });
                                server.listen(0, '127.0.0.1', () => {
                                    const port = server.address().port;
                                    const client = connect({
                                        port,
                                        host: '127.0.0.1',
                                        ca,
                                        rejectUnauthorized: true,
                                    });
                                    client.on('error', reject);
                                });
                                server.on('error', reject);
                            });
                        }
                    "#,
                    (
                        lexe_test_tls::fixtures::SERVER_CERT,
                        lexe_test_tls::fixtures::SERVER_KEY,
                        lexe_test_tls::fixtures::ROOT_CA,
                    ),
                )
                .await;

                assert!(seen.is_none());
            })
        })
        .await;
    }
}
