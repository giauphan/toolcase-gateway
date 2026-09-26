with open("src/museai.rs", "r") as f:
    text = f.read()

# Let's break if the response payload status is 'completed'
new_body_chunk = """            ServiceFrameKind::BodyChunk { data, end_body } => {
                let s = String::from_utf8_lossy(&data);
                println!("[museai] RAW BodyChunk (end_body={}): {}", end_body, s);
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&data) {
                    if let Some(text) = parse_assistant_content_from_json(&val) {
                        println!("[museai] Parsed Chunk: {}", text);
                        full_assistant_text.push_str(&text);
                        received_any_text = true;
                    }
                    
                    if let Some(status) = val.get("payload").and_then(|p| p.get("status")).and_then(|s| s.as_str()) {
                        if status == "completed" {
                            println!("[museai] Stream completed successfully!");
                            break;
                        }
                    }
                }
            }"""

import re
text = re.sub(
    r'ServiceFrameKind::BodyChunk.*?// if end_body && received_any_text \{ break; \}\n            \}',
    new_body_chunk,
    text,
    flags=re.DOTALL
)

with open("src/museai.rs", "w") as f:
    f.write(text)
