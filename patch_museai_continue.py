with open("src/museai.rs", "r") as f:
    text = f.read()

# Don't break on end_body! Keep reading for more pushed frames!
text = text.replace(
    """                if end_body && received_any_text {
                    break;
                }""",
    """                // if end_body && received_any_text { break; }"""
)

with open("src/museai.rs", "w") as f:
    f.write(text)
