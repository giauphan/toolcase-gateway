use crate::config::Config;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MuseHarConfig {
    pub ws_url: Option<String>,
    pub base_url: Option<String>,
    pub access_token: Option<String>,
    pub notary_token: Option<String>,
    pub vm_id: Option<String>,
    pub cookie: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum HarExtractError {
    InvalidJson(String),
    NotHar,
    NoMuseEntries,
}

impl MuseHarConfig {
    pub fn found_fields(&self) -> usize {
        [
            self.ws_url.is_some(),
            self.base_url.is_some(),
            self.access_token.is_some(),
            self.notary_token.is_some(),
            self.vm_id.is_some(),
            self.cookie.is_some(),
        ]
        .iter()
        .filter(|v| **v)
        .count()
    }

    pub fn keep_fields(&self, current: &Config) -> Vec<&'static str> {
        let checks: [(&Option<String>, &'static str); 6] = [
            (&self.ws_url, "ws_url"),
            (&self.base_url, "base_url"),
            (&self.access_token, "access_token"),
            (&self.notary_token, "notary_token"),
            (&self.vm_id, "vm_id"),
            (&self.cookie, "cookie"),
        ];
        checks
            .iter()
            .filter(|(found, _)| found.is_none())
            .map(|(_, name)| *name)
            .filter(|name| current_field_present(current, name))
            .collect()
    }
}

pub(crate) fn current_field_present(config: &Config, name: &str) -> bool {
    match name {
        "ws_url" => !config.museai_ws_url.is_empty(),
        "base_url" => !config.museai_base_url.is_empty(),
        "access_token" => !config.museai_access_token.is_empty(),
        "notary_token" => !config.museai_notary_token.is_empty(),
        "vm_id" => !config.museai_vm_id.is_empty(),
        "cookie" => !config.museai_cookie.is_empty(),
        _ => false,
    }
}

pub(crate) struct WsEntry {
    pub ws_url: Option<String>,
    pub access_token: Option<String>,
    pub notary_token: Option<String>,
    pub vm_id: Option<String>,
}

pub(crate) struct RestEntry {
    pub base_url: String,
    pub cookie: Option<String>,
    pub ws_url: Option<String>,
    pub access_token: Option<String>,
    pub notary_token: Option<String>,
    pub vm_id: Option<String>,
}

pub fn mask_secret(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 8 {
        return "****".to_string();
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

pub fn safe_ws_url(value: &str) -> String {
    let base = value.split('?').next().unwrap_or(value);
    if value.contains('?') {
        format!("{base}?[credentials masked]")
    } else {
        base.to_string()
    }
}

#[derive(Debug, Clone)]
pub struct ApplyReport {
    pub applied: Vec<AppliedField>,
    pub config: Vec<AppliedField>,
    pub kept: Vec<&'static str>,
    pub env: EnvReport,
}

#[derive(Debug, Clone)]
pub struct AppliedField {
    pub key: &'static str,
    pub value: String,
    pub masked: bool,
}

#[derive(Debug, Clone)]
pub enum EnvReport {
    Written { path: String, keys: Vec<String> },
    SkippedNoFields,
    Failed { path: String, message: String },
}

impl serde::Serialize for ApplyReport {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("ApplyReport", 4)?;
        s.serialize_field("applied", &self.applied)?;
        s.serialize_field("config", &self.config)?;
        s.serialize_field("kept", &self.kept)?;
        s.serialize_field("env", &self.env)?;
        s.end()
    }
}

impl serde::Serialize for AppliedField {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("AppliedField", 3)?;
        s.serialize_field("key", self.key)?;
        s.serialize_field("value", &self.value)?;
        s.serialize_field("masked", &self.masked)?;
        s.end()
    }
}

impl serde::Serialize for EnvReport {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        match self {
            EnvReport::Written { path, keys } => {
                let mut s = serializer.serialize_struct("Env", 3)?;
                s.serialize_field("status", "written")?;
                s.serialize_field("path", path)?;
                s.serialize_field("keys", keys)?;
                s.end()
            }
            EnvReport::SkippedNoFields => {
                let mut s = serializer.serialize_struct("Env", 1)?;
                s.serialize_field("status", "skipped_no_fields")?;
                s.end()
            }
            EnvReport::Failed { path, message } => {
                let mut s = serializer.serialize_struct("Env", 3)?;
                s.serialize_field("status", "failed")?;
                s.serialize_field("path", path)?;
                s.serialize_field("message", message)?;
                s.end()
            }
        }
    }
}
