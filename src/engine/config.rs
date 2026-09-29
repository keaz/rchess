//! Engine configuration from environment variables (spec 5.7). Parsing never
//! fails: an invalid value falls back to its default and adds a warning. The Jev
//! endpoint is fixed ([`JEV_ENDPOINT`](super::JEV_ENDPOINT)); Laya's comes from `LAYA_URL`.

use std::fmt;
use std::time::Duration;

/// Jev model used when `JEV_MODEL` is unset.
pub const DEFAULT_MODEL: &str = "jev-latest";
/// Shortlist cap used when `JEV_MAX_OPTIONS` is unset or invalid.
pub const DEFAULT_MAX_OPTIONS: usize = 40;
/// Jev accepts at most 255 options in one `choice` question.
pub const MAX_CHOICE_OPTIONS: usize = 255;

use super::jev::{JEV_ENDPOINT, printable};

/// Laya model sent when `LAYA_MODEL` is unset.
pub const DEFAULT_LAYA_MODEL: &str = "laya";
/// Timeout for one Laya attempt: `laya-serve` often runs on a CPU.
pub const LAYA_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest part of a bad `LAYA_URL` quoted in its warning.
const URL_WARNING_CHARS: usize = 60;

/// Which System One model a computer player asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Provider {
    /// TypeSafe's hosted Jev.
    Jev,
    /// Laya, served by the person's own `laya-serve`.
    Laya,
}

impl Provider {
    /// `Jev` or `Laya`.
    pub const fn name(self) -> &'static str {
        match self {
            Provider::Jev => "Jev",
            Provider::Laya => "Laya",
        }
    }

    /// The variable that turns this provider on: `JEV_API_KEY` or `LAYA_URL`.
    pub const fn setting(self) -> &'static str {
        match self {
            Provider::Jev => "JEV_API_KEY",
            Provider::Laya => "LAYA_URL",
        }
    }
}

/// Settings for the computer player, normally read with [`EngineConfig::from_env`].
/// `Debug` output redacts the API key.
#[derive(Clone, PartialEq, Eq)]
pub struct EngineConfig {
    /// `None` means no key: Jev falls back to local search; Laya sends no
    /// `Authorization` header.
    pub api_key: Option<String>,
    /// The model this config asks.
    pub provider: Provider,
    /// The System One endpoint: [`JEV_ENDPOINT`] for Jev, `LAYA_URL` for Laya (empty
    /// when Laya is off).
    pub endpoint: String,
    /// Jev model ID sent with each request, e.g. `jev-latest`.
    pub model: String,
    /// Shortlist cap, always within 1..=255.
    pub max_options: usize,
    /// Leave `losing` moves off the shortlist when any other move exists.
    pub filter_losing: bool,
    /// Timeout for one HTTP attempt.
    pub timeout: Duration,
    /// A Jev pick scoring more than this many centipawns below the search best is vetoed.
    pub veto_margin_cp: i32,
    /// Human-readable notes about ignored or adjusted settings, for the TUI.
    pub warnings: Vec<String>,
    /// Record each Jev request and its responses in [`ComputerMove::exchange`]
    /// (debug mode). Off by default and never read from the environment here: the
    /// TUI sets it from `--debug` / `RCHESS_DEBUG`.
    ///
    /// [`ComputerMove::exchange`]: super::ComputerMove::exchange
    pub trace: bool,
}

impl Default for EngineConfig {
    fn default() -> EngineConfig {
        EngineConfig {
            api_key: None,
            provider: Provider::Jev,
            endpoint: JEV_ENDPOINT.to_string(),
            model: DEFAULT_MODEL.to_string(),
            max_options: DEFAULT_MAX_OPTIONS,
            filter_losing: true,
            timeout: Duration::from_secs(5),
            veto_margin_cp: 150,
            warnings: Vec::new(),
            trace: false,
        }
    }
}

impl fmt::Debug for EngineConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EngineConfig")
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("provider", &self.provider)
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("max_options", &self.max_options)
            .field("filter_losing", &self.filter_losing)
            .field("timeout", &self.timeout)
            .field("veto_margin_cp", &self.veto_margin_cp)
            .field("warnings", &self.warnings)
            .field("trace", &self.trace)
            .finish()
    }
}

impl EngineConfig {
    /// Reads the process environment.
    pub fn from_env() -> EngineConfig {
        EngineConfig::from_vars(|name| std::env::var(name).ok())
    }

    /// Builds a Jev config from a variable lookup; `from_env` passes the process environment.
    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> EngineConfig {
        let non_empty = non_empty(&get);
        let mut config = EngineConfig {
            api_key: non_empty("JEV_API_KEY").or_else(|| non_empty("TYPESAFE_API_KEY")),
            ..EngineConfig::default()
        };
        if let Some(model) = non_empty("JEV_MODEL") {
            config.model = model;
        }
        config.read_shortlist("JEV", &non_empty);
        config
    }

