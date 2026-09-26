with open("src/museai.rs", "r") as f:
    text = f.read()

old_payload = """    let chat_payload = serde_json::json!({
        "items": [
            {
                "type": "text",
                "text": prompt
            }
        ],
        "node_id": _active_vm_id.clone(),
        "capabilities": [
            "chat_cancel",
            "delta_stream"
        ]
    });"""

new_payload = """    let thread_id = uuid::Uuid::new_v4().to_string();
    let chat_payload = serde_json::json!({
        "items": [
            {
                "type": "text",
                "text": prompt
            }
        ],
        "node_id": _active_vm_id.clone(),
        "session_id": thread_id,
        "capabilities": [
            "chat_cancel",
            "delta_stream"
        ]
    });"""

text = text.replace(old_payload, new_payload)

with open("src/museai.rs", "w") as f:
    f.write(text)
