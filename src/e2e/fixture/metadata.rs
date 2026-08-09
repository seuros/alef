use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateReturnForm {
    #[default]
    Dict,
    BareString,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureEnv {
    #[serde(default)]
    pub api_key_var: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupCall {
    pub call: String,
    #[serde(default)]
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FixtureDocs {
    pub topic: String,
    #[serde(default)]
    pub stem: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub side_effects: SideEffectClass,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectClass {
    #[default]
    None,
    Local,
    Network,
    ExternalMutation,
}
