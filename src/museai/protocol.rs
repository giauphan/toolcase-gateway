#![allow(dead_code)]

use std::io;

pub const SERVICE_DAEMON: u64 = 0;
pub const MAX_TRANSPORT_CHUNK_BYTES: usize = 65_489;

#[derive(Debug, PartialEq)]
pub struct Header {
    pub key: String,
    pub value: String,
}

#[derive(Debug, PartialEq)]
pub enum ServiceFrameKind {
    Response {
        status: u32,
        headers: Vec<Header>,
        body: Vec<u8>,
        end_body: bool,
    },
    BodyChunk {
        data: Vec<u8>,
        end_body: bool,
    },
    Reset {
        code: u32,
        reason: String,
    },
}

#[derive(Debug, PartialEq)]
pub struct ServiceFrame {
    pub stream_id: u64,
    pub kind: ServiceFrameKind,
}

#[derive(Debug, PartialEq)]
pub struct NoiseTransportFrame {
    pub chunk_id: i64,
    pub chunk_index: u32,
    pub total_chunks: u32,
    pub payload: Vec<u8>,
}

pub fn encode_application_request(
    verb: &str,
    path: &str,
    headers: &[Header],
    body: &[u8],
    end_body: bool,
) -> Vec<u8> {
    let mut output = Vec::new();
    write_bytes_field(1, verb.as_bytes(), &mut output);
    write_bytes_field(2, path.as_bytes(), &mut output);
    for header in headers {
        let mut encoded = Vec::new();
        write_bytes_field(1, header.key.as_bytes(), &mut encoded);
        write_bytes_field(2, header.value.as_bytes(), &mut encoded);
        write_bytes_field(3, &encoded, &mut output);
    }
    write_bytes_field(4, body, &mut output);
    if end_body {
        write_varint_field(5, 1, &mut output);
    }
    output
}

pub fn encode_body_chunk(data: &[u8], end_body: bool) -> Vec<u8> {
    let mut output = Vec::new();
    if !data.is_empty() {
        write_bytes_field(1, data, &mut output);
    }
    if end_body {
        write_varint_field(2, 1, &mut output);
    }
    output
}

pub fn encode_service_frame_body_chunk(stream_id: u64, chunk: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    write_varint_field(1, stream_id, &mut output);
    write_bytes_field(4, chunk, &mut output);
    output
}

pub fn encode_service_frame_request(stream_id: u64, request: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    write_varint_field(1, stream_id, &mut output);
    write_bytes_field(2, request, &mut output);
    output
}

pub fn encode_service_request(service: u64, service_frame: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    if service != SERVICE_DAEMON {
        write_varint_field(1, service, &mut output);
    }
    write_bytes_field(2, service_frame, &mut output);
    output
}

pub fn encode_transport_frames(chunk_id: i64, payload: &[u8]) -> io::Result<Vec<Vec<u8>>> {
    let chunks = payload
        .chunks(MAX_TRANSPORT_CHUNK_BYTES)
        .collect::<Vec<_>>();
    let total_chunks = u32::try_from(chunks.len().max(1)).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Muse payload has too many chunks",
        )
    })?;
    let mut frames = Vec::with_capacity(total_chunks as usize);
    if chunks.is_empty() {
        frames.push(encode_transport_frame(chunk_id, 0, 1, &[]));
        return Ok(frames);
    }
    for (index, chunk) in chunks.into_iter().enumerate() {
        frames.push(encode_transport_frame(
            chunk_id,
            index as u32,
            total_chunks,
            chunk,
        ));
    }
    Ok(frames)
}

