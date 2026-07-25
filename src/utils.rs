use itertools::Itertools as _;

/// Check whether the process is running inside a Docker container.
pub async fn is_docker() -> bool {
    #[cfg(target_os = "linux")]
    {
        static CACHE: tokio::sync::OnceCell<bool> =
            tokio::sync::OnceCell::const_new();

        *CACHE
            .get_or_init(async || {
                tokio::fs::try_exists("/.dockerenv")
                    .await
                    .unwrap_or(false)
            })
            .await
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Format an error chain as a human-readable string with arrows.
pub fn pretty_error(e: &crate::Error) -> String {
    e.chain().join(" \u{2192} ")
}

#[cfg(test)]
#[expect(clippy::inline_modules, clippy::redundant_test_prefix)]
mod tests {
    use super::*;

    #[test]
    fn test_pretty_error_single() {
        let e = color_eyre::eyre::eyre!("something went wrong");
        assert_eq!(pretty_error(&e), "something went wrong");
    }

    #[test]
    fn test_pretty_error_chain() {
        let e = color_eyre::eyre::eyre!("root cause")
            .wrap_err("intermediate")
            .wrap_err("top level");
        let formatted = pretty_error(&e);
        assert!(formatted.contains("top level"));
        assert!(formatted.contains("intermediate"));
        assert!(formatted.contains("root cause"));
        assert!(formatted.contains("\u{2192}"));
    }
}
