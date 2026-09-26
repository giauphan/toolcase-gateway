with open("src/museai.rs", "r") as f:
    text = f.read()

# Make it print the kind of EVERY frame received
text = text.replace(
    """        if i == 0 {
            println!("[museai] Received first encrypted frame: {:?}", frame.kind);
        }""",
    """        println!("[museai] Received encrypted frame: {:?}", frame.kind);"""
)

with open("src/museai.rs", "w") as f:
    f.write(text)
