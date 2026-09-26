with open("src/museai.rs", "r") as f:
    text = f.read()

new_logic = """        match frame.kind {
            ServiceFrameKind::Response { body, end_body, status, headers } => {
                println!("[museai] Response status: {}", status);
                if !body.is_empty() {
                    let s = String::from_utf8_lossy(&body);
                    println!("[museai] RAW Response Body: {}", s);
                    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&body) {
                        if let Some(text) = parse_assistant_content_from_json(&val) {
                            println!("[museai] Parsed Chunk: {}", text);
                            full_assistant_text.push_str(&text);
                            received_any_text = true;
                        }
                    }
                }
                if end_body {
                    break;
                }
            }
            ServiceFrameKind::BodyChunk { data, end_body } => {
                let s = String::from_utf8_lossy(&data);
                println!("[museai] RAW BodyChunk: {}", s);
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&data) {
                    if let Some(text) = parse_assistant_content_from_json(&val) {
                        println!("[museai] Parsed Chunk: {}", text);
                        full_assistant_text.push_str(&text);
                        received_any_text = true;
                    }
                }
                if end_body {
                    break;
                }
            }
            ServiceFrameKind::Reset { .. } => {
                break;
            }
            _ => {}
        }"""

old_logic = """        match frame.kind {
            ServiceFrameKind::Response { body, end_body, .. } => {
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&body) {
                    if let Some(text) = parse_assistant_content_from_json(&val) {
                        println!("[museai] Chunk: {}", text);
                        full_assistant_text.push_str(&text);
                        received_any_text = true;
                    }
                }
                if end_body && received_any_text {
                    break;
                }
            }
            ServiceFrameKind::BodyChunk { data, end_body } => {
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&data) {
                    if let Some(text) = parse_assistant_content_from_json(&val) {
                        println!("[museai] Chunk: {}", text);
                        full_assistant_text.push_str(&text);
                        received_any_text = true;
                    }
                }
                if end_body && received_any_text {
                    break;
                }
            }
            ServiceFrameKind::Reset { .. } => {
                break;
            }
            _ => {}
        }"""

if old_logic in text:
    text = text.replace(old_logic, new_logic)
    with open("src/museai.rs", "w") as f:
        f.write(text)
    print("Patched frame reading logic to break on end_body and print raw strings")
else:
    print("Could not find old logic")
