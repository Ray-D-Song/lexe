// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#[cfg(test)]
mod mtls {
    use lexe_test_tls::fixtures;

    use crate::test_support::{eval_tls_test, init_tls_modules, run_tls_test};

    #[tokio::test]
    async fn strict_mtls_accepts_valid_client_cert() {
        run_tls_test(|ctx| {
            Box::pin(async move {
                init_tls_modules(&ctx).await;

                let result: Vec<bool> = eval_tls_test(
                    &ctx,
                    r#"
                        import tls, { connect } from 'tls';

                        export async function test(serverCert, serverKey, rootCa, clientCert, clientKey) {
                            return new Promise((resolve, reject) => {
                                const server = tls.createServer({
                                    cert: serverCert,
                                    key: serverKey,
                                    ca: rootCa,
                                    requestCert: true,
                                    rejectUnauthorized: true,
                                });

                                server.on('secureConnection', (socket) => {
                                    resolve([
                                        socket.authorized === true,
                                        socket.encrypted === true,
                                    ]);
                                });
                                server.on('tlsClientError', () => resolve([false, false]));
                                server.on('error', reject);

                                server.listen(0, '127.0.0.1', () => {
                                    const port = server.address().port;
                                    const client = connect({
                                        port,
                                        host: '127.0.0.1',
                                        servername: 'localhost',
                                        ca: rootCa,
                                        cert: clientCert,
                                        key: clientKey,
                                        rejectUnauthorized: true,
                                    });
                                    client.on('error', reject);
                                });
                            });
                        }
                    "#,
                    (
                        fixtures::SERVER_CERT,
                        fixtures::SERVER_KEY,
                        fixtures::ROOT_CA,
                        fixtures::CLIENT_CERT,
                        fixtures::CLIENT_KEY,
                    ),
                )
                .await;

                assert!(result[0], "server socket should be authorized");
                assert!(result[1], "server socket should be encrypted");
            })
        })
        .await;
    }

    #[tokio::test]
    async fn strict_mtls_rejects_missing_client_cert() {
        run_tls_test(|ctx| {
            Box::pin(async move {
                init_tls_modules(&ctx).await;

                let ok: bool = eval_tls_test(
                    &ctx,
                    r#"
                        import tls, { connect } from 'tls';

                        export async function test(serverCert, serverKey, rootCa) {
                            return new Promise((resolve) => {
                                let sawSecure = false;
                                const server = tls.createServer({
                                    cert: serverCert,
                                    key: serverKey,
                                    ca: rootCa,
                                    requestCert: true,
                                    rejectUnauthorized: true,
                                });

                                server.on('secureConnection', () => {
                                    sawSecure = true;
                                });
                                server.on('tlsClientError', () => resolve(!sawSecure));
                                server.listen(0, '127.0.0.1', () => {
                                    const port = server.address().port;
                                    const client = connect({
                                        port,
                                        host: '127.0.0.1',
                                        servername: 'localhost',
                                        ca: rootCa,
                                        rejectUnauthorized: true,
                                    });
                                    client.on('error', () => {});
                                });
                            });
                        }
                    "#,
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
    async fn strict_mtls_rejects_wrong_ca_client_cert() {
        run_tls_test(|ctx| {
            Box::pin(async move {
                init_tls_modules(&ctx).await;

                let ok: bool = eval_tls_test(
                    &ctx,
                    r#"
                        import tls, { connect } from 'tls';

                        export async function test(serverCert, serverKey, rootCa, clientCert, clientKey) {
                            return new Promise((resolve) => {
                                let sawSecure = false;
                                const server = tls.createServer({
                                    cert: serverCert,
                                    key: serverKey,
                                    ca: rootCa,
                                    requestCert: true,
                                    rejectUnauthorized: true,
                                });

                                server.on('secureConnection', () => {
                                    sawSecure = true;
                                });
                                server.on('tlsClientError', () => resolve(!sawSecure));
                                server.listen(0, '127.0.0.1', () => {
                                    const port = server.address().port;
                                    const client = connect({
                                        port,
                                        host: '127.0.0.1',
                                        servername: 'localhost',
                                        ca: rootCa,
                                        cert: clientCert,
                                        key: clientKey,
                                        rejectUnauthorized: true,
                                    });
                                    client.on('error', () => {});
                                });
                            });
                        }
                    "#,
                    (
                        fixtures::SERVER_CERT,
                        fixtures::SERVER_KEY,
                        fixtures::ROOT_CA,
                        fixtures::WRONG_CLIENT_CERT,
                        fixtures::WRONG_CLIENT_KEY,
                    ),
                )
                .await;

                assert!(ok);
            })
        })
        .await;
    }
}
