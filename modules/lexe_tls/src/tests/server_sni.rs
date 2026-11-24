// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#[cfg(test)]
mod server_sni {
    use lexe_test_tls::fixtures;

    use crate::test_support::{eval_tls_test, init_tls_modules, run_tls_test};

    const SNI_HELPERS: &str = r#"
        import tls from 'tls';
        import { connect } from 'tls';

        async function startServer(mode, defaultCert, defaultKey, rootCa, altCert, altKey, wildcardCert, wildcardKey) {
            const server = tls.createServer({
                cert: defaultCert,
                key: defaultKey,
                ca: rootCa,
                rejectUnauthorized: true,
            });

            if (mode === 'alt' || mode === 'isolated') {
                server.addContext('alt.localhost', tls.createSecureContext({
                    cert: altCert,
                    key: altKey,
                }));
            } else if (mode === 'wildcard') {
                server.addContext('*.example.com', tls.createSecureContext({
                    cert: wildcardCert,
                    key: wildcardKey,
                }));
            }

            await new Promise((resolve, reject) => {
                server.listen(0, '127.0.0.1', resolve);
                server.on('error', reject);
            });
            return server.address().port;
        }

        async function peerCn(port, servername, rootCa, permissive) {
            return new Promise((resolve, reject) => {
                const socket = connect({
                    port,
                    host: '127.0.0.1',
                    servername,
                    ca: rootCa,
                    rejectUnauthorized: !permissive,
                });
                socket.on('secureConnect', () => {
                    const cert = socket.getPeerCertificate();
                    resolve(cert.subject?.CN ?? null);
                });
                socket.on('error', reject);
            });
        }
    "#;

    fn cert_args() -> (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    ) {
        (
            fixtures::SERVER_CERT,
            fixtures::SERVER_KEY,
            fixtures::ROOT_CA,
            fixtures::ALT_SERVER_CERT,
            fixtures::ALT_SERVER_KEY,
            fixtures::WILDCARD_SERVER_CERT,
            fixtures::WILDCARD_SERVER_KEY,
        )
    }

    #[tokio::test]
    async fn sni_exact_match_returns_alt_cert() {
        run_tls_test(|ctx| {
            Box::pin(async move {
                init_tls_modules(&ctx).await;
                let (default_cert, default_key, root_ca, alt_cert, alt_key, wildcard_cert, wildcard_key) =
                    cert_args();
                let cn: Option<String> = eval_tls_test(
                    &ctx,
                    &format!(
                        r#"{helpers}
                        export async function test(defaultCert, defaultKey, rootCa, altCert, altKey, wildcardCert, wildcardKey) {{
                            const port = await startServer('alt', defaultCert, defaultKey, rootCa, altCert, altKey, wildcardCert, wildcardKey);
                            return peerCn(port, 'alt.localhost', rootCa);
                        }}"#,
                        helpers = SNI_HELPERS
                    ),
                    (
                        default_cert,
                        default_key,
                        root_ca,
                        alt_cert,
                        alt_key,
                        wildcard_cert,
                        wildcard_key,
                    ),
                )
                .await;
                assert_eq!(cn.as_deref(), Some("alt.localhost"));
            })
        })
        .await;
    }

    #[tokio::test]
    async fn sni_default_cert_for_unknown_name() {
        run_tls_test(|ctx| {
            Box::pin(async move {
                init_tls_modules(&ctx).await;
                let (default_cert, default_key, root_ca, alt_cert, alt_key, wildcard_cert, wildcard_key) =
                    cert_args();
                let cn: Option<String> = eval_tls_test(
                    &ctx,
                    &format!(
                        r#"{helpers}
                        export async function test(defaultCert, defaultKey, rootCa, altCert, altKey, wildcardCert, wildcardKey) {{
                            const port = await startServer('alt', defaultCert, defaultKey, rootCa, altCert, altKey, wildcardCert, wildcardKey);
                            return peerCn(port, 'localhost', rootCa);
                        }}"#,
                        helpers = SNI_HELPERS
                    ),
                    (
                        default_cert,
                        default_key,
                        root_ca,
                        alt_cert,
                        alt_key,
                        wildcard_cert,
                        wildcard_key,
                    ),
                )
                .await;
                assert_eq!(cn.as_deref(), Some("localhost"));
            })
        })
        .await;
    }

    #[tokio::test]
    async fn sni_wildcard_matches_single_label() {
        run_tls_test(|ctx| {
            Box::pin(async move {
                init_tls_modules(&ctx).await;
                let (default_cert, default_key, root_ca, alt_cert, alt_key, wildcard_cert, wildcard_key) =
                    cert_args();
                let cn: Option<String> = eval_tls_test(
                    &ctx,
                    &format!(
                        r#"{helpers}
                        export async function test(defaultCert, defaultKey, rootCa, altCert, altKey, wildcardCert, wildcardKey) {{
                            const port = await startServer('wildcard', defaultCert, defaultKey, rootCa, altCert, altKey, wildcardCert, wildcardKey);
                            return peerCn(port, 'www.example.com', rootCa);
                        }}"#,
                        helpers = SNI_HELPERS
                    ),
                    (
                        default_cert,
                        default_key,
                        root_ca,
                        alt_cert,
                        alt_key,
                        wildcard_cert,
                        wildcard_key,
                    ),
                )
                .await;
                assert_eq!(cn.as_deref(), Some("*.example.com"));
            })
        })
        .await;
    }