    /// Reads the process environment for Laya.
    pub fn laya_from_env() -> EngineConfig {
        EngineConfig::laya_from_vars(|name| std::env::var(name).ok())
    }

    /// Builds a Laya config from a variable lookup (`LAYA_URL`, `LAYA_API_KEY`,
    /// `LAYA_MODEL`, `LAYA_MAX_OPTIONS`, `LAYA_FILTER_LOSING`). Without a usable
    /// `LAYA_URL` the endpoint is empty and Laya is off.
    pub fn laya_from_vars(get: impl Fn(&str) -> Option<String>) -> EngineConfig {
        let non_empty = non_empty(&get);
        let mut config = EngineConfig {
            provider: Provider::Laya,
            endpoint: String::new(),
            api_key: non_empty("LAYA_API_KEY"),
            model: DEFAULT_LAYA_MODEL.to_string(),
            timeout: LAYA_TIMEOUT,
            ..EngineConfig::default()
        };
        if let Some(model) = non_empty("LAYA_MODEL") {
            config.model = model;
        }
        if let Some(url) = non_empty("LAYA_URL") {
            let lower = url.to_ascii_lowercase();
            if lower.starts_with("http://") || lower.starts_with("https://") {
                if config.api_key.is_some()
                    && let Some(host) = clear_text_host(&url)
                {
                    config.warnings.push(format!(
                        "LAYA_API_KEY is sent unencrypted to {host}; use https"
                    ));
                }
                config.endpoint = url;
            } else {
                config.warnings.push(format!(
                    "LAYA_URL={} is not an http(s) URL; Laya is off",
                    printable(&url, URL_WARNING_CHARS)
                ));
            }
        }
        config.read_shortlist("LAYA", &non_empty);
        config
    }

    /// True when this config can ask its model: Jev needs a key, Laya an endpoint.
    pub fn enabled(&self) -> bool {
        match self.provider {
            Provider::Jev => self.api_key.is_some(),
            Provider::Laya => !self.endpoint.is_empty(),
        }
    }

    /// Reads `<prefix>_MAX_OPTIONS` and `<prefix>_FILTER_LOSING`, adding a warning for
    /// each invalid value.
    fn read_shortlist(&mut self, prefix: &str, non_empty: &impl Fn(&str) -> Option<String>) {
        let name = format!("{prefix}_MAX_OPTIONS");
        if let Some(raw) = non_empty(&name) {
            match raw.parse::<usize>() {
                Ok(n) => {
                    self.max_options = n.clamp(1, MAX_CHOICE_OPTIONS);
                    if self.max_options != n {
                        self.warnings.push(format!(
                            "{name}={raw} is out of range; using {}",
                            self.max_options
                        ));
                    }
                }
                Err(_) => self.warnings.push(format!(
                    "{name}={raw} is not a number; using {DEFAULT_MAX_OPTIONS}"
                )),
            }
        }
        let name = format!("{prefix}_FILTER_LOSING");
        if let Some(raw) = non_empty(&name) {
            match raw.to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" => self.filter_losing = true,
                "false" | "0" | "no" => self.filter_losing = false,
                _ => self
                    .warnings
                    .push(format!("{name}={raw} is not true/false; using true")),
            }
        }
    }
}

/// `get` with values trimmed and empty values treated as unset.
fn non_empty(get: &impl Fn(&str) -> Option<String>) -> impl Fn(&str) -> Option<String> + '_ {
    move |name| {
        get(name)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    }
}

