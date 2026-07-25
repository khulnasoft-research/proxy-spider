use std::sync::LazyLock;

/// Regex for extracting proxy entries from arbitrary text.
///
/// Captures named groups: `protocol`, `username`, `password`, `host`, `port`.
pub static PROXY_REGEX: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
    let pattern = r"(?:^|[^0-9A-Za-z])(?:(?P<protocol>https?|socks[45]):\/\/)?(?:(?P<username>[0-9A-Za-z]{1,64}):(?P<password>[0-9A-Za-z]{1,64})@)?(?P<host>[A-Za-z][\-\.A-Za-z]{0,251}[A-Za-z]|[A-Za-z]|(?:[0-9]|[1-9][0-9]|1[0-9]{2}|2[0-4][0-9]|25[0-5])(?:\.(?:[0-9]|[1-9][0-9]|1[0-9]{2}|2[0-4][0-9]|25[0-5])){3}):(?P<port>[0-9]|[1-9][0-9]{1,3}|[1-5][0-9]{4}|6[0-4][0-9]{3}|65[0-4][0-9]{2}|655[0-2][0-9]|6553[0-5])(?=[^0-9A-Za-z]|$)";
    #[expect(clippy::unwrap_used)]
    fancy_regex::RegexBuilder::new(pattern)
        .backtrack_limit(usize::MAX)
        .build()
        .unwrap()
});

/// Regex for extracting an IPv4 address (optionally with port) from a string.
static IPV4_REGEX: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
    let pattern = r"^\s*(?P<host>(?:[0-9]|[1-9][0-9]|1[0-9]{2}|2[0-4][0-9]|25[0-5])(?:\.(?:[0-9]|[1-9][0-9]|1[0-9]{2}|2[0-4][0-9]|25[0-5])){3})(?::(?:[0-9]|[1-9][0-9]{1,3}|[1-5][0-9]{4}|6[0-4][0-9]{3}|65[0-4][0-9]{2}|655[0-2][0-9]|6553[0-5]))?\s*$";
    #[expect(clippy::unwrap_used)]
    fancy_regex::Regex::new(pattern).unwrap()
});

/// Extract an IPv4 address (without port) from a string, if one is found.
///
/// Returns `None` if no valid IPv4 address is present.
#[must_use]
pub fn parse_ipv4(s: &str) -> Option<String> {
    if let Ok(Some(captures)) = IPV4_REGEX.captures(s) {
        captures.name("host").map(|capture| capture.as_str().to_owned())
    } else {
        None
    }
}

#[cfg(test)]
#[expect(
    clippy::indexing_slicing,
    clippy::inline_modules,
    clippy::needless_collect,
    clippy::redundant_closure_for_method_calls,
    clippy::redundant_test_prefix,
    clippy::unwrap_used
)]
mod tests {
    use super::*;

    #[test]
    fn test_proxy_regex_http_without_protocol() {
        let captures: Vec<_> = PROXY_REGEX
            .captures_iter("192.168.1.1:8080")
            .filter_map(|c| c.ok())
            .collect();
        assert_eq!(captures.len(), 1);
        let cap = &captures[0];
        assert!(cap.name("protocol").is_none());
        assert_eq!(cap.name("host").unwrap().as_str(), "192.168.1.1");
        assert_eq!(cap.name("port").unwrap().as_str(), "8080");
    }

    #[test]
    fn test_proxy_regex_http_with_protocol() {
        let captures: Vec<_> = PROXY_REGEX
            .captures_iter("http://192.168.1.1:8080")
            .filter_map(|c| c.ok())
            .collect();
        assert_eq!(captures.len(), 1);
        let cap = &captures[0];
        assert_eq!(cap.name("protocol").unwrap().as_str(), "http");
        assert_eq!(cap.name("host").unwrap().as_str(), "192.168.1.1");
        assert_eq!(cap.name("port").unwrap().as_str(), "8080");
    }

    #[test]
    fn test_proxy_regex_https_with_protocol() {
        let captures: Vec<_> = PROXY_REGEX
            .captures_iter("https://192.168.1.1:8080")
            .filter_map(|c| c.ok())
            .collect();
        assert_eq!(captures.len(), 1);
        let cap = &captures[0];
        assert_eq!(cap.name("protocol").unwrap().as_str(), "https");
        assert_eq!(cap.name("host").unwrap().as_str(), "192.168.1.1");
        assert_eq!(cap.name("port").unwrap().as_str(), "8080");
    }

    #[test]
    fn test_proxy_regex_socks4() {
        let captures: Vec<_> = PROXY_REGEX
            .captures_iter("socks4://192.168.1.1:1080")
            .filter_map(|c| c.ok())
            .collect();
        assert_eq!(captures.len(), 1);
        let cap = &captures[0];
        assert_eq!(cap.name("protocol").unwrap().as_str(), "socks4");
        assert_eq!(cap.name("host").unwrap().as_str(), "192.168.1.1");
        assert_eq!(cap.name("port").unwrap().as_str(), "1080");
    }