pub fn decode_transport_frame(input: &[u8]) -> io::Result<NoiseTransportFrame> {
    let mut reader = ProtoReader::new(input);
    let mut frame = NoiseTransportFrame {
        chunk_id: 0,
        chunk_index: 0,
        total_chunks: 0,
        payload: Vec::new(),
    };
    while let Some((field, wire)) = reader.field()? {
        match (field, wire) {
            (1, 0) => frame.chunk_id = reader.varint()? as i64,
            (2, 0) => frame.chunk_index = u32_from_varint(reader.varint()?)?,
            (3, 0) => frame.total_chunks = u32_from_varint(reader.varint()?)?,
            (4, 2) => frame.payload = reader.bytes()?.to_vec(),
            _ => reader.skip(wire)?,
        }
    }
    if frame.total_chunks == 0 || frame.chunk_index >= frame.total_chunks {
        return Err(invalid_data("invalid Muse transport chunk metadata"));
    }
    Ok(frame)
}

pub fn decode_service_response(input: &[u8]) -> io::Result<Vec<u8>> {
    let mut reader = ProtoReader::new(input);
    let mut payload = None;
    while let Some((field, wire)) = reader.field()? {
        match (field, wire) {
            (1, 2) => payload = Some(reader.bytes()?.to_vec()),
            _ => reader.skip(wire)?,
        }
    }
    payload.ok_or_else(|| invalid_data("Muse ServiceResponse has no payload"))
}

pub fn decode_service_frame(input: &[u8]) -> io::Result<ServiceFrame> {
    let mut reader = ProtoReader::new(input);
    let mut stream_id = 0;
    let mut kind = None;
    while let Some((field, wire)) = reader.field()? {
        match (field, wire) {
            (1, 0) => stream_id = reader.varint()?,
            (3, 2) => kind = Some(decode_application_response(reader.bytes()?)?),
            (4, 2) => kind = Some(decode_body_chunk(reader.bytes()?)?),
            (5, 2) => kind = Some(decode_reset(reader.bytes()?)?),
            _ => reader.skip(wire)?,
        }
    }
    Ok(ServiceFrame {
        stream_id,
        kind: kind.ok_or_else(|| invalid_data("Muse ServiceFrame has no response kind"))?,
    })
}

fn encode_transport_frame(
    chunk_id: i64,
    chunk_index: u32,
    total_chunks: u32,
    payload: &[u8],
) -> Vec<u8> {
    let mut output = Vec::new();
    if chunk_id != 0 {
        write_varint_field(1, chunk_id as u64, &mut output);
    }
    if chunk_index != 0 {
        write_varint_field(2, u64::from(chunk_index), &mut output);
    }
    if total_chunks != 0 {
        write_varint_field(3, u64::from(total_chunks), &mut output);
    }
    if !payload.is_empty() {
        write_bytes_field(4, payload, &mut output);
    }
    output
}

fn decode_application_response(input: &[u8]) -> io::Result<ServiceFrameKind> {
    let mut reader = ProtoReader::new(input);
    let mut status = 0;
    let mut headers = Vec::new();
    let mut body = Vec::new();
    let mut end_body = false;
    while let Some((field, wire)) = reader.field()? {
        match (field, wire) {
            (1, 0) => status = u32_from_varint(reader.varint()?)?,
            (2, 2) => headers.push(decode_header(reader.bytes()?)?),
            (3, 2) => body = reader.bytes()?.to_vec(),
            (4, 0) => end_body = reader.varint()? != 0,
            _ => reader.skip(wire)?,
        }
    }
    Ok(ServiceFrameKind::Response {
        status,
        headers,
        body,
        end_body,
    })
}

fn decode_header(input: &[u8]) -> io::Result<Header> {
    let mut reader = ProtoReader::new(input);
    let mut key = String::new();
    let mut value = String::new();
    while let Some((field, wire)) = reader.field()? {
        match (field, wire) {
            (1, 2) => key = utf8(reader.bytes()?)?,
            (2, 2) => value = utf8(reader.bytes()?)?,
            _ => reader.skip(wire)?,
        }
    }
    Ok(Header { key, value })
}

