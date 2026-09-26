with open("src/museai.rs", "r") as f:
    text = f.read()

text = text.replace(
    "ServiceFrameKind::Response { body, end_body, status, headers: _ } => {",
    "ServiceFrameKind::Response { body, end_body: _, status, headers: _ } => {"
)

with open("src/museai.rs", "w") as f:
    f.write(text)
