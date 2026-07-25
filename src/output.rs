use std::{
    cmp::Ordering,
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
    time::Duration,
};

use color_eyre::eyre::WrapErr as _;
use itertools::Itertools as _;

use crate::{
    HashMap,
    config::Config,
    ipdb,
    proxy::{Proxy, ProxyType},
    utils::is_docker,
};

#[must_use]
fn compare_timeout(a: &Proxy, b: &Proxy) -> Ordering {
    a.timeout.unwrap_or(Duration::MAX).cmp(&b.timeout.unwrap_or(Duration::MAX))
}

#[must_use]
fn compare_natural(a: &Proxy, b: &Proxy) -> Ordering {
    a.protocol
        .cmp(&b.protocol)
        .then_with(move || {
            match (a.host.parse::<Ipv4Addr>(), b.host.parse::<Ipv4Addr>()) {
                (Ok(ai), Ok(bi)) => ai.octets().cmp(&bi.octets()),
                (Ok(_), Err(_)) => Ordering::Less,
                (Err(_), Ok(_)) => Ordering::Greater,
                (Err(_), Err(_)) => a.host.cmp(&b.host),
            }
        })
        .then_with(move || a.port.cmp(&b.port))
}

#[derive(serde::Serialize)]
struct ProxyJson<'a> {
    protocol: ProxyType,
    username: Option<&'a str>,
    password: Option<&'a str>,
    host: &'a str,
    port: u16,
    timeout: Option<f64>,
    exit_ip: Option<&'a str>,
    asn: Option<maxminddb::geoip2::Asn<'a>>,
    geolocation: Option<maxminddb::geoip2::City<'a>>,
}

fn group_proxies<'a>(
    config: &Config,
    proxies: &'a [Proxy],
) -> HashMap<ProxyType, Vec<&'a Proxy>> {
    let mut groups: HashMap<_, _> =
        config.enabled_protocols().copied().map(|p| (p, Vec::new())).collect();
    for proxy in proxies {
        if let Some(proxies) = groups.get_mut(&proxy.protocol) {
            proxies.push(proxy);
        }
    }
    groups
}

