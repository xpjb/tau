//! Favorites are exact provider/model IDs, independent of optional metadata.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use tau_protocol::SessionModel;

#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Preferences {
    // This configures tiles only. Any legacy `default` field is ignored by serde.
    pub slugs: Vec<String>,
}
impl Default for Preferences {
    fn default() -> Self {
        let slugs: Vec<String> = [
            "openai-codex/gpt-6-luna",
            "openai-codex/gpt-6-sol",
            "openai-codex/gpt-6-astra",
            "openrouter/deepseek/deepseek-v4.1-flash",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        Self { slugs }
    }
}
impl Preferences {
    pub fn text(&self) -> String {
        self.slugs.join(
            "
",
        )
    }
    pub fn parse(text: &str) -> Result<Self> {
        let mut preferences = Self { slugs: vec![] };
        for slug in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
            let _: SessionModel = slug.parse().map_err(anyhow::Error::msg)?;
            ensure!(
                !preferences.slugs.iter().any(|s| s == slug),
                "Duplicate model: {slug}"
            );
            preferences.slugs.push(slug.into());
            ensure!(
                preferences.slugs.len() <= 12,
                "Choose at most 12 quick models"
            );
        }
        Ok(preferences)
    }
}
