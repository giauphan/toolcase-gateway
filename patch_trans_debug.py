with open("src/museai_transport.rs", "r") as f:
    text = f.read()

target = "let encrypted = self.read_binary()?;"
debug = """let encrypted = match self.read_binary() {
            Ok(bytes) => {
                println!("[museai_transport] Received encrypted frame of length: {}", bytes.len());
                bytes
            }
            Err(e) => {
                println!("[museai_transport] Failed to read encrypted frame: {e}");
                return Err(e);
            }
        };"""

text = text.replace(target, debug)

with open("src/museai_transport.rs", "w") as f:
    f.write(text)

print("Added transport debug logging")
