with open("src/museai.rs", "r") as f:
    text = f.read()

send_subscribe_logic = """
    println!("[museai] Sending encrypted POST /chat/subscribe over Noise...");
    let sub_payload = serde_json::json!({
        "channel": "main"
    });
    let sub_bytes = serde_json::to_vec(&sub_payload).unwrap();
    socket.send_encrypted_service_request(
        &mut session,
        SERVICE_DAEMON,
        2, // stream_id = 2 for subscribe
        "POST",
        "/chat/subscribe",
        &headers,
        &sub_bytes,
    )?;

    // Give it a small delay
    std::thread::sleep(std::time::Duration::from_millis(50));

    println!("[museai] Sending encrypted POST /chat/stream over Noise...");
"""

if "[museai] Sending encrypted POST /chat/stream over Noise" in text and "subscribe" not in text:
    text = text.replace(
        'println!("[museai] Sending encrypted POST /chat/stream over Noise...");',
        send_subscribe_logic
    )
    with open("src/museai.rs", "w") as f:
        f.write(text)
    print("Patched to send /chat/subscribe")
else:
    print("Could not patch")