    #[test]
    fn test_proxy_regex_socks5() {
        let captures: Vec<_> = PROXY_REGEX
            .captures_iter("socks5://192.168.1.1:1080")
            .filter_map(|c| c.ok())
            .collect();
        assert_eq!(captures.len(), 1);
        let cap = &captures[0];
        assert_eq!(cap.name("protocol").unwrap().as_str(), "socks5");
        assert_eq!(cap.name("host").unwrap().as_str(), "192.168.1.1");
        assert_eq!(cap.name("port").unwrap().as_str(), "1080");
    }

    #[test]
    fn test_proxy_regex_with_auth() {
        let captures: Vec<_> = PROXY_REGEX
            .captures_iter("http://user:pass@192.168.1.1:8080")
            .filter_map(|c| c.ok())
            .collect();
        assert_eq!(captures.len(), 1);
        let cap = &captures[0];
        assert_eq!(cap.name("protocol").unwrap().as_str(), "http");
        assert_eq!(cap.name("username").unwrap().as_str(), "user");
        assert_eq!(cap.name("password").unwrap().as_str(), "pass");
        assert_eq!(cap.name("host").unwrap().as_str(), "192.168.1.1");
        assert_eq!(cap.name("port").unwrap().as_str(), "8080");
    }

    #[test]
    fn test_proxy_regex_ipv4_like_host_domain() {
        let captures: Vec<_> = PROXY_REGEX
            .captures_iter("socks5://proxy.example.com:3128")
            .filter_map(|c| c.ok())
            .collect();
        assert_eq!(captures.len(), 1);
        let cap = &captures[0];
        assert_eq!(cap.name("protocol").unwrap().as_str(), "socks5");
        assert_eq!(cap.name("host").unwrap().as_str(), "proxy.example.com");
        assert_eq!(cap.name("port").unwrap().as_str(), "3128");
    }

    #[test]
    fn test_proxy_regex_multiple_proxies_in_text() {
        let text =
            "http://1.2.3.4:80\nhttps://5.6.7.8:443\nsocks5://9.10.11.12:1080";
        let captures: Vec<_> =
            PROXY_REGEX.captures_iter(text).filter_map(|c| c.ok()).collect();
        assert_eq!(captures.len(), 3);
    }

    #[test]
    fn test_proxy_regex_ignores_invalid_port() {
        let captures: Vec<_> = PROXY_REGEX
            .captures_iter("http://1.2.3.4:99999")
            .filter_map(|c| c.ok())
            .collect();
        assert_eq!(captures.len(), 0);
    }

    #[test]
    fn test_proxy_regex_ignores_invalid_ip() {
        let captures: Vec<_> = PROXY_REGEX
            .captures_iter("http://999.999.999.999:80")
            .filter_map(|c| c.ok())
            .collect();
        assert_eq!(captures.len(), 0);
    }

    #[test]
    fn test_parse_ipv4_valid() {
        assert_eq!(parse_ipv4("192.168.1.1"), Some("192.168.1.1".into()));
        assert_eq!(parse_ipv4("8.8.8.8"), Some("8.8.8.8".into()));
        assert_eq!(parse_ipv4("0.0.0.0"), Some("0.0.0.0".into()));
        assert_eq!(
            parse_ipv4("255.255.255.255"),
            Some("255.255.255.255".into())
        );
    }

    #[test]
    fn test_parse_ipv4_with_port() {
        assert_eq!(parse_ipv4("192.168.1.1:8080"), Some("192.168.1.1".into()));
    }

    #[test]
    fn test_parse_ipv4_with_whitespace() {
        assert_eq!(parse_ipv4("  192.168.1.1  "), Some("192.168.1.1".into()));
        assert_eq!(parse_ipv4("\t10.0.0.1\n"), Some("10.0.0.1".into()));
    }

    #[test]
    fn test_parse_ipv4_invalid() {
        assert_eq!(parse_ipv4("not an ip"), None);
        assert_eq!(parse_ipv4(""), None);
        assert_eq!(parse_ipv4("256.256.256.256"), None);
        assert_eq!(parse_ipv4("1.2.3.4.5"), None);
        assert_eq!(parse_ipv4("abc.def.ghi.jkl"), None);
    }

    #[test]
    fn test_parse_ipv4_json_response() {
        let json = r#"{"origin": "203.0.113.1"}"#;
        assert_eq!(parse_ipv4(json), None);
        let json = r#"{"origin": "203.0.113.1"}"#;
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        let origin = parsed["origin"].as_str().unwrap();
        assert_eq!(parse_ipv4(origin), Some("203.0.113.1".into()));
    }
}
