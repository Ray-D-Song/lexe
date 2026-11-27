use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use hyper_util::{
    client::legacy::{connect::HttpConnector, Client},
    rt::{TokioExecutor, TokioTimer},
};
use lexe_dns_cache::CachedDnsResolver;
use once_cell::sync::Lazy;

pub type HyperBodyError = Box<dyn std::error::Error + Send + Sync>;

use crate::get_pool_idle_timeout;

#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))]
use crate::get_http_version;

// Rustls-based TLS backends
#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))]
mod rustls_client {
    use super::*;
    use hyper_rustls::HttpsConnector;
    use lexe_tls::TLS_CONFIG;
    use rustls::ClientConfig;

    #[cfg(feature = "http2")]
    use crate::HttpVersion;

    pub type HyperClient =
        Client<HttpsConnector<HttpConnector<CachedDnsResolver>>, BoxBody<Bytes, HyperBodyError>>;

    pub static HTTP_CLIENT: Lazy<Result<HyperClient, Box<dyn std::error::Error + Send + Sync>>> =
        Lazy::new(|| build_client(None));

    /// Dedicated pooled client for `fetch()` that negotiates HTTP/2 via TLS
    /// ALPN when built with `http2` while still speaking HTTP/1.1 cleanly.
    pub static FETCH_HTTP_CLIENT: Lazy<
        Result<HyperClient, Box<dyn std::error::Error + Send + Sync>>,
    > = Lazy::new(|| build_client_with_alpn(None, cfg!(feature = "http2")));

    pub fn build_client(
        tls_config: Option<ClientConfig>,
    ) -> Result<HyperClient, Box<dyn std::error::Error + Send + Sync>> {
        let allow_h2 = match get_http_version() {
            #[cfg(feature = "http2")]
            HttpVersion::Http2 => true,
            _ => false,
        };
        build_client_with_alpn(tls_config, allow_h2)
    }

    pub fn build_client_with_alpn(
        tls_config: Option<ClientConfig>,
        allow_h2: bool,
    ) -> Result<HyperClient, Box<dyn std::error::Error + Send + Sync>> {
        let pool_idle_timeout = get_pool_idle_timeout();

        let config = if let Some(tls_config) = tls_config {
            tls_config
        } else {
            match &*TLS_CONFIG {
                Ok(tls_config) => tls_config.clone(),
                Err(e) => return Err(e.to_string().into()),
            }
        };

        let builder = hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config(config)
            .https_or_http();

        let mut cache_dns_connector = CachedDnsResolver::new().into_http_connector();
        cache_dns_connector.enforce_http(false);

        let https = if allow_h2 {
            #[cfg(feature = "http2")]
            {
                builder
                    .enable_all_versions()
                    .wrap_connector(cache_dns_connector)
            }
            #[cfg(not(feature = "http2"))]
            {
                builder.enable_http1().wrap_connector(cache_dns_connector)
            }
        } else {
            builder.enable_http1().wrap_connector(cache_dns_connector)
        };

        Ok(Client::builder(TokioExecutor::new())
            .pool_idle_timeout(pool_idle_timeout)
            .pool_timer(TokioTimer::new())
            .build(https))
    }
}

// OpenSSL TLS backend
#[cfg(feature = "tls-openssl")]
mod openssl_client {
    use super::*;
    use hyper_openssl::client::legacy::HttpsConnector;
    use lexe_tls::TLS_CONFIG;
    use openssl::ssl::SslConnectorBuilder;

    pub type HyperClient =
        Client<HttpsConnector<HttpConnector<CachedDnsResolver>>, BoxBody<Bytes, HyperBodyError>>;

    pub static HTTP_CLIENT: Lazy<Result<HyperClient, Box<dyn std::error::Error + Send + Sync>>> =
        Lazy::new(|| build_client(None));

    pub static FETCH_HTTP_CLIENT: Lazy<
        Result<HyperClient, Box<dyn std::error::Error + Send + Sync>>,
    > = Lazy::new(|| build_client(None));

    pub fn build_client(
        tls_config: Option<SslConnectorBuilder>,
    ) -> Result<HyperClient, Box<dyn std::error::Error + Send + Sync>> {
        let pool_idle_timeout = get_pool_idle_timeout();

        let connector = if let Some(tls_config) = tls_config {
            tls_config
        } else {
            match TLS_CONFIG.as_ref() {
                Ok(_builder) => {
                    // Clone the builder by creating a new one with same settings
                    lexe_tls::build_client_config(lexe_tls::BuildClientConfigOptions {
                        reject_unauthorized: true,
                        ca: None,
                    })?
                },
                Err(e) => return Err(e.to_string().into()),
            }
        };

        let mut cache_dns_connector = CachedDnsResolver::new().into_http_connector();
        cache_dns_connector.enforce_http(false);

        let mut https = HttpsConnector::with_connector(cache_dns_connector, connector)?;
        https.set_callback(|ssl, _| {
            #[cfg(feature = "http2")]
            ssl.set_alpn_protos(b"\x02h2\x08http/1.1")?;
            #[cfg(not(feature = "http2"))]
            ssl.set_alpn_protos(b"\x08http/1.1")?;
            Ok(())
        });

        Ok(Client::builder(TokioExecutor::new())
            .pool_idle_timeout(pool_idle_timeout)
            .pool_timer(TokioTimer::new())
            .build(https))
    }
}

// Re-export based on feature
#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))]
pub use rustls_client::*;

#[cfg(feature = "tls-openssl")]
pub use openssl_client::*;

/// Build a client for one request that supplied its own TLS verification
/// options.
///
/// `https.get`/`https.request` in Node read `rejectUnauthorized` and `ca` from
/// the request options; the shared pooled client cannot, because it is built
/// once from the process-wide `TLS_CONFIG`. Requests that pass neither keep
/// using the pool.
pub fn build_client_for_request(
    ctx: &rquickjs::Ctx<'_>,
    options: crate::client_request::RequestTlsOptions,
) -> rquickjs::Result<HyperClient> {
    #[cfg(any(feature = "tls-ring", feature = "tls-aws-lc", feature = "tls-graviola"))]
    {
        let reject_unauthorized = options.reject_unauthorized.unwrap_or(true);
        let config = lexe_tls::build_client_config(lexe_tls::BuildClientConfigOptions {
            reject_unauthorized,
            ca: options.ca,
        })
        .map_err(|err| rquickjs::Exception::throw_message(ctx, &err.to_string()))?;
        build_client(Some(config))
            .map_err(|err| rquickjs::Exception::throw_message(ctx, &err.to_string()))
    }
    #[cfg(feature = "tls-openssl")]
    {
        let reject_unauthorized = options.reject_unauthorized.unwrap_or(true);
        // The openssl builder has no per-request `ca` slot in this back-end;
        // honour verification, which is what requests actually depend on.
        let _ = options.ca;
        let builder = lexe_tls::build_client_config(lexe_tls::BuildClientConfigOptions {
            reject_unauthorized,
            ca: None,
        })
        .map_err(|err| rquickjs::Exception::throw_message(ctx, &err.to_string()))?;
        build_client(Some(builder))
            .map_err(|err| rquickjs::Exception::throw_message(ctx, &err.to_string()))
    }
}
