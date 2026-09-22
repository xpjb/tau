//! User preferences are selectors, not invented provider model capabilities.
//! Only an unambiguous entry in the daemon's built-in /model catalog is sent.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use tau_protocol::{SlashCommand, SlashCommandArgument, SlashCommandSource};

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
            ensure!(
                slug.len() <= 240
                    && !slug.chars().any(|c| c.is_whitespace() || c.is_control())
                    && slug
                        .split_once('/')
                        .is_some_and(|(p, m)| !p.is_empty() && !m.is_empty()),
                "Use one provider/model slug per line (up to 240 bytes)"
            );
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
pub fn catalog(commands: &[SlashCommand]) -> &[SlashCommandArgument] {
    commands
        .iter()
        .find(|c| c.name == "model" && c.source == SlashCommandSource::Builtin)
        .map(|c| c.arguments.as_slice())
        .unwrap_or(&[])
}
pub fn resolve<'a>(selector: &str, commands: &'a [SlashCommand]) -> Option<&'a str> {
    let catalog = catalog(commands);
    if let Some(model) = catalog.iter().find(|m| m.value == selector) {
        return valid_slug(&model.value).then_some(model.value.as_str());
    }
    // Accommodate punctuation-only catalog spelling differences (v4.1/v4-1),
    // never provider changes, dated versions, billing suffixes or fuzzy prefixes.
    if !selector.is_ascii() {
        return None;
    }
    let (provider, id) = selector.split_once('/')?;
    let normalize = |value: &str| {
        value
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | ':'))
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let id = normalize(id);
    let mut matches = catalog.iter().filter(|m| {
        m.value
            .split_once('/')
            .is_some_and(|(p, value)| p == provider && normalize(value) == id)
    });
    let first = matches.next()?;
    (matches.next().is_none() && valid_slug(&first.value)).then_some(first.value.as_str())
}
fn valid_slug(slug: &str) -> bool {
    slug.len() <= 240
        && !slug.chars().any(|c| c.is_whitespace() || c.is_control())
        && slug
            .split_once('/')
            .is_some_and(|(p, m)| !p.is_empty() && !m.is_empty())
}