#[expect(clippy::too_many_lines)]
pub async fn save_proxies(
    config: Arc<Config>,
    mut proxies: Vec<Proxy>,
) -> crate::Result<()> {
    if config.output.sort_by_speed {
        proxies.sort_unstable_by(compare_timeout);
    } else {
        proxies.sort_unstable_by(compare_natural);
    }

    if config.output.json.enabled {
        let (maybe_asn_db, maybe_geo_db) = tokio::try_join!(
            async {
                if config.output.json.include_asn {
                    ipdb::DbType::Asn.open_mmap().await.map(Some)
                } else {
                    Ok(None)
                }
            },
            async {
                if config.output.json.include_geolocation {
                    ipdb::DbType::Geo.open_mmap().await.map(Some)
                } else {
                    Ok(None)
                }
            }
        )?;

        let mut proxy_dicts = Vec::with_capacity(proxies.len());
        for proxy in &proxies {
            let exit_ip_addr: Option<IpAddr> =
                proxy.exit_ip.as_ref().and_then(|ip| ip.parse::<IpAddr>().ok());

            proxy_dicts.push(ProxyJson {
                protocol: proxy.protocol,
                username: proxy.username.as_deref(),
                password: proxy.password.as_deref(),
                host: &proxy.host,
                port: proxy.port,
                timeout: proxy
                    .timeout
                    .map(|d| (d.as_secs_f64() * 100.0).round() / 100.0_f64),
                exit_ip: proxy.exit_ip.as_deref(),
                asn: if let (Some(asn_db), Some(addr)) =
                    (maybe_asn_db.as_ref(), exit_ip_addr)
                {
                    asn_db
                        .lookup::<maxminddb::geoip2::Asn<'_>>(addr)
                        .wrap_err_with(move || {
                            format!("failed to lookup {addr} in ASN database")
                        })?
                } else {
                    None
                },
                geolocation: if let (Some(geo_db), Some(addr)) =
                    (maybe_geo_db.as_ref(), exit_ip_addr)
                {
                    geo_db
                        .lookup::<maxminddb::geoip2::City<'_>>(addr)
                        .wrap_err_with(move || {
                            format!(
                                "failed to lookup {addr} in geolocation \
                                 database"
                            )
                        })?
                } else {
                    None
                },
            });
        }

        let json_value = serde_json::to_value(&proxy_dicts)
            .wrap_err("failed to serialize proxies to json")?;
        for (path, pretty) in [
            (config.output.path.join("proxies.json"), false),
            (config.output.path.join("proxies_pretty.json"), true),
        ] {
            drop(tokio::fs::remove_file(&path).await);
            let json_data = if pretty {
                serde_json::to_vec_pretty(&json_value)
                    .wrap_err("failed to serialize proxies to pretty json")?
            } else {
                serde_json::to_vec(&json_value)
                    .wrap_err("failed to serialize proxies to json")?
            };
            tokio::fs::write(&path, json_data).await.wrap_err_with(
                move || {
                    format!("failed to write proxies to {}", path.display())
                },
            )?;
        }
    }

    if config.output.txt.enabled {
        let grouped_proxies = group_proxies(&config, &proxies);
        let directory_path = config.output.path.join("proxies");
        drop(tokio::fs::remove_dir_all(&directory_path).await);
        tokio::fs::create_dir_all(&directory_path).await.wrap_err_with(
            || {
                format!(
                    "failed to create directory: {}",
                    directory_path.display()
                )
            },
        )?;

        let text = create_proxy_list_str(proxies.iter(), true);
        tokio::fs::write(directory_path.join("all.txt"), &text)
            .await
            .wrap_err_with(|| {
                format!(
                    "failed to write proxies to {}",
                    directory_path.join("all.txt").display()
                )
            })?;

        for (proto, proxies) in grouped_proxies {
            let text = create_proxy_list_str(proxies, false);
            let mut file_path = directory_path.join(proto.as_str());
            file_path.set_extension("txt");
            tokio::fs::write(&file_path, &text).await.wrap_err_with(
                move || {
                    format!(
                        "failed to write proxies to {}",
                        file_path.display()
                    )
                },
            )?;
        }
    }

    let path = config
        .output
        .path
        .canonicalize()
        .unwrap_or_else(move |_| config.output.path.clone());
    if is_docker().await {
        tracing::info!(
            "Proxies have been saved to ./out ({} in container)",
            path.display()
        );
    } else {
        tracing::info!("Proxies have been saved to {}", path.display());
    }

    Ok(())
}

#[must_use]
fn create_proxy_list_str<'a, I>(proxies: I, include_protocol: bool) -> String
where
    I: IntoIterator<Item = &'a Proxy>,
{
    proxies
        .into_iter()
        .map(move |proxy| proxy.to_string(include_protocol))
        .join("\n")
}

#[cfg(test)]
#[expect(
    clippy::get_unwrap,
    clippy::inline_modules,
    clippy::redundant_test_prefix,
    clippy::unwrap_used
)]
mod tests {
    use foldhash::HashMapExt as _;

    use super::*;
    use crate::{config, proxy::ProxyType};

    fn make_proxy(
        protocol: ProxyType,
        host: &str,
        port: u16,
        timeout: Option<Duration>,
    ) -> Proxy {
        Proxy {
            protocol,
            host: host.into(),
            port,
            username: None,
            password: None,
            timeout,
            exit_ip: None,
        }
    }

    #[test]
    fn test_compare_timeout_some_first() {
        let a = make_proxy(
            ProxyType::Http,
            "1.2.3.4",
            80,
            Some(Duration::from_millis(100)),
        );
        let b = make_proxy(
            ProxyType::Http,
            "5.6.7.8",
            80,
            Some(Duration::from_millis(200)),
        );
        assert_eq!(compare_timeout(&a, &b), Ordering::Less);
    }

