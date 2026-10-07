use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Locale-neutral user-facing text produced by the backend.
/// The frontend resolves `code` through the `backend` section of
/// `src/locales/*.json` and interpolates `params`.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Text {
    pub code: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
}

impl Text {
    pub fn new(code: &str) -> Self {
        Self {
            code: code.to_owned(),
            params: BTreeMap::new(),
        }
    }
    pub fn with<const N: usize>(code: &str, params: [(&str, String); N]) -> Self {
        Self {
            code: code.to_owned(),
            params: params
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value))
                .collect(),
        }
    }
}

impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.code)?;
        for (key, value) in &self.params {
            write!(f, " {key}={value}")?;
        }
        Ok(())
    }
}
