use std::{
    io,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, SystemTime},
};

use crate::config::Config;

const DEFAULT_MAX_RETRIES: u32 = 2;
const INITIAL_RETRY_DELAY: Duration = Duration::from_millis(500);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(8);
const MAX_RETRY_AFTER: Duration = Duration::from_mins(1);

static RETRY_STATUSES: &[reqwest::StatusCode] = &[
    reqwest::StatusCode::REQUEST_TIMEOUT,
    reqwest::StatusCode::TOO_MANY_REQUESTS,
    reqwest::StatusCode::INTERNAL_SERVER_ERROR,
    reqwest::StatusCode::BAD_GATEWAY,
    reqwest::StatusCode::SERVICE_UNAVAILABLE,
    reqwest::StatusCode::GATEWAY_TIMEOUT,
];

#[derive(Clone, serde::Deserialize)]
pub struct BasicAuth {
    pub username: String,
    pub password: Option<String>,
}

/// A DNS resolver backed by `hickory-resolver` with Cloudflare DNS and
/// IPv4+IPv6 support.
pub struct HickoryDnsResolver(Arc<hickory_resolver::TokioResolver>);

impl HickoryDnsResolver {
    pub fn new() -> Self {
        let mut builder = hickory_resolver::TokioResolver::builder_tokio()
            .unwrap_or_else(|_| {
                hickory_resolver::TokioResolver::builder_with_config(
                hickory_resolver::config::ResolverConfig::cloudflare(),
                hickory_resolver::name_server::TokioConnectionProvider::default(
                ),
            )
            });
        builder.options_mut().ip_strategy =
            hickory_resolver::config::LookupIpStrategy::Ipv4AndIpv6;
        Self(Arc::new(builder.build()))
    }
}

impl reqwest::dns::Resolve for HickoryDnsResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let resolver = Arc::clone(&self.0);
        Box::pin(async move {
            let lookup = resolver.lookup_ip(name.as_str()).await?;
            drop(resolver);
            let addrs: reqwest::dns::Addrs = Box::new(
                lookup.into_iter().map(|ip_addr| SocketAddr::new(ip_addr, 0)),
            );
            Ok(addrs)
        })
    }
}

/// Parse `Retry-After` or `retry-after-ms` header values into a `Duration`.
#[must_use]
#[inline]
fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    if let Some(val) = headers.get("retry-after-ms")
        && let Ok(s) = val.to_str()
        && let Ok(ms) = s.parse()
    {
        return Some(Duration::from_millis(ms));
    }

    if let Some(val) = headers.get(reqwest::header::RETRY_AFTER)
        && let Ok(s) = val.to_str()
    {
        if let Ok(sec) = s.parse() {
            return Some(Duration::from_secs(sec));
        }

        if let Ok(parsed) = httpdate::parse_http_date(s)
            && let Ok(dur) = parsed.duration_since(SystemTime::now())
        {
            return Some(dur);
        }
    }
    None
}

/// Calculate the delay before the next retry attempt.
///
/// Uses `Retry-After` header if present (capped at 60 seconds), otherwise
/// applies exponential backoff with jitter.
#[must_use]
fn calculate_retry_timeout(
    headers: Option<&reqwest::header::HeaderMap>,
    attempt: u32,
) -> Option<Duration> {
    if let Some(h) = headers
        && let Some(after) = parse_retry_after(h)
    {
        if after > MAX_RETRY_AFTER {
            return None;
        }
        return Some(after);
    }

    let base = INITIAL_RETRY_DELAY
        .saturating_mul(2_u32.pow(attempt))
        .min(MAX_RETRY_DELAY);
    let jitter = 0.25_f64.mul_add(-rand::random::<f64>(), 1.0);
    Some(base.mul_f64(jitter))
}

/// A `reqwest_middleware` middleware that retries failed requests with
/// exponential backoff and jitter.
///
/// Retries on: timeout (408), too-many-requests (429), server errors (5xx),
/// and connection errors. Uses `Retry-After` / `retry-after-ms` headers
/// when available.
pub struct RetryMiddleware;

