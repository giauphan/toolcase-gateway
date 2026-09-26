with open("src/museai.rs", "r") as f:
    text = f.read()

# Let's see how assistant content is structured in the chunks:
# In parse_assistant_content_from_json:
new_parse = """fn parse_assistant_content_from_json(val: &serde_json::Value) -> Option<String> {
    if let Some(content) = val.get("content").and_then(|c| c.as_str()) {
        if !content.is_empty() {
            return Some(content.to_string());
        }
    }
    if let Some(text) = val.get("text").and_then(|t| t.as_str()) {
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    if let Some(delta) = val.get("delta").and_then(|d| d.get("text")).and_then(|t| t.as_str()) {
        if !delta.is_empty() {
            return Some(delta.to_string());
        }
    }
    if let Some(message) = val.get("message") {
        if let Some(text) = parse_assistant_content_from_json(message) {
            return Some(text);
        }
    }
    if let Some(events) = val.get("events").and_then(|e| e.as_array()) {
        let mut combined = String::new();
        for ev in events {
            if let Some(text) = parse_assistant_content_from_json(ev) {
                combined.push_str(&text);
            }
        }
        if !combined.is_empty() {
            return Some(combined);
        }
    }
    if let Some(items) = val.get("items").and_then(|i| i.as_array()) {
        let mut combined = String::new();
        for item in items {
            if let Some(text) = parse_assistant_content_from_json(item) {
                combined.push_str(&text);
            }
        }
        if !combined.is_empty() {
            return Some(combined);
        }
    }
    None
}"""

old_parse = """fn parse_assistant_content_from_json(val: &serde_json::Value) -> Option<String> {
    if let Some(content) = val.get("content").and_then(|c| c.as_str()) {
        if !content.is_empty() {
            return Some(content.to_string());
        }
    }
    if let Some(text) = val.get("text").and_then(|t| t.as_str()) {
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    if let Some(events) = val.get("events").and_then(|e| e.as_array()) {
        let mut combined = String::new();
        for ev in events {
            if let Some(text) = parse_assistant_content_from_json(ev) {
                combined.push_str(&text);
            }
        }
        if !combined.is_empty() {
            return Some(combined);
        }
    }
    None
}"""

if old_parse in text:
    text = text.replace(old_parse, new_parse)
    with open("src/museai.rs", "w") as f:
        f.write(text)
    print("Patched parse_assistant_content_from_json")
else:
    print("Could not find old parse")
