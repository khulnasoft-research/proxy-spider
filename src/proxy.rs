use std::{
    fmt::Write as _,
    hash::{Hash, Hasher},
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};

use color_eyre::eyre::{WrapErr as _, eyre};

use crate::{
    config::{Config, HttpbinResponse},
    parsers::parse_ipv4,
};

/// The type of proxy protocol.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize,
)]
#[serde(rename_all = "lowercase")]
pub enum ProxyType {
    Http,
    Socks4,
    Socks5,
}

impl FromStr for ProxyType {
    type Err = crate::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("http") || s.eq_ignore_ascii_case("https") {
            Ok(Self::Http)
        } else if s.eq_ignore_ascii_case("socks4") {
            Ok(Self::Socks4)
        } else if s.eq_ignore_ascii_case("socks5") {
            Ok(Self::Socks5)
        } else {
            Err(eyre!("failed to convert {s} to ProxyType"))
        }
    }
}

impl ProxyType {
    /// Return the string representation of this proxy type.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Socks4 => "socks4",
            Self::Socks5 => "socks5",
        }
    }
}

/// A parsed proxy entry with connection metadata.
///
/// Equality and hashing are based on `protocol`, `host`, `port`, `username`,
/// and `password` only (not `timeout` or `exit_ip`) to allow deduplication.
#[derive(Eq, Debug)]
pub struct Proxy {
    pub protocol: ProxyType,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub timeout: Option<Duration>,
    pub exit_ip: Option<String>,
}

impl TryFrom<&mut Proxy> for reqwest::Proxy {
    type Error = crate::Error;

    #[inline]
    fn try_from(value: &mut Proxy) -> Result<Self, Self::Error> {
        let proxy = Self::all(format!(
            "{}://{}:{}",
            value.protocol.as_str(),
            value.host,
            value.port
        ))
        .wrap_err("failed to create reqwest::Proxy")?;

        if let (Some(username), Some(password)) =
            (value.username.as_ref(), value.password.as_ref())
        {
            Ok(proxy.basic_auth(username, password))
        } else {
            Ok(proxy)
        }
    }
}

impl Proxy {
    /// Whether this proxy has been checked (has a timeout value).
    pub const fn is_checked(&self) -> bool {
        self.timeout.is_some()
    }

    /// Test this proxy by making a request through it to the configured check
    /// URL.
    ///
    /// On success, sets `timeout` to the response latency and attempts to
    /// extract the exit IP from the response body.
    pub async fn check<R>(
        &mut self,
        config: &Config,
        dns_resolver: Arc<R>,
    ) -> crate::Result<()>
    where
        R: reqwest::dns::Resolve + 'static,
    {
        if let Some(check_url) = &config.checking.check_url {
            let builder = reqwest::ClientBuilder::new()
                .user_agent(&config.checking.user_agent)
                .proxy(self.try_into()?)
                .timeout(config.checking.timeout)
                .connect_timeout(config.checking.connect_timeout)
                .pool_max_idle_per_host(0)
                .http1_only()
                .tcp_keepalive(None)
                .tcp_keepalive_interval(Duration::ZERO)
                .tcp_keepalive_retries(0)
                .dns_resolver(dns_resolver);
            #[cfg(any(
                target_os = "android",
                target_os = "fuchsia",
                target_os = "linux"
            ))]
            let builder = builder.tcp_user_timeout(None);
            let client = builder.build()?;
            let start = Instant::now();
            let response = client
                .get(check_url.clone())
                .send()
                .await?
                .error_for_status()?;
            drop(client);
            self.timeout = Some(start.elapsed());
            self.exit_ip = response.text().await.map_or(None, |text| {
                if let Ok(httpbin) =
                    serde_json::from_str::<HttpbinResponse>(&text)
                {
                    parse_ipv4(&httpbin.origin)
                } else {
                    parse_ipv4(&text)
                }
            });
        }
        Ok(())
    }

    /// Format this proxy as a string, optionally including the protocol prefix.
    ///
    /// Example with protocol: `"http://user:pass@1.2.3.4:8080"`
    /// Example without:      `"user:pass@1.2.3.4:8080"`.
    pub fn to_string(&self, include_protocol: bool) -> String {
        let mut s = String::new();

        if include_protocol {
            s.push_str(self.protocol.as_str());
            s.push_str("://");
        }

        if let (Some(username), Some(password)) =
            (&self.username, &self.password)
        {
            s.push_str(username);
            s.push(':');
            s.push_str(password);
            s.push('@');
        }

        s.push_str(&self.host);
        s.push(':');
        #[expect(clippy::unwrap_used)]
        write!(s, "{}", self.port).unwrap();

        s
    }
}

#[expect(clippy::missing_trait_methods)]
impl PartialEq for Proxy {
    fn eq(&self, other: &Self) -> bool {
        self.protocol == other.protocol
            && self.host == other.host
            && self.port == other.port
            && self.username == other.username
            && self.password == other.password
    }
}

#[expect(clippy::missing_trait_methods)]
impl Hash for Proxy {
    fn hash<H>(&self, state: &mut H)
    where
        H: Hasher,
    {
        self.protocol.hash(state);
        self.host.hash(state);
        self.port.hash(state);
        self.username.hash(state);
        self.password.hash(state);
    }
}

