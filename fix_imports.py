with open("src/museai_transport.rs") as f:
    text = f.read()

text = "use crate::museai_noise::MuseNoiseSession;\nuse crate::museai_protocol::Header;\n" + text

with open("src/museai_transport.rs", "w") as f:
    f.write(text)

print("Imports fixed")
