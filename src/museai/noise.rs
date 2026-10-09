//! Noise Protocol Framework implementation for Muse.ai /v1/noise WebSocket streams.
#![allow(dead_code)]

use snow::params::NoiseParams;
use snow::Builder;
use std::error::Error;

/// Standard Meta AI / Noise protocol parameters used by Hatch/VM
pub const NOISE_PATTERN_XX: &str = "Noise_XX_25519_AESGCM_SHA256";
pub const NOISE_PATTERN_IK: &str = "Noise_IK_25519_AESGCM_SHA256";

pub(crate) fn encode_confidential_vm_message_three(notary_token: &str) -> Vec<u8> {
    let mut payload = Vec::new();

    // Field 1: notary_token (string)
    if !notary_token.is_empty() {
        // Tag 1 (wire type 2) = (1 << 3) | 2 = 10
        payload.push(10);
        let token_bytes = notary_token.as_bytes();
        // Write varint length
        let mut len = token_bytes.len() as u64;
        while len >= 0x80 {
            payload.push((len as u8) | 0x80);
            len >>= 7;
        }
        payload.push(len as u8);
        payload.extend_from_slice(token_bytes);
    }

    // Field 2: fresh_rv_key (bytes), unprovisioned branch
    // Tag 2 (wire type 2) = (2 << 3) | 2 = 18
    payload.push(18);
    let mut rv_key = [0u8; 32];
    let r1 = uuid::Uuid::new_v4();
    let r2 = uuid::Uuid::new_v4();
    rv_key[..16].copy_from_slice(r1.as_bytes());
    rv_key[16..].copy_from_slice(r2.as_bytes());

    // len is 32
    payload.push(32);
    payload.extend_from_slice(&rv_key);

    payload
}

pub(crate) fn get_primary_message_one_payload() -> Vec<u8> {
    let mut nonce = [0u8; 32];
    let r1 = uuid::Uuid::new_v4();
    let r2 = uuid::Uuid::new_v4();
    nonce[..16].copy_from_slice(r1.as_bytes());
    nonce[16..].copy_from_slice(r2.as_bytes());

    let mut payload = Vec::with_capacity(34);
    payload.push(0x0a);
    payload.push(0x20);
    payload.extend_from_slice(&nonce);
    payload
}

/// Encrypted stream wrapper for Muse.ai WebSocket session
pub struct MuseNoiseSession {
    handshake_state: Option<snow::HandshakeState>,
    transport_state: Option<snow::TransportState>,
}

impl MuseNoiseSession {
    /// Initialize a new Noise initiator session using the specified pattern and optional remote public key
    pub fn new_initiator(
        pattern: &str,
        remote_static_key: Option<&[u8]>,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let params: NoiseParams = pattern.parse()?;
        let mut builder = Builder::new(params);
        let keypair = builder.generate_keypair()?;
        builder = builder.local_private_key(&keypair.private);

        if let Some(r_key) = remote_static_key {
            builder = builder.remote_public_key(r_key);
        }

        let handshake = builder.build_initiator()?;

        Ok(Self {
            handshake_state: Some(handshake),
            transport_state: None,
        })
    }

    /// Write next handshake message to buffer
    pub fn write_handshake_message(
        &mut self,
        payload: &[u8],
        message: &mut [u8],
    ) -> Result<usize, Box<dyn Error + Send + Sync>> {
        let handshake = self
            .handshake_state
            .as_mut()
            .ok_or("Handshake not in progress")?;
        let len = handshake.write_message(payload, message)?;
        Ok(len)
    }

    /// Read incoming handshake message from buffer
    pub fn read_handshake_message(
        &mut self,
        message: &[u8],
        payload: &mut [u8],
    ) -> Result<usize, Box<dyn Error + Send + Sync>> {
        let handshake = self
            .handshake_state
            .as_mut()
            .ok_or("Handshake not in progress")?;
        let len = handshake.read_message(message, payload)?;
        Ok(len)
    }

    /// Check if handshake completed
    pub fn is_handshake_finished(&self) -> bool {
        self.handshake_state
            .as_ref()
            .map(|h| h.is_handshake_finished())
            .unwrap_or(false)
    }

    /// Transition from handshake to transport state
    #[allow(clippy::wrong_self_convention)]
    pub fn into_transport_mode(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let handshake = self
            .handshake_state
            .take()
            .ok_or("No handshake in progress to transition")?;
        let transport = handshake.into_transport_mode()?;
        self.transport_state = Some(transport);
        Ok(())
    }

    pub fn perform_client_handshake(
        &mut self,
        socket: &mut crate::museai::transport::MuseWebSocket,
        notary_token: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let mut message = vec![0u8; 65535];
        let mut payload = vec![0u8; 65535];

        let message_one_payload = get_primary_message_one_payload();
        println!(
            "[museai_noise] Handshake Phase: Sending message 1 (length: {})",
            message_one_payload.len()
        );
        let length = self.write_handshake_message(&message_one_payload, &mut message)?;
        socket.send_binary(&message[..length])?;

        println!("[museai_noise] Handshake Phase: Waiting for message 2");
        let message_two = socket.read_binary()?;
        println!(
            "[museai_noise] Handshake Phase: Received message 2 (length: {})",
            message_two.len()
        );
        let payload_length = self.read_handshake_message(&message_two, &mut payload)?;
        println!(
            "[museai_noise] Handshake Phase: Message 2 decrypted successfully (payload length: {})",
            payload_length
        );

        let msg3_payload = if payload_length > 0 {
            println!("[museai_noise] Handshake Phase: CVM challenge detected, generating message 3 credentials");
            encode_confidential_vm_message_three(notary_token)
        } else {
            println!("[museai_noise] Handshake Phase: No CVM challenge, empty message 3");
            vec![]
        };

        println!(
            "[museai_noise] Handshake Phase: Sending message 3 (length: {})",
            msg3_payload.len()
        );
        let length = self.write_handshake_message(&msg3_payload, &mut message)?;
        socket.send_binary(&message[..length])?;
        self.into_transport_mode()?;
        println!("[museai_noise] Handshake Phase: Transport split completed");
        Ok(())
    }

    /// Encrypt plaintext message into ciphertext buffer
    pub fn encrypt(
        &mut self,
        plaintext: &[u8],
        message: &mut [u8],
    ) -> Result<usize, Box<dyn Error + Send + Sync>> {
        let transport = self
            .transport_state
            .as_mut()
            .ok_or("Transport state not initialized")?;
        let len = transport.write_message(plaintext, message)?;
        Ok(len)
    }

    /// Decrypt ciphertext message into plaintext buffer
    pub fn decrypt(
        &mut self,
        message: &[u8],
        plaintext: &mut [u8],
    ) -> Result<usize, Box<dyn Error + Send + Sync>> {
        let transport = self
            .transport_state
            .as_mut()
            .ok_or("Transport state not initialized")?;
        let len = transport.read_message(message, plaintext)?;
        Ok(len)
    }
}