fn decode_body_chunk(input: &[u8]) -> io::Result<ServiceFrameKind> {
    let mut reader = ProtoReader::new(input);
    let mut data = Vec::new();
    let mut end_body = false;
    while let Some((field, wire)) = reader.field()? {
        match (field, wire) {
            (1, 2) => data = reader.bytes()?.to_vec(),
            (2, 0) => end_body = reader.varint()? != 0,
            _ => reader.skip(wire)?,
        }
    }
    Ok(ServiceFrameKind::BodyChunk { data, end_body })
}

fn decode_reset(input: &[u8]) -> io::Result<ServiceFrameKind> {
    let mut reader = ProtoReader::new(input);
    let mut code = 0;
    let mut reason = String::new();
    while let Some((field, wire)) = reader.field()? {
        match (field, wire) {
            (1, 0) => code = u32_from_varint(reader.varint()?)?,
            (2, 2) => reason = utf8(reader.bytes()?)?,
            _ => reader.skip(wire)?,
        }
    }
    Ok(ServiceFrameKind::Reset { code, reason })
}

fn write_varint_field(field: u64, value: u64, output: &mut Vec<u8>) {
    write_varint(field << 3, output);
    write_varint(value, output);
}

fn write_bytes_field(field: u64, value: &[u8], output: &mut Vec<u8>) {
    write_varint((field << 3) | 2, output);
    write_varint(value.len() as u64, output);
    output.extend_from_slice(value);
}

fn write_varint(mut value: u64, output: &mut Vec<u8>) {
    loop {
        if value < 0x80 {
            output.push(value as u8);
            break;
        }
        output.push((value as u8) | 0x80);
        value >>= 7;
    }
}

fn u32_from_varint(value: u64) -> io::Result<u32> {
    u32::try_from(value).map_err(|_| invalid_data("protobuf uint32 overflow"))
}

fn utf8(value: &[u8]) -> io::Result<String> {
    String::from_utf8(value.to_vec()).map_err(|_| invalid_data("invalid protobuf UTF-8"))
}

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

struct ProtoReader<'a> {
    input: &'a [u8],
    offset: usize,
}

impl<'a> ProtoReader<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, offset: 0 }
    }

    fn field(&mut self) -> io::Result<Option<(u64, u8)>> {
        if self.offset == self.input.len() {
            return Ok(None);
        }
        let key = self.varint()?;
        let field = key >> 3;
        if field == 0 {
            return Err(invalid_data("invalid protobuf field number"));
        }
        Ok(Some((field, (key & 7) as u8)))
    }

    fn varint(&mut self) -> io::Result<u64> {
        let mut value = 0u64;
        for shift in (0..=63).step_by(7) {
            let byte = *self
                .input
                .get(self.offset)
                .ok_or_else(|| invalid_data("truncated protobuf varint"))?;
            self.offset += 1;
            if shift == 63 && byte > 1 {
                return Err(invalid_data("protobuf varint overflow"));
            }
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(invalid_data("protobuf varint overflow"))
    }

    fn bytes(&mut self) -> io::Result<&'a [u8]> {
        let length = usize::try_from(self.varint()?)
            .map_err(|_| invalid_data("protobuf field too large"))?;
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| invalid_data("protobuf field too large"))?;
        let value = self
            .input
            .get(self.offset..end)
            .ok_or_else(|| invalid_data("truncated protobuf field"))?;
        self.offset = end;
        Ok(value)
    }

    fn skip(&mut self, wire: u8) -> io::Result<()> {
        match wire {
            0 => {
                self.varint()?;
            }
            1 => self.advance(8)?,
            2 => {
                self.bytes()?;
            }
            5 => self.advance(4)?,
            _ => return Err(invalid_data("unsupported protobuf wire type")),
        }
        Ok(())
    }

    fn advance(&mut self, length: usize) -> io::Result<()> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| invalid_data("protobuf field too large"))?;
        if end > self.input.len() {
            return Err(invalid_data("truncated protobuf field"));
        }
        self.offset = end;
        Ok(())
    }
}
