//! Extensible MCP tools and resources contributed by registered platform modules.

use crate::api::mcp::protocol::{McpResource, McpTool};
use crate::config::ConfigStore;
use crate::gateway::modules::{ModuleDescriptor, ModuleKind, ModuleStatus, PersonalManifest};
use serde_json::{json, Value};

/// Internal target representation matching an advertised MCP tool to its owning module.
#[derive(Clone, Debug)]
pub(crate) struct ModuleToolTarget {
    pub module_id: String,
    pub sub_provider_id: Option<String>,
    pub action: String,
    pub is_synthesized: bool,
    pub tool: McpTool,
}

/// Discovers and aggregates MCP tools contributed by all currently enabled modules.
pub fn list_module_tools(store: &ConfigStore) -> Vec<McpTool> {
    let reg = store.module_registry();
    let guard = match reg.read() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };

    let mut result = Vec::new();
    for module in guard.list_modules() {
        if module.status != ModuleStatus::Enabled {
            continue;
        }
        for target in synthesize_targets_for_module(&module) {
            result.push(target.tool);
        }
    }
    result
}

/// Discovers and aggregates MCP resources contributed by all currently enabled modules.
pub fn list_module_resources(store: &ConfigStore) -> Vec<McpResource> {
    let reg = store.module_registry();
    let guard = match reg.read() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };

    let mut result = Vec::new();
    for module in guard.list_modules() {
        if module.status != ModuleStatus::Enabled {
            continue;
        }
        result.push(McpResource {
            uri: format!("gateway://modules/{}", module.id),
            name: format!("{} Module Descriptor", module.name),
            description: format!(
                "Runtime manifest, capabilities, and namespace bindings for module '{}'.",
                module.id
            ),
            mime_type: Some("application/json".to_string()),
        });
    }
    result
}

/// Synthesizes or converts tools declared on a module descriptor into advertised tool targets.
fn synthesize_targets_for_module(module: &ModuleDescriptor) -> Vec<ModuleToolTarget> {
    let mut targets = Vec::new();

    // 1. Explicit tools declared directly on the module manifest
    if let Some(ref declared) = module.tools {
        for def in declared {
            push_explicit_target(
                &mut targets,
                module,
                None,
                &format!("{}__", module.id),
                &module.name,
                def,
            );
        }
    }

    // 2. Explicit tools declared on enabled sub-providers
    if let Some(ref providers) = module.providers {
        for p in providers {
            if p.status != ModuleStatus::Enabled {
                continue;
            }
            if let Some(ref sub_tools) = p.tools {
                for def in sub_tools {
                    push_explicit_target(
                        &mut targets,
                        module,
                        Some(p.id.clone()),
                        &format!("{}__{}__", module.id, p.id),
                        &format!("{} / {}", module.name, p.name),
                        def,
                    );
                }
            }
        }
    }

    // 3. Capability-driven synthesis for Personal modules when no explicit tools are declared
    if targets.is_empty() && module.kind == ModuleKind::Personal {
        let caps = &module.capabilities;
        if caps.iter().any(|c| c == "chat_completions" || c == "chat") {
            targets.push(cap_target(
                module,
                "chat",
                "Send a prompt or conversation messages to generate completions.",
                json!({
                    "type": "object",
                    "properties": { "prompt": { "type": "string" }, "messages": { "type": "array" }, "model": { "type": "string" } },
                    "required": ["prompt"]
                }),
            ));
        }
        if caps.iter().any(|c| c == "models") {
            targets.push(cap_target(
                module,
                "list_models",
                "List available models supported by this provider.",
                json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            ));
        }
        if caps.iter().any(|c| c == "decisions") {
            targets.push(cap_target(
                module,
                "evaluate",
                "Evaluate structured probabilities and decisions from state facts.",
                json!({
                    "type": "object",
                    "properties": { "state": { "type": "object" }, "questions": { "type": "object" } },
                    "required": ["state", "questions"]
                }),
            ));
        }
        if caps.iter().any(|c| c == "image") {
            targets.push(cap_target(
                module,
                "generate_image",
                "Generate visual image assets from descriptive prompts.",
                json!({ "type": "object", "properties": { "prompt": { "type": "string" } }, "required": ["prompt"] }),
            ));
        }
    }

    targets
}

