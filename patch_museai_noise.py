with open("src/museai.rs", "r") as f:
    text = f.read()

# Remove unreachable pattern and fix warnings
text = text.replace(
    "            ServiceFrameKind::Reset { .. } => {\n                break;\n            }\n            _ => {}\n        }",
    "            ServiceFrameKind::Reset { .. } => {\n                break;\n            }\n        }"
)
text = text.replace(
    "let node_id = active_vm_id.clone();",
    "// let node_id = active_vm_id.clone();"
)
text = text.replace(
    "for i in 0..100 {",
    "for _ in 0..100 {"
)
text = text.replace(
    "ServiceFrameKind::Response { body, end_body, status, headers } => {",
    "ServiceFrameKind::Response { body, end_body, status, headers: _ } => {"
)

with open("src/museai.rs", "w") as f:
    f.write(text)