#[async_trait::async_trait]
impl reqwest_middleware::Middleware for RetryMiddleware {
    async fn handle(
        &self,
        req: reqwest::Request,
        extensions: &mut http::Extensions,
        next: reqwest_middleware::Next<'_>,
    ) -> reqwest_middleware::Result<reqwest::Response> {
        let mut attempt: u32 = 0;
        loop {
            let req = req.try_clone().ok_or_else(|| {
                reqwest_middleware::Error::middleware(io::Error::other(
                    "Request object is not cloneable",
                ))
            })?;

            match next.clone().run(req, extensions).await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_client_error() || status.is_server_error() {
                        if attempt < DEFAULT_MAX_RETRIES
                            && RETRY_STATUSES.contains(&status)
                            && let Some(delay) = calculate_retry_timeout(
                                Some(resp.headers()),
                                attempt,
                            )
                        {
                            tokio::time::sleep(delay).await;
                            attempt = attempt.saturating_add(1);
                            continue;
                        }
                        resp.error_for_status_ref()?;
                    }
                    return Ok(resp);
                }
                Err(err) => {
                    if attempt < DEFAULT_MAX_RETRIES
                        && err.is_connect()
                        && let Some(delay) =
                            calculate_retry_timeout(None, attempt)
                    {
                        tokio::time::sleep(delay).await;
                        attempt = attempt.saturating_add(1);
                        continue;
                    }
                    return Err(err);
                }
            }
        }
    }
}

/// Build a `reqwest` client with retry middleware and custom DNS resolver.
pub fn create_reqwest_client<R>(
    config: &Config,
    dns_resolver: Arc<R>,
) -> reqwest::Result<reqwest_middleware::ClientWithMiddleware>
where
    R: reqwest::dns::Resolve + 'static,
{
    let mut builder = reqwest::ClientBuilder::new()
        .user_agent(&config.scraping.user_agent)
        .timeout(config.scraping.timeout)
        .connect_timeout(config.scraping.connect_timeout)
        .dns_resolver(dns_resolver);

    if let Some(proxy) = &config.scraping.proxy {
        builder = builder.proxy(reqwest::Proxy::all(proxy.clone())?);
    }

    let client = builder.build()?;
    let client_with_middleware = reqwest_middleware::ClientBuilder::new(client)
        .with(RetryMiddleware)
        .build();

    Ok(client_with_middleware)
}

#[cfg(test)]
#[expect(
    clippy::default_numeric_fallback,
    clippy::inline_modules,
    clippy::map_with_unused_argument_over_ranges,
    clippy::redundant_test_prefix,
    clippy::unwrap_used
)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_retry_after_seconds() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("30"),
        );
        let dur = parse_retry_after(&headers);
        assert_eq!(dur, Some(Duration::from_secs(30)));
    }

    #[test]
    fn test_parse_retry_after_ms() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::HeaderName::from_static("retry-after-ms"),
            reqwest::header::HeaderValue::from_static("500"),
        );
        let dur = parse_retry_after(&headers);
        assert_eq!(dur, Some(Duration::from_millis(500)));
    }

    #[test]
    fn test_parse_retry_after_missing() {
        let headers = reqwest::header::HeaderMap::new();
        assert!(parse_retry_after(&headers).is_none());
    }

    #[test]
    fn test_calculate_retry_timeout_uses_header() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("5"),
        );
        let dur = calculate_retry_timeout(Some(&headers), 0);
        assert_eq!(dur, Some(Duration::from_secs(5)));
    }

    #[test]
    fn test_calculate_retry_timeout_caps_at_60s() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("120"),
        );
        let dur = calculate_retry_timeout(Some(&headers), 0);
        assert!(dur.is_none());
    }

    #[test]
    fn test_calculate_retry_timeout_exponential_backoff() {
        let dur0 = calculate_retry_timeout(None, 0).unwrap();
        let dur1 = calculate_retry_timeout(None, 1).unwrap();
        let dur2 = calculate_retry_timeout(None, 2).unwrap();
        // Each should be larger than the previous
        assert!(dur1 > dur0);
        assert!(dur2 > dur1);
        // Should not exceed max
        assert!(dur2 <= MAX_RETRY_DELAY);
    }

    #[test]
    fn test_calculate_retry_timeout_jitter() {
        // Multiple calls should produce different values due to jitter
        let durs: Vec<_> = (0..10)
            .map(|_| calculate_retry_timeout(None, 0).unwrap())
            .collect();
        let unique = durs.iter().collect::<std::collections::HashSet<_>>();
        // At least 2 unique values (jitter range is 0.75x to 1.0x of base)
        assert!(unique.len() >= 2);
    }
}
