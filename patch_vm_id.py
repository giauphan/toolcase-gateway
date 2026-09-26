with open("src/museai.rs", "r") as f:
    text = f.read()

# 1. Update bootstrap_museai_config to extract the shared vm_id if empty
target1 = """    if !config.museai_vm_id.is_empty() && config.museai_vm_id != "." {
        body.insert(
            "vmName".to_string(),
            serde_json::Value::String(config.museai_vm_id.clone()),
        );
    }"""
new_target1 = """    let shared_vm = if config.museai_ws_url.is_empty() {
        if config.museai_base_url.contains("metaaivm.com") && !config.museai_base_url.contains("hatch.metaaivm.com") {
            config.museai_base_url.split("://").nth(1).unwrap_or("").split('.').next().unwrap_or("").to_string()
        } else {
            "".to_string()
        }
    } else {
        if config.museai_ws_url.contains("metaaivm.com") && !config.museai_ws_url.contains("hatch.metaaivm.com") {
            config.museai_ws_url.split("://").nth(1).unwrap_or("").split('.').next().unwrap_or("").to_string()
        } else {
            "".to_string()
        }
    };
    
    let active_vm_id = if !config.museai_vm_id.is_empty() && config.museai_vm_id != "." {
        config.museai_vm_id.clone()
    } else {
        shared_vm
    };
    
    if !active_vm_id.is_empty() {
        body.insert(
            "vmName".to_string(),
            serde_json::Value::String(active_vm_id),
        );
    }"""
text = text.replace(target1, new_target1)

# 2. Fix the node_id!
target2 = """    let node_id = config
        .museai_vm_id
        .strip_prefix("node:")
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());"""
new_target2 = """    let base_url = config.museai_base_url.as_str();
    let shared = base_url.contains("metaaivm.com") && !base_url.contains("hatch.metaaivm.com");
    let active_vm_id = if !config.museai_vm_id.is_empty() && config.museai_vm_id != "." {
        config.museai_vm_id.clone()
    } else if shared {
        base_url.split("://").nth(1).unwrap_or("").split('.').next().unwrap_or("").to_string()
    } else {
        uuid::Uuid::new_v4().to_string()
    };
    let node_id = active_vm_id.clone(); // use the identical vm_id for node_id!"""
text = text.replace(target2, new_target2)

with open("src/museai.rs", "w") as f:
    f.write(text)

print("vm_id logic patched")
