//! Engine configuration from environment variables (spec 5.7). Parsing never
//! fails: an invalid value falls back to its default and adds a warning. The Jev
//! endpoint is fixed ([`JEV_ENDPOINT`](super::JEV_ENDPOINT)) and has no variable.

use std::fmt;
use std::time::Duration;

/// Jev model used when `JEV_MODEL` is unset.
pub const DEFAULT_MODEL: &str = "jev-latest";
/// Shortlist cap used when `JEV_MAX_OPTIONS` is unset or invalid.
pub const DEFAULT_MAX_OPTIONS: usize = 40;
/// Jev accepts at most 255 options in one `choice` question.
pub const MAX_CHOICE_OPTIONS: usize = 255;

/// Settings for the computer player, normally read with [`EngineConfig::from_env`].
/// `Debug` output redacts the API key.
#[derive(Clone, PartialEq, Eq)]
pub struct EngineConfig {
    /// `None` means no Jev: the player falls back to local search.
    pub api_key: Option<String>,
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
}

impl Default for EngineConfig {
    fn default() -> EngineConfig {
        EngineConfig {
            api_key: None,
            model: DEFAULT_MODEL.to_string(),
            max_options: DEFAULT_MAX_OPTIONS,
            filter_losing: true,
            timeout: Duration::from_secs(5),
            veto_margin_cp: 150,
            warnings: Vec::new(),
        }
    }
}

impl fmt::Debug for EngineConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EngineConfig")
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("model", &self.model)
            .field("max_options", &self.max_options)
            .field("filter_losing", &self.filter_losing)
            .field("timeout", &self.timeout)
            .field("veto_margin_cp", &self.veto_margin_cp)
            .field("warnings", &self.warnings)
            .finish()
    }
}

impl EngineConfig {
    /// Reads the process environment.
    pub fn from_env() -> EngineConfig {
        EngineConfig::from_vars(|name| std::env::var(name).ok())
    }

    /// Builds a config from a variable lookup; `from_env` passes the process environment.
    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> EngineConfig {
        let non_empty = |name: &str| {
            get(name)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let mut config = EngineConfig {
            api_key: non_empty("JEV_API_KEY").or_else(|| non_empty("TYPESAFE_API_KEY")),
            ..EngineConfig::default()
        };
        if let Some(model) = non_empty("JEV_MODEL") {
            config.model = model;
        }
        if let Some(raw) = non_empty("JEV_MAX_OPTIONS") {
            match raw.parse::<usize>() {
                Ok(n) => {
                    config.max_options = n.clamp(1, MAX_CHOICE_OPTIONS);
                    if config.max_options != n {
                        config.warnings.push(format!(
                            "JEV_MAX_OPTIONS={raw} is out of range; using {}",
                            config.max_options
                        ));
                    }
                }
                Err(_) => config.warnings.push(format!(
                    "JEV_MAX_OPTIONS={raw} is not a number; using {DEFAULT_MAX_OPTIONS}"
                )),
            }
        }
        if let Some(raw) = non_empty("JEV_FILTER_LOSING") {
            match raw.to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" => config.filter_losing = true,
                "false" | "0" | "no" => config.filter_losing = false,
                _ => config.warnings.push(format!(
                    "JEV_FILTER_LOSING={raw} is not true/false; using true"
                )),
            }
        }
        config
    }
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
        // The endpoint is fixed in `jev.rs`; the config carries no URL.
        let text = format!("{c:?}");
        assert!(!text.contains("url"), "{text}");
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
    fn debug_hides_the_key() {
        let c = config(&[("JEV_API_KEY", "secret-key-123")]);
        let text = format!("{c:?}");
        assert!(!text.contains("secret-key-123"), "{text}");
        assert!(text.contains("<redacted>"));
    }
}
