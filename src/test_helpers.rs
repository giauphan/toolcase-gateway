use std::io::Write;

pub fn save_live_owned_session(id: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/muse-live-owned-session.json");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(serde_json::json!({"session_id": id}).to_string().as_bytes())?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = id;
        Err(std::io::Error::other(
            "secure live-session storage unavailable on this platform",
        ))
    }
}

pub fn log_owned_record_schema(record: &serde_json::Value) {
    let payload = record.get("payload").unwrap_or(record);
    let event = record
        .get("event")
        .and_then(|value| value.as_str())
        .unwrap_or("<no-event>");
    let status = payload
        .get("status")
        .and_then(|value| value.as_str())
        .unwrap_or("<no-status>");
    let keys: Vec<&str> = record
        .as_object()
        .map(|map| map.keys().map(String::as_str).collect())
        .unwrap_or_default();
    println!("live owned record schema: event={event} status={status} keys={keys:?}");
}