fn cap_target(
    module: &ModuleDescriptor,
    action: &str,
    desc: &str,
    schema: Value,
) -> ModuleToolTarget {
    ModuleToolTarget {
        module_id: module.id.clone(),
        sub_provider_id: None,
        action: action.to_string(),
        is_synthesized: true,
        tool: McpTool {
            name: format!("{}__{action}", module.id),
            description: format!("[{}] {desc}", module.name),
            input_schema: schema,
        },
    }
}

fn push_explicit_target(
    targets: &mut Vec<ModuleToolTarget>,
    module: &ModuleDescriptor,
    sub_provider_id: Option<String>,
    prefix: &str,
    label: &str,
    def: &crate::gateway::modules::ModuleToolDefinition,
) {
    let action = def
        .name
        .strip_prefix(prefix)
        .unwrap_or(&def.name)
        .to_string();
    let namespaced_name = format!("{prefix}{action}");
    if targets.iter().any(|t| t.tool.name == namespaced_name) {
        return;
    }
    targets.push(ModuleToolTarget {
        module_id: module.id.clone(),
        sub_provider_id,
        action,
        is_synthesized: false,
        tool: McpTool {
            name: namespaced_name,
            description: format!("[{label}] {}", def.description),
            input_schema: def.input_schema.clone(),
        },
    });
}

/// Executes an MCP tool call targeting a platform module.
pub fn call_module_tool(name: &str, params: &Value, store: &ConfigStore) -> Result<Value, String> {
    let reg = store.module_registry();
    let guard = reg.read().map_err(|e| e.to_string())?;

    let mut found = None;
    for module in guard.list_modules() {
        for target in synthesize_targets_for_module(&module) {
            if target.tool.name == name {
                found = Some((module, target));
                break;
            }
        }
        if found.is_some() {
            break;
        }
    }

    let (module, target) = found.ok_or_else(|| format!("Unknown or unadvertised tool '{name}'"))?;
    if module.status != ModuleStatus::Enabled {
        return Err(format!(
            "Module '{}' is currently disabled",
            target.module_id
        ));
    }
    let sp_info = if let Some(ref sp_id) = target.sub_provider_id {
        let sp = module
            .providers
            .as_ref()
            .and_then(|ps| ps.iter().find(|p| &p.id == sp_id))
            .ok_or_else(|| {
                format!(
                    "Sub-provider '{sp_id}' not found in module '{}'",
                    target.module_id
                )
            })?;
        if sp.status != ModuleStatus::Enabled {
            return Err(format!(
                "Sub-provider '{sp_id}' is currently disabled in module '{}'",
                target.module_id
            ));
        }
        Some((sp.id.clone(), sp.subpath.clone()))
    } else {
        None
    };
    let pm = guard.get_personal_manifest(&target.module_id).cloned();
    drop(guard);

    match module.kind {
        ModuleKind::Personal => {
            let manifest = pm.ok_or_else(|| {
                format!(
                    "Personal manifest missing for module '{}'",
                    target.module_id
                )
            })?;
            let sp_ref = sp_info.as_ref().map(|(i, s)| (i.as_str(), s.as_str()));
            execute_personal_tool(
                &manifest,
                sp_ref,
                &target.action,
                target.is_synthesized,
                params,
            )
        }
        ModuleKind::BuiltIn => execute_builtin_tool(&target.module_id, &target.action),
    }
}

/// Dispatches tool execution to an out-of-process personal module.
fn execute_personal_tool(
    manifest: &PersonalManifest,
    sub_provider: Option<(&str, &str)>,
    action: &str,
    is_synthesized: bool,
    params: &Value,
) -> Result<Value, String> {
    let (subpath, method, body_value) = if is_synthesized && action == "chat" {
        let prompt_str = params.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
        let messages = params
            .get("messages")
            .cloned()
            .unwrap_or_else(|| json!([{"role": "user", "content": prompt_str}]));
        (
            "/v1/chat/completions",
            "POST",
            json!({
                "model": params.get("model").and_then(|v| v.as_str()).unwrap_or("default"),
                "messages": messages,
                "stream": false
            }),
        )
    } else if is_synthesized && (action == "list_models" || action == "models") {
        ("/v1/models", "GET", json!({}))
    } else {
        (
            "/tools/call",
            "POST",
            json!({ "name": action, "tool": action, "arguments": params }),
        )
    };

    let mut effective_body = body_value;
    let mut effective_subpath = subpath.to_string();
    let sub_provider_id = if let Some((sp_id, sp_subpath)) = sub_provider {
        effective_subpath = format!("{}{subpath}", sp_subpath.trim_end_matches('/'));
        if let Some(obj) = effective_body.as_object_mut() {
            obj.insert("sub_provider".to_string(), json!(sp_id));
        }
        Some(sp_id)
    } else {
        None
    };

    let is_http = manifest.protocol == "local-http" || manifest.protocol == "http";
    let raw_resp = if is_http {
        let http_resp = crate::modules::personal::execute_http_payload(
            manifest,
            &effective_subpath,
            method,
            &effective_body.to_string(),
        )
        .map_err(|(code, msg)| {
            format!(
                "Module '{}' HTTP execution failed ({code}): {msg}",
                manifest.id
            )
        })?;
        json!({ "status": 200, "body": http_resp }).to_string()
    } else {
        let payload = json!({
            "method": method,
            "path": effective_subpath,
            "headers": [
                ["content-type", "application/json"],
                ["x-mcp-tool", action]
            ],
            "body": effective_body.to_string()
        });
        crate::modules::personal::execute_stdio_payload(manifest, sub_provider_id, &payload)
            .map_err(|(code, msg)| {
                format!("Module '{}' execution failed ({code}): {msg}", manifest.id)
            })?
    };

    normalize_module_output(&raw_resp)
}