#[cfg(test)]
#[expect(
    clippy::assertions_on_result_states,
    clippy::inline_modules,
    clippy::redundant_test_prefix,
    clippy::unwrap_used
)]
mod tests {
    use super::*;

    #[test]
    fn test_proxy_type_from_str() {
        assert_eq!("http".parse::<ProxyType>().unwrap(), ProxyType::Http);
        assert_eq!("https".parse::<ProxyType>().unwrap(), ProxyType::Http);
        assert_eq!("HTTP".parse::<ProxyType>().unwrap(), ProxyType::Http);
        assert_eq!("socks4".parse::<ProxyType>().unwrap(), ProxyType::Socks4);
        assert_eq!("SOCKS4".parse::<ProxyType>().unwrap(), ProxyType::Socks4);
        assert_eq!("socks5".parse::<ProxyType>().unwrap(), ProxyType::Socks5);
        assert!("invalid".parse::<ProxyType>().is_err());
    }

    #[test]
    fn test_proxy_type_as_str() {
        assert_eq!(ProxyType::Http.as_str(), "http");
        assert_eq!(ProxyType::Socks4.as_str(), "socks4");
        assert_eq!(ProxyType::Socks5.as_str(), "socks5");
    }

    #[test]
    fn test_proxy_to_string_without_protocol() {
        let p = Proxy {
            protocol: ProxyType::Http,
            host: "1.2.3.4".into(),
            port: 8080,
            username: None,
            password: None,
            timeout: None,
            exit_ip: None,
        };
        assert_eq!(p.to_string(false), "1.2.3.4:8080");
    }

    #[test]
    fn test_proxy_to_string_with_protocol() {
        let p = Proxy {
            protocol: ProxyType::Socks5,
            host: "proxy.example.com".into(),
            port: 1080,
            username: None,
            password: None,
            timeout: None,
            exit_ip: None,
        };
        assert_eq!(p.to_string(true), "socks5://proxy.example.com:1080");
    }

    #[test]
    fn test_proxy_to_string_with_auth() {
        let p = Proxy {
            protocol: ProxyType::Http,
            host: "1.2.3.4".into(),
            port: 3128,
            username: Some("user".into()),
            password: Some("pass".into()),
            timeout: None,
            exit_ip: None,
        };
        assert_eq!(p.to_string(true), "http://user:pass@1.2.3.4:3128");
    }

    #[test]
    fn test_proxy_partial_eq_same() {
        let a = Proxy {
            protocol: ProxyType::Http,
            host: "1.2.3.4".into(),
            port: 8080,
            username: None,
            password: None,
            timeout: None,
            exit_ip: None,
        };
        let b = Proxy {
            protocol: ProxyType::Http,
            host: "1.2.3.4".into(),
            port: 8080,
            username: None,
            password: None,
            timeout: Some(Duration::from_secs(1)),
            exit_ip: Some("5.6.7.8".into()),
        };
        assert_eq!(a, b);
    }

    #[test]
    fn test_proxy_partial_eq_different() {
        let a = Proxy {
            protocol: ProxyType::Http,
            host: "1.2.3.4".into(),
            port: 8080,
            username: None,
            password: None,
            timeout: None,
            exit_ip: None,
        };
        let b = Proxy {
            protocol: ProxyType::Socks5,
            host: "1.2.3.4".into(),
            port: 8080,
            username: None,
            password: None,
            timeout: None,
            exit_ip: None,
        };
        assert_ne!(a, b);
    }

    #[test]
    fn test_proxy_is_checked() {
        let mut p = Proxy {
            protocol: ProxyType::Http,
            host: "1.2.3.4".into(),
            port: 8080,
            username: None,
            password: None,
            timeout: None,
            exit_ip: None,
        };
        assert!(!p.is_checked());
        p.timeout = Some(Duration::from_secs(1));
        assert!(p.is_checked());
    }

    #[test]
    fn test_proxy_dedup_via_hashset() {
        use foldhash::HashSetExt as _;
        let mut set = crate::HashSet::new();
        set.insert(Proxy {
            protocol: ProxyType::Http,
            host: "1.2.3.4".into(),
            port: 8080,
            username: None,
            password: None,
            timeout: None,
            exit_ip: None,
        });
        set.insert(Proxy {
            protocol: ProxyType::Http,
            host: "1.2.3.4".into(),
            port: 8080,
            username: None,
            password: None,
            timeout: Some(Duration::from_secs(2)),
            exit_ip: Some("5.6.7.8".into()),
        });
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn test_proxy_dedup_different_port() {
        use foldhash::HashSetExt as _;
        let mut set = crate::HashSet::new();
        set.insert(Proxy {
            protocol: ProxyType::Http,
            host: "1.2.3.4".into(),
            port: 8080,
            username: None,
            password: None,
            timeout: None,
            exit_ip: None,
        });
        set.insert(Proxy {
            protocol: ProxyType::Http,
            host: "1.2.3.4".into(),
            port: 8081,
            username: None,
            password: None,
            timeout: None,
            exit_ip: None,
        });
        assert_eq!(set.len(), 2);
    }
}
