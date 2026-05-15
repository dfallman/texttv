use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

use crate::cli::{Mode, Size, Source};

/// User configuration loaded from `~/.config/texttv/config.yaml`
/// (or `$XDG_CONFIG_HOME/texttv/config.yaml` if set).
///
/// Every field is optional. CLI arguments always win over the config file,
/// and the config file always wins over the built-in defaults.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub mode: Option<Mode>,
    pub size: Option<Size>,
    pub source: Option<Source>,
    pub no_color: Option<bool>,
}

impl Config {
    /// Load the config file, returning `Config::default()` if the file is
    /// missing. Parse/IO errors are propagated; main.rs prints them as
    /// warnings and falls back to defaults.
    pub fn load() -> Result<Self> {
        let Some(path) = Self::path() else {
            return Ok(Self::default());
        };
        if !path.exists() {
            return Ok(Self::default());
        }
        let body = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let cfg: Self = serde_yaml::from_str(&body)
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok(cfg)
    }

    /// Path to the config file, or `None` when neither `XDG_CONFIG_HOME`
    /// nor `HOME` is set (e.g. on a constrained CI runner).
    pub fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("texttv").join("config.yaml"))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::cli::{Mode, Size, Source};

    #[test]
    fn empty_yaml_yields_defaults() {
        let cfg: Config = serde_yaml::from_str("").unwrap_or_default();
        assert!(cfg.mode.is_none());
        assert!(cfg.size.is_none());
        assert!(cfg.source.is_none());
        assert!(cfg.no_color.is_none());
    }

    #[test]
    fn parses_all_fields() {
        let yaml = "\
mode: teletext
size: large
source: texttv-nu
no_color: true
";
        let cfg: Config = serde_yaml::from_str(yaml).expect("parse");
        assert_eq!(cfg.mode, Some(Mode::Teletext));
        assert_eq!(cfg.size, Some(Size::Large));
        assert_eq!(cfg.source, Some(Source::TexttvNu));
        assert_eq!(cfg.no_color, Some(true));
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let yaml = "bogus_key: 1\n";
        let err = serde_yaml::from_str::<Config>(yaml).unwrap_err();
        let msg = err.to_string().to_lowercase();
        assert!(msg.contains("unknown") || msg.contains("bogus"));
    }
}