/// The host of a plain `http://` URL that is not this machine (`localhost`,
/// `127.0.0.1`, `[::1]`), lower-cased; `None` for `https://` or a local host.
fn clear_text_host(url: &str) -> Option<String> {
    let rest = url
        .get(..7)?
        .eq_ignore_ascii_case("http://")
        .then(|| &url[7..])?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit('@').next().unwrap_or("");
    let host = if host_port.starts_with('[') {
        host_port.split_inclusive(']').next().unwrap_or("")
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    let host = host.to_ascii_lowercase();
    (!matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]")).then_some(host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn config(vars: &[(&str, &str)]) -> EngineConfig {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        EngineConfig::from_vars(|name| map.get(name).cloned())
    }

    #[test]
    fn defaults_without_variables() {
        let c = config(&[]);
        assert_eq!(c, EngineConfig::default());
        assert_eq!(c.api_key, None);
        assert_eq!(c.model, "jev-latest");
        assert_eq!(c.max_options, 40);
        assert!(c.filter_losing);
        assert_eq!(c.timeout, Duration::from_secs(5));
        assert_eq!(c.veto_margin_cp, 150);
        assert_eq!(c.provider, Provider::Jev);
        assert_eq!(c.endpoint, crate::engine::JEV_ENDPOINT);
        assert!(!c.enabled(), "Jev needs a key");
    }

    #[test]
    fn key_falls_back_to_typesafe_variable() {
        assert_eq!(
            config(&[("JEV_API_KEY", "jev")]).api_key.as_deref(),
            Some("jev")
        );
        assert_eq!(
            config(&[("TYPESAFE_API_KEY", "ts")]).api_key.as_deref(),
            Some("ts")
        );
        assert_eq!(
            config(&[("JEV_API_KEY", "  "), ("TYPESAFE_API_KEY", "ts")])
                .api_key
                .as_deref(),
            Some("ts"),
            "a blank JEV_API_KEY is treated as unset"
        );
        assert_eq!(
            config(&[("JEV_API_KEY", ""), ("TYPESAFE_API_KEY", "")]).api_key,
            None
        );
    }

    #[test]
    fn model() {
        assert_eq!(config(&[("JEV_MODEL", "jev-1.13.0")]).model, "jev-1.13.0");
    }

    #[test]
    fn max_options_parsing() {
        assert_eq!(config(&[("JEV_MAX_OPTIONS", "12")]).max_options, 12);
        let high = config(&[("JEV_MAX_OPTIONS", "300")]);
        assert_eq!(high.max_options, 255);
        assert_eq!(
            high.warnings,
            vec!["JEV_MAX_OPTIONS=300 is out of range; using 255"]
        );
        assert_eq!(config(&[("JEV_MAX_OPTIONS", "0")]).max_options, 1);
        let bad = config(&[("JEV_MAX_OPTIONS", "lots")]);
        assert_eq!(bad.max_options, 40);
        assert_eq!(
            bad.warnings,
            vec!["JEV_MAX_OPTIONS=lots is not a number; using 40"]
        );
    }

    #[test]
    fn filter_losing_parsing() {
        for (raw, expected) in [
            ("false", false),
            ("0", false),
            ("No", false),
            ("TRUE", true),
            ("1", true),
            ("yes", true),
        ] {
            assert_eq!(
                config(&[("JEV_FILTER_LOSING", raw)]).filter_losing,
                expected,
                "{raw}"
            );
        }
        let bad = config(&[("JEV_FILTER_LOSING", "maybe")]);
        assert!(bad.filter_losing);
        assert_eq!(
            bad.warnings,
            vec!["JEV_FILTER_LOSING=maybe is not true/false; using true"]
        );
    }

    #[test]
    fn trace_is_off_and_never_read_from_the_environment() {
        assert!(!EngineConfig::default().trace);
        // The TUI sets `trace` from its own flag; no variable turns it on here.
        let c = config(&[
            ("RCHESS_DEBUG", "1"),
            ("JEV_TRACE", "1"),
            ("JEV_DEBUG", "1"),
        ]);
        assert!(!c.trace);
        assert!(c.warnings.is_empty(), "{:?}", c.warnings);
        let traced = EngineConfig {
            trace: true,
            ..EngineConfig::default()
        };
        assert!(format!("{traced:?}").contains("trace: true"));
    }

    #[test]
    fn debug_hides_the_key() {
        let c = config(&[("JEV_API_KEY", "secret-key-123")]);
        let text = format!("{c:?}");
        assert!(!text.contains("secret-key-123"), "{text}");
        assert!(text.contains("<redacted>"));
    }

    fn laya(vars: &[(&str, &str)]) -> EngineConfig {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        EngineConfig::laya_from_vars(|name| map.get(name).cloned())
    }

    const LOCAL_URL: &str = "http://127.0.0.1:8000/v1/systemone";

    #[test]
    fn provider_names_and_settings() {
        assert_eq!(Provider::Jev.name(), "Jev");
        assert_eq!(Provider::Laya.name(), "Laya");
        assert_eq!(Provider::Jev.setting(), "JEV_API_KEY");
        assert_eq!(Provider::Laya.setting(), "LAYA_URL");
    }

    #[test]
    fn jev_is_enabled_by_a_key() {
        assert!(config(&[("JEV_API_KEY", "k")]).enabled());
        // Laya variables never turn Jev on.
        assert!(!config(&[("LAYA_URL", LOCAL_URL)]).enabled());
    }

    #[test]
    fn laya_is_off_without_a_url() {
        let c = laya(&[]);
        assert_eq!(c.provider, Provider::Laya);
        assert_eq!(c.endpoint, "");
        assert!(!c.enabled());
        assert_eq!(c.api_key, None);
        assert_eq!(c.model, "laya");
        assert_eq!(c.max_options, 40);
        assert!(c.filter_losing);
        assert_eq!(c.timeout, Duration::from_secs(10));
        assert_eq!(c.veto_margin_cp, 150);
        assert!(c.warnings.is_empty(), "{:?}", c.warnings);
        // Jev's key does not reach Laya.
        assert_eq!(laya(&[("JEV_API_KEY", "jev")]).api_key, None);
        assert!(!laya(&[("LAYA_URL", "   ")]).enabled(), "blank is unset");
    }

    #[test]
    fn laya_reads_its_variables() {
        let c = laya(&[
            ("LAYA_URL", &format!("  {LOCAL_URL} ")),
            ("LAYA_API_KEY", "laya-key"),
            ("LAYA_MODEL", "laya-typed-decisions"),
            ("LAYA_MAX_OPTIONS", "24"),
            ("LAYA_FILTER_LOSING", "no"),
        ]);
        assert!(c.enabled());
        assert_eq!(c.endpoint, LOCAL_URL);
        assert_eq!(c.api_key.as_deref(), Some("laya-key"));
        assert_eq!(c.model, "laya-typed-decisions");
        assert_eq!(c.max_options, 24);
        assert!(!c.filter_losing);
        assert!(c.warnings.is_empty(), "{:?}", c.warnings);
        assert!(laya(&[("LAYA_URL", "HTTPS://laya.example/v1/systemone")]).enabled());
    }

    #[test]
    fn laya_shortlist_warnings_name_laya_variables() {
        let high = laya(&[("LAYA_URL", LOCAL_URL), ("LAYA_MAX_OPTIONS", "300")]);
        assert_eq!(high.max_options, 255);
        assert_eq!(
            high.warnings,
            vec!["LAYA_MAX_OPTIONS=300 is out of range; using 255"]
        );
        let bad = laya(&[
            ("LAYA_MAX_OPTIONS", "lots"),
            ("LAYA_FILTER_LOSING", "maybe"),
        ]);
        assert_eq!(bad.max_options, 40);
        assert_eq!(
            bad.warnings,
            vec![
                "LAYA_MAX_OPTIONS=lots is not a number; using 40",
                "LAYA_FILTER_LOSING=maybe is not true/false; using true",
            ]
        );
    }

    #[test]
    fn laya_rejects_a_url_that_is_not_http() {
        for raw in [
            "127.0.0.1:8000/v1/systemone",
            "ftp://host/x",
            "file:///tmp/sock",
        ] {
            let c = laya(&[("LAYA_URL", raw)]);
            assert!(!c.enabled(), "{raw}");
            assert_eq!(
                c.warnings,
                vec![format!("LAYA_URL={raw} is not an http(s) URL; Laya is off")]
            );
        }
        let long = format!("x{}\u{1b}[2J", "y".repeat(80));
        let c = laya(&[("LAYA_URL", &long)]);
        let warning = &c.warnings[0];
        assert!(!warning.contains('\u{1b}'), "{warning:?}");
        assert!(
            !warning.contains(&"y".repeat(60)),
            "cut to 60 characters: {warning}"
        );
    }

    #[test]
    fn laya_warns_when_the_key_travels_in_clear_text() {
        let warned = |url: &str| {
            laya(&[("LAYA_URL", url), ("LAYA_API_KEY", "k")])
                .warnings
                .iter()
                .any(|w| w.starts_with("LAYA_API_KEY is sent unencrypted"))
        };
        assert!(warned("http://gpu-box:8000/v1/systemone"));
        assert!(warned("HTTP://user@gpu-box.lan/v1/systemone"));
        for url in [
            "http://localhost:8000/v1/systemone",
            "http://127.0.0.1/v1/systemone",
            "http://[::1]:8000/v1/systemone",
            "http://LOCALHOST:8000/v1/systemone",
            "https://gpu-box/v1/systemone",
        ] {
            assert!(!warned(url), "{url}");
        }
        assert_eq!(
            laya(&[
                ("LAYA_URL", "http://gpu-box:8000/v1/systemone"),
                ("LAYA_API_KEY", "k")
            ])
            .warnings,
            vec!["LAYA_API_KEY is sent unencrypted to gpu-box; use https"]
        );
        // No key, no warning.
        assert!(
            laya(&[("LAYA_URL", "http://gpu-box/v1/systemone")])
                .warnings
                .is_empty()
        );
    }

    #[test]
    fn debug_hides_the_laya_key_and_shows_the_provider() {
        let c = laya(&[("LAYA_URL", LOCAL_URL), ("LAYA_API_KEY", "secret-laya-9")]);
        let text = format!("{c:?}");
        assert!(!text.contains("secret-laya-9"), "{text}");
        assert!(text.contains("Laya"), "{text}");
        assert!(text.contains(LOCAL_URL), "{text}");
    }
}
