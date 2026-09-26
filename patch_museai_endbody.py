with open("src/museai.rs", "r") as f:
    text = f.read()

text = text.replace(
    'println!("[museai] RAW BodyChunk: {}", s);',
    'println!("[museai] RAW BodyChunk (end_body={}): {}", end_body, s);'
)

with open("src/museai.rs", "w") as f:
    f.write(text)
