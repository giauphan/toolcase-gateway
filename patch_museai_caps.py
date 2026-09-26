with open("src/museai.rs", "r") as f:
    text = f.read()

old_payload = """    let chat_payload = serde_json::json!({
        "items": [
            {
                "type": "text",
                "text": prompt
            }
        ]
    });"""

new_payload = """    let chat_payload = serde_json::json!({
        "items": [
            {
                "type": "text",
                "text": prompt
            }
        ],
        "node_id": active_vm_id.clone(),
        "capabilities": [
            "chat_cancel",
            "delta_stream"
        ]
    });"""

text = text.replace(old_payload, new_payload)

with open("src/museai.rs", "w") as f:
    f.write(text)
