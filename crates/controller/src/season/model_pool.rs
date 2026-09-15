//! Resolve a provider slice once, at draw time. Runs only consume frozen fleets.
use super::{FleetEntry, SeasonError, safe_id};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Selection {
    pub registry: PathBuf,
    pub provider: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    schema_version: u32,
    checked_at: String,
    models: Vec<Model>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Model {
    id: String,
    name: String,
    family: String,
    routes: BTreeMap<String, Route>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Route {
    model: String,
    reasoning_effort: String,
    status: Status,
    listed: bool,
    note: String,
    #[serde(default)]
    pricing: serde_json::Value,
    #[serde(default)]
    definition: Option<serde_json::Value>,
}

#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Status {
    Eligible,
    Disabled,
    Blocked,
}

fn binding(provider: &str) -> Result<(&'static str, &'static str), SeasonError> {
    match provider {
        "openrouter" => Ok(("claux", "")),
        "vercel" => Ok(("claux", "vercel/")),
        "opencode-go" => Ok(("opencode", "opencode-go/")),
        _ => Err(SeasonError::Invalid(format!(
            "unknown pool provider {provider:?}"
        ))),
    }
}

impl Selection {
    pub(super) fn resolve(&self, base: &Path) -> Result<Vec<FleetEntry>, SeasonError> {
        let (adapter, prefix) = binding(&self.provider)?;
        let path = base.join(&self.registry);
        let bytes = std::fs::read(&path).map_err(|source| SeasonError::Read { path, source })?;
        resolve_registry(&bytes, &self.provider, adapter, prefix)
    }
}

fn resolve_registry(
    bytes: &[u8],
    provider: &str,
    adapter: &str,
    prefix: &str,
) -> Result<Vec<FleetEntry>, SeasonError> {
    let registry: Registry = serde_json::from_slice(bytes)?;
    if registry.schema_version != 1 || registry.checked_at.is_empty() {
        return Err(SeasonError::Invalid(
            "invalid model registry version or check date".into(),
        ));
    }
    let mut ids = BTreeSet::new();
    let mut routes = BTreeSet::new();
    let mut fleet = Vec::new();
    for model in registry.models {
        if !safe_id(&model.id)
            || !ids.insert(model.id.clone())
            || model.name.is_empty()
            || model.family.is_empty()
        {
            return Err(SeasonError::Invalid(format!(
                "invalid or duplicate registry model {:?}",
                model.id
            )));
        }
        for (route_provider, route) in &model.routes {
            binding(route_provider)?;
            let safe_model = !route.model.is_empty()
                && route
                    .model
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._:/-".contains(&c));
            let safe_effort = matches!(
                route.reasoning_effort.as_str(),
                "default" | "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
            );
            if !safe_model
                || !safe_effort
                || !routes.insert((route_provider.clone(), route.model.clone()))
            {
                return Err(SeasonError::Invalid(format!(
                    "invalid or duplicate route for {}",
                    model.id
                )));
            }
            if route.status == Status::Eligible && route.listed {
                if matches!(model.family.as_str(), "meta" | "xai") {
                    return Err(SeasonError::Invalid(
                        "excluded model family cannot be eligible".into(),
                    ));
                }
                if route_provider == "opencode-go"
                    && route
                        .definition
                        .as_ref()
                        .and_then(|d| d.get("id"))
                        .and_then(serde_json::Value::as_str)
                        != Some(route.model.as_str())
                {
                    return Err(SeasonError::Invalid(format!(
                        "missing pinned OpenCode definition for {}",
                        model.id
                    )));
                }
                if route_provider == provider {
                    fleet.push(FleetEntry {
                        id: model.id.clone(),
                        model: format!("{prefix}{}", route.model),
                        adapter: adapter.into(),
                        reasoning_effort: route.reasoning_effort.clone(),
                    });
                }
            }
            // Descriptive metadata is validated by serde, not used as live pricing.
            let _ = (&route.note, &route.pricing);
        }
    }
    fleet.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(fleet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> serde_json::Value {
        json!({"schema_version":1,"checked_at":"2026-09-14","models":[
            {"id":"test","name":"Test","family":"vendor","routes":{
                "vercel":{"model":"vendor/test","reasoning_effort":"default","status":"eligible","listed":true,"note":"catalog only"},
                "openrouter":{"model":"vendor/test","reasoning_effort":"high","status":"blocked","listed":true,"note":"access error"}
            }}
        ]})
    }
    fn resolve(value: &serde_json::Value, provider: &str) -> Result<Vec<FleetEntry>, SeasonError> {
        let (adapter, prefix) = binding(provider)?;
        resolve_registry(
            &serde_json::to_vec(value).unwrap(),
            provider,
            adapter,
            prefix,
        )
    }
    #[test]
    fn provider_eligibility_and_delisting_are_independent() {
        let mut value = fixture();
        assert_eq!(
            resolve(&value, "vercel").unwrap()[0].model,
            "vercel/vendor/test"
        );
        assert!(resolve(&value, "openrouter").unwrap().is_empty());
        value["models"][0]["routes"]["vercel"]["listed"] = json!(false);
        assert!(resolve(&value, "vercel").unwrap().is_empty());
    }
    #[test]
    fn rejects_duplicates_unknown_providers_and_unsafe_ids() {
        let mut value = fixture();
        let model = value["models"][0].clone();
        value["models"].as_array_mut().unwrap().push(model);
        assert!(resolve(&value, "vercel").is_err());
        assert!(resolve(&fixture(), "unknown").is_err());
        let mut value = fixture();
        value["models"][0]["routes"]["vercel"]["model"] = json!("vendor/'unsafe");
        assert!(resolve(&value, "vercel").is_err());
    }
}