    #[test]
    fn test_compare_timeout_none_sorts_last() {
        let a = make_proxy(
            ProxyType::Http,
            "1.2.3.4",
            80,
            Some(Duration::from_millis(100)),
        );
        let b = make_proxy(ProxyType::Http, "5.6.7.8", 80, None);
        assert_eq!(compare_timeout(&a, &b), Ordering::Less);
    }

    #[test]
    fn test_compare_natural_ipv4() {
        let a = make_proxy(ProxyType::Http, "10.0.0.1", 80, None);
        let b = make_proxy(ProxyType::Http, "10.0.0.2", 80, None);
        assert_eq!(compare_natural(&a, &b), Ordering::Less);
    }

    #[test]
    fn test_compare_natural_ipv4_vs_domain() {
        let a = make_proxy(ProxyType::Http, "10.0.0.1", 80, None);
        let b = make_proxy(ProxyType::Http, "proxy.example.com", 80, None);
        assert_eq!(compare_natural(&a, &b), Ordering::Less);
    }

    #[test]
    fn test_compare_natural_same_host_diff_port() {
        let a = make_proxy(ProxyType::Http, "1.2.3.4", 80, None);
        let b = make_proxy(ProxyType::Http, "1.2.3.4", 8080, None);
        assert_eq!(compare_natural(&a, &b), Ordering::Less);
    }

    #[test]
    fn test_compare_natural_diff_protocol() {
        let a = make_proxy(ProxyType::Http, "1.2.3.4", 80, None);
        let b = make_proxy(ProxyType::Socks5, "1.2.3.4", 80, None);
        assert_eq!(compare_natural(&a, &b), Ordering::Less);
    }

    #[test]
    fn test_create_proxy_list_str_with_protocol() {
        let proxies = vec![
            make_proxy(ProxyType::Http, "1.2.3.4", 80, None),
            make_proxy(ProxyType::Socks5, "5.6.7.8", 1080, None),
        ];
        let result = create_proxy_list_str(&proxies, true);
        assert_eq!(result, "http://1.2.3.4:80\nsocks5://5.6.7.8:1080");
    }

    #[test]
    fn test_create_proxy_list_str_without_protocol() {
        let proxies = vec![make_proxy(ProxyType::Http, "1.2.3.4", 80, None)];
        let result = create_proxy_list_str(&proxies, false);
        assert_eq!(result, "1.2.3.4:80");
    }

    #[test]
    fn test_group_proxies() {
        let cfg = config::Config {
            debug: false,
            scraping: config::ScrapingConfig {
                sources: {
                    let mut m = HashMap::new();
                    m.insert(ProxyType::Http, Vec::new());
                    m.insert(ProxyType::Socks5, Vec::new());
                    m
                },
                max_proxies_per_source: 0,
                timeout: Duration::from_secs(30),
                connect_timeout: Duration::from_secs(5),
                proxy: None,
                user_agent: String::new(),
                rate_limit_ms: 0,
            },
            checking: config::CheckingConfig {
                check_url: None,
                check_schema: crate::raw_config::CheckSchema::None,
                max_concurrent_checks: 10,
                timeout: Duration::from_secs(30),
                connect_timeout: Duration::from_secs(5),
                user_agent: String::new(),
            },
            output: config::OutputConfig {
                path: std::path::PathBuf::from("./out"),
                sort_by_speed: false,
                txt: config::TxtOutputConfig { enabled: true },
                json: config::JsonOutputConfig {
                    enabled: false,
                    include_asn: false,
                    include_geolocation: false,
                },
            },
        };
        let proxies = vec![
            make_proxy(ProxyType::Http, "1.2.3.4", 80, None),
            make_proxy(ProxyType::Socks5, "5.6.7.8", 1080, None),
            make_proxy(ProxyType::Http, "9.10.11.12", 8080, None),
        ];
        let groups = group_proxies(&cfg, &proxies);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups.get(&ProxyType::Http).unwrap().len(), 2);
        assert_eq!(groups.get(&ProxyType::Socks5).unwrap().len(), 1);
    }
}
