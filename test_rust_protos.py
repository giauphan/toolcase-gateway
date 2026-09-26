import re

with open("src/museai_protocol.rs", "r") as f:
    text = f.read()

funcs = [
    "encode_application_request",
    "encode_service_frame_request",
    "encode_service_request",
    "encode_transport_frames"
]

for func in funcs:
    print(f"=== {func} ===")
    m = re.search(f'pub\(crate\) fn {func}\([^)]+\)(?: -> [^{{]+)?{{', text)
    if m:
        start = m.end()
        end = start
        braces = 1
        while braces > 0 and end < len(text):
            if text[end] == '{': braces += 1
            if text[end] == '}': braces -= 1
            end += 1
        print(text[m.start():end])

