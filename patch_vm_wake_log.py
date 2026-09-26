with open("src/museai.rs", "r") as f:
    text = f.read()

import re
text = re.sub(
    r"match wake_req\.send_json\(wake_body\) \{.*?\}",
    r"""let rsp = wake_req.send_json(wake_body);
        if let Ok(response) = rsp {
            if let Ok(json) = response.into_body().read_json::<serde_json::Value>() {
                println!("[museai] Note: Woke VM. Response: {}", json);
            }
        } else {
            println!("[museai] Note: Failed to wake VM.");
        }""",
    text,
    flags=re.DOTALL
)

with open("src/museai.rs", "w") as f:
    f.write(text)

print("Fixed VM wake response debug")
