with open("src/museai_transport.rs", "r") as f:
    code = f.read()

# Replace send_encrypted_service_request
old_impl = """        let chunk_id = (uuid::Uuid::new_v4().as_u128() & 0x7FFF_FFFF_FFFF_FFFF) as i64;
        
        // 1. Send ApplicationRequest (no body, end_body = false)
        let app_req =
            crate::museai_protocol::encode_application_request(verb, path, headers, &[], false);
        let frame_req = crate::museai_protocol::encode_service_frame_request(stream_id, &app_req);
        let svc_req = crate::museai_protocol::encode_service_request(service, &frame_req);
        
        let frames1 = crate::museai_protocol::encode_transport_frames(chunk_id, &svc_req)?;
        for frame in frames1 {
            let mut encrypted = vec![0u8; frame.len() + 64];
            let len = session
                .encrypt(&frame, &mut encrypted)
                .map_err(io::Error::other)?;
            println!("[museai_transport] Sending ApplicationRequest frame (len: {})", len);
            self.send_binary(&encrypted[..len])?;
        }

        // 2. Send BodyChunk (with body, end_body = true)
        let body_chunk = crate::museai_protocol::encode_body_chunk(body, true);
        let frame_chunk = crate::museai_protocol::encode_service_frame_body_chunk(stream_id, &body_chunk);
        let svc_chunk = crate::museai_protocol::encode_service_request(service, &frame_chunk);
        
        let frames2 = crate::museai_protocol::encode_transport_frames(chunk_id + 1, &svc_chunk)?;
        for frame in frames2 {
            let mut encrypted = vec![0u8; frame.len() + 64];
            let len = session
                .encrypt(&frame, &mut encrypted)
                .map_err(io::Error::other)?;
            println!("[museai_transport] Sending BodyChunk frame (len: {})", len);
            self.send_binary(&encrypted[..len])?;
        }

        Ok(())"""

new_impl = """        let chunk_id = (uuid::Uuid::new_v4().as_u128() & 0x7FFF_FFFF_FFFF_FFFF) as i64;
        
        // Send single ApplicationRequest with body and end_body = true
        let app_req =
            crate::museai_protocol::encode_application_request(verb, path, headers, body, true);
        let frame_req = crate::museai_protocol::encode_service_frame_request(stream_id, &app_req);
        let svc_req = crate::museai_protocol::encode_service_request(service, &frame_req);
        
        let frames = crate::museai_protocol::encode_transport_frames(chunk_id, &svc_req)?;
        for frame in frames {
            let mut encrypted = vec![0u8; frame.len() + 64];
            let len = session
                .encrypt(&frame, &mut encrypted)
                .map_err(io::Error::other)?;
            println!("[museai_transport] Sending ApplicationRequest frame with full body (len: {})", len);
            self.send_binary(&encrypted[..len])?;
        }

        Ok(())"""

if old_impl in code:
    code = code.replace(old_impl, new_impl)
    with open("src/museai_transport.rs", "w") as f:
        f.write(code)
    print("Patched send_encrypted_service_request to send full body in ApplicationRequest")
else:
    print("Could not find old implementation")
