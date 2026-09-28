//! Transient UI notices own their optional destination. Replacing the message
//! also replaces its action; display text is never used as a navigation key.
use std::ops::Deref;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadTarget {
    pub(crate) identity: String,
    pub(crate) lineage: String,
    pub(crate) session: String,
    pub(crate) entry: String,
}
impl DownloadTarget {
    pub(crate) fn matches_source(&self, identity: &str, lineage: Option<&str>) -> bool {
        self.identity == identity && self.lineage == lineage.unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    text: String,
    pub(crate) download: Option<DownloadTarget>,
}
impl Notice {
    pub(crate) fn download(text: String, target: DownloadTarget) -> Self {
        Self { text, download: Some(target) }
    }
}
impl From<String> for Notice {
    fn from(text: String) -> Self { Self { text, download: None } }
}
impl From<&str> for Notice {
    fn from(text: &str) -> Self { text.to_owned().into() }
}
impl Deref for Notice {
    type Target = str;
    fn deref(&self) -> &str { &self.text }
}