    #[tokio::test]
    async fn sni_servers_are_isolated() {
        run_tls_test(|ctx| {
            Box::pin(async move {
                init_tls_modules(&ctx).await;
                let (default_cert, default_key, root_ca, alt_cert, alt_key, wildcard_cert, wildcard_key) =
                    cert_args();
                let result: Vec<Option<String>> = eval_tls_test(
                    &ctx,
                    &format!(
                        r#"{helpers}
                        export async function test(defaultCert, defaultKey, rootCa, altCert, altKey, wildcardCert, wildcardKey) {{
                            const portA = await startServer('isolated', defaultCert, defaultKey, rootCa, altCert, altKey, wildcardCert, wildcardKey);
                            const portB = await startServer('default', defaultCert, defaultKey, rootCa, altCert, altKey, wildcardCert, wildcardKey);
                            const cnA = await peerCn(portA, 'alt.localhost', rootCa);
                            const cnB = await peerCn(portB, 'alt.localhost', rootCa, true);
                            return [cnA, cnB];
                        }}"#,
                        helpers = SNI_HELPERS
                    ),
                    (
                        default_cert,
                        default_key,
                        root_ca,
                        alt_cert,
                        alt_key,
                        wildcard_cert,
                        wildcard_key,
                    ),
                )
                .await;
                assert_eq!(result[0].as_deref(), Some("alt.localhost"));
                assert_eq!(result[1].as_deref(), Some("localhost"));
            })
        })
        .await;
    }

    #[tokio::test]
    async fn tls_client_error_includes_socket() {
        run_tls_test(|ctx| {
            Box::pin(async move {
                init_tls_modules(&ctx).await;
                let (default_cert, default_key, root_ca, _, _, _, _) = cert_args();
                let result: Vec<bool> = eval_tls_test(
                    &ctx,
                    r#"
                        import tls from 'tls';

                        export async function test(cert, key, ca) {
                            return new Promise((resolve) => {
                                const server = tls.createServer({
                                    cert,
                                    key,
                                    ca,
                                    rejectUnauthorized: true,
                                });
                                server.on('tlsClientError', (err, socket) => {
                                    resolve([socket != null, socket?.encrypted === true]);
                                    server.close();
                                });
                                server.listen(0, '127.0.0.1', () => {
                                    const port = server.address().port;
                                    const client = tls.connect({
                                        port,
                                        host: '127.0.0.1',
                                        servername: 'wrong.example.com',
                                        rejectUnauthorized: true,
                                    });
                                    client.on('secureConnect', () => resolve([false, false]));
                                });
                            });
                        }
                    "#,
                    (default_cert, default_key, root_ca),
                )
                .await;
                assert!(result[0]);
                assert!(result[1]);
            })
        })
        .await;
    }

    #[tokio::test]
    async fn server_get_certificate_reflects_sni_context() {
        run_tls_test(|ctx| {
            Box::pin(async move {
                init_tls_modules(&ctx).await;
                let (default_cert, default_key, root_ca, alt_cert, alt_key, _, _) = cert_args();
                let cn: Option<String> = eval_tls_test(
                    &ctx,
                    &format!(
                        r#"{helpers}
                        export async function test(defaultCert, defaultKey, rootCa, altCert, altKey) {{
                            return new Promise((resolve, reject) => {{
                                const server = tls.createServer({{
                                    cert: defaultCert,
                                    key: defaultKey,
                                    ca: rootCa,
                                }});
                                server.addContext('alt.localhost', tls.createSecureContext({{
                                    cert: altCert,
                                    key: altKey,
                                }}));
                                server.on('secureConnection', (socket) => {{
                                    resolve(socket.getCertificate()?.subject?.CN ?? null);
                                }});
                                server.listen(0, '127.0.0.1', () => {{
                                    const port = server.address().port;
                                    const client = connect({{
                                        port,
                                        host: '127.0.0.1',
                                        servername: 'alt.localhost',
                                        ca: rootCa,
                                        rejectUnauthorized: true,
                                    }});
                                    client.on('error', reject);
                                }});
                                server.on('error', reject);
                            }});
                        }}"#,
                        helpers = SNI_HELPERS
                    ),
                    (
                        default_cert,
                        default_key,
                        root_ca,
                        alt_cert,
                        alt_key,
                    ),
                )
                .await;
                assert_eq!(cn.as_deref(), Some("alt.localhost"));
            })
        })
        .await;
    }
}