/// Dispatches tool execution for built-in providers (fails closed when unadapted).
fn execute_builtin_tool(module_id: &str, action: &str) -> Result<Value, String> {
    Err(format!(
        "Built-in module '{module_id}' does not implement MCP tool '{action}'"
    ))
}

/// Normalizes output from child processes into the standard MCP tool content response.
fn normalize_module_output(raw_line: &str) -> Result<Value, String> {
    let parsed: Value = serde_json::from_str(raw_line)
        .map_err(|e| format!("Invalid JSON output from module: {e}"))?;

    // Check if the output already has standard MCP "content" array
    if let Some(content) = parsed.get("content").and_then(|v| v.as_array()) {
        return Ok(json!({ "content": content }));
    }

    // Check if output is standard HTTP envelope with status and body
    if let Some(status) = parsed.get("status").and_then(|v| v.as_u64()) {
        if status >= 400 {
            let err_text = parsed
                .get("body")
                .map(|b| b.to_string())
                .unwrap_or_else(|| format!("HTTP status {status}"));
            return Err(format!("Module returned error status {status}: {err_text}"));
        }
        if let Some(body) = parsed.get("body") {
            let inner = match body {
                Value::String(s) => {
                    serde_json::from_str::<Value>(s).unwrap_or_else(|_| body.clone())
                }
                other => other.clone(),
            };
            if let Some(content) = inner.get("content").and_then(|c| c.as_array()) {
                return Ok(json!({ "content": content }));
            }
            let text = extract_chat_completion_text(&inner).unwrap_or_else(|| match inner {
                Value::String(s) => s,
                other => serde_json::to_string_pretty(&other).unwrap_or_default(),
            });
            return Ok(json!({ "content": [{ "type": "text", "text": text }] }));
        }
    }

    // Check if output has "result" envelope
    if let Some(result) = parsed.get("result") {
        if let Some(content) = result.get("content") {
            return Ok(json!({ "content": content }));
        }
    }

    // Fallback: format entire JSON response into text content
    Ok(json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&parsed).unwrap_or_default()
        }]
    }))
}

fn extract_chat_completion_text(val: &Value) -> Option<String> {
    val.get("choices")?
        .as_array()?
        .first()?
        .get("message")?
        .get("content")?
        .as_str()
        .map(ToString::to_string)
}

/// Reads module-specific resources by URI (enforcing enabled status).
pub fn read_module_resource(uri: &str, store: &ConfigStore) -> Result<Value, String> {
    if let Some(id) = uri.strip_prefix("gateway://modules/") {
        let clean_id = id.trim_matches('/');
        let reg = store.module_registry();
        let guard = reg.read().map_err(|e| e.to_string())?;
        let module = guard
            .get_module(clean_id)
            .ok_or_else(|| format!("Module '{clean_id}' not found"))?;
        if module.status != ModuleStatus::Enabled {
            return Err(format!("Module '{clean_id}' is currently disabled"));
        }
        let personal = guard.get_personal_manifest(clean_id);
        let payload = json!({ "module": module, "personal_manifest": personal });
        return Ok(json!({
            "contents": [{
                "uri": uri,
                "mimeType": "application/json",
                "text": serde_json::to_string_pretty(&payload).unwrap_or_default()
            }]
        }));
    }
    Err(format!("Resource '{uri}' not found"))
}
