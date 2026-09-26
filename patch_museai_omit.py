with open("src/museai.rs", "r") as f:
    text = f.read()

old_payload = """    let chat_payload = serde_json::json!({
        "items": [
            {
                "type": "text",
                "text": prompt
            }
        ],
        "node_id": node_id,
        "capabilities": [
            "chat_cancel",
            "delta_stream",
            "custom_reactions",
            "custom_reactions_facebook_thumbs_up_v1"
        ]
    });"""

new_payload = """    let chat_payload = serde_json::json!({
        "items": [
            {
                "type": "text",
                "text": prompt
            }
        ]
    });"""

text = text.replace(old_payload, new_payload)

with open("src/museai.rs", "w") as f:
    f.write(text)
