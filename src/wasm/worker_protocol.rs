//! Private, bounded v1 framing for the one-request Wasm worker.
//!
//! This intentionally transports only one module and terminal observation. It
//! is not a capability, resource, or generic streaming protocol.

use std::io::{Read, Write};

const MAGIC: [u8; 4] = *b"KWW1";
const VERSION: u16 = 1;
const HEADER_LEN: usize = 11;
pub(super) const MAX_FRAME_PAYLOAD: usize = 1024 * 1024;
/// One-request worker protocol ceiling.  This is intentionally distinct from
/// admission policy: only the process transport is bounded by this contract.
pub(super) const WORKER_PROTOCOL_MAX_MODULE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_DIAGNOSTIC_BYTES: usize = 1024;
const NO_STATUS_CODE: i32 = i32::MIN;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(super) enum Kind {
    Hello = 1,
    HelloAck = 2,
    ExecuteStart = 3,
    ModuleChunk = 4,
    ExecuteEnd = 5,
    Cancel = 6,
    Terminal = 7,
    ExecuteAck = 8,
}

impl TryFrom<u8> for Kind {
    type Error = ProtocolError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Hello),
            2 => Ok(Self::HelloAck),
            3 => Ok(Self::ExecuteStart),
            4 => Ok(Self::ModuleChunk),
            5 => Ok(Self::ExecuteEnd),
            6 => Ok(Self::Cancel),
            7 => Ok(Self::Terminal),
            8 => Ok(Self::ExecuteAck),
            _ => Err(ProtocolError::UnknownKind),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(super) enum TerminalKind {
    Completed = 1,
    Cancelled = 2,
    DeadlineExceeded = 3,
    OutOfFuel = 4,
    Trapped = 5,
    NonzeroExit = 6,
    ChildFailure = 7,
    ProtocolFailure = 8,
    WorkerFailure = 9,
    ForcedContainment = 10,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(super) enum RootOutcome {
    None = 0,
    Started = 1,
    Exited = 2,
    StartedWithThreadRejections = 3,
    ExitedWithThreadRejections = 4,
}
impl TryFrom<u8> for RootOutcome {
    type Error = ProtocolError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::Started),
            2 => Ok(Self::Exited),
            3 => Ok(Self::StartedWithThreadRejections),
            4 => Ok(Self::ExitedWithThreadRejections),
            _ => Err(ProtocolError::InvalidTerminal),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TerminalDetail {
    pub(super) root_outcome: RootOutcome,
    pub(super) capacity_rejections: u32,
    pub(super) closing_rejections: u32,
    pub(super) fuel_rejections: u32,
    pub(super) epoch_rejections: u32,
    pub(super) status_code: Option<i32>,
}
impl TerminalDetail {
    pub(super) const fn none() -> Self {
        Self {
            root_outcome: RootOutcome::None,
            capacity_rejections: 0,
            closing_rejections: 0,
            fuel_rejections: 0,
            epoch_rejections: 0,
            status_code: None,
        }
    }
    fn validate(self, kind: TerminalKind) -> Result<(), ProtocolError> {
        let counts = self.capacity_rejections != 0
            || self.closing_rejections != 0
            || self.fuel_rejections != 0
            || self.epoch_rejections != 0;
        match kind {
            TerminalKind::Completed
                if self.root_outcome == RootOutcome::None || self.status_code.is_some() =>
            {
                Err(ProtocolError::InvalidTerminal)
            }
            TerminalKind::Completed
                if counts
                    && !matches!(
                        self.root_outcome,
                        RootOutcome::StartedWithThreadRejections
                            | RootOutcome::ExitedWithThreadRejections
                    ) =>
            {
                Err(ProtocolError::InvalidTerminal)
            }
            TerminalKind::Completed
                if !counts
                    && matches!(
                        self.root_outcome,
                        RootOutcome::StartedWithThreadRejections
                            | RootOutcome::ExitedWithThreadRejections
                    ) =>
            {
                Err(ProtocolError::InvalidTerminal)
            }
            TerminalKind::NonzeroExit
                if self.root_outcome != RootOutcome::None
                    || counts
                    || self.status_code.is_none() =>
            {
                Err(ProtocolError::InvalidTerminal)
            }
            TerminalKind::ChildFailure if self.root_outcome != RootOutcome::None || counts => {
                Err(ProtocolError::InvalidTerminal)
            }
            TerminalKind::Completed | TerminalKind::NonzeroExit => Ok(()),
            _ if self.root_outcome != RootOutcome::None || counts || self.status_code.is_some() => {
                Err(ProtocolError::InvalidTerminal)
            }
            _ => Ok(()),
        }
    }
}

impl TryFrom<u8> for TerminalKind {
    type Error = ProtocolError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Completed),
            2 => Ok(Self::Cancelled),
            3 => Ok(Self::DeadlineExceeded),
            4 => Ok(Self::OutOfFuel),
            5 => Ok(Self::Trapped),
            6 => Ok(Self::NonzeroExit),
            7 => Ok(Self::ChildFailure),
            8 => Ok(Self::ProtocolFailure),
            9 => Ok(Self::WorkerFailure),
            10 => Ok(Self::ForcedContainment),
            _ => Err(ProtocolError::InvalidTerminal),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ProtocolError {
    BadMagic,
    UnsupportedVersion,
    UnknownKind,
    Truncated,
    TrailingBytes,
    FrameTooLarge,
    InvalidRequestId,
    InvalidPayload,
    LengthOverflow,
    ModuleTooLarge,
    WrongRequestId,
    UnexpectedMessage,
    DuplicateTerminal,
    InvalidTerminal,
    Sequence,
    DiagnosticTooLarge,
}

/// Facade semantic primitives needed to reconstruct compiler/limit settings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ExecuteMetadata {
    pub(super) max_wasm_stack_bytes: u64,
    pub(super) reserved_memory_bytes: u64,
    pub(super) maximum_active_roots: u64,
    pub(super) total_fuel: u64,
    pub(super) root_fuel: u64,
    pub(super) child_fuel: u64,
    pub(super) epoch_deadline_millis: u64,
    pub(super) epoch_tick_millis: u64,
    pub(super) maximum_epoch_registrations: u64,
    pub(super) max_module_bytes: u64,
    pub(super) max_shared_memory_pages: u32,
    pub(super) max_guest_threads: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct FinalCounters {
    pub(super) active_roots: u64,
    pub(super) live_stores: u64,
    pub(super) live_instances: u64,
    pub(super) active_epoch_registrations: u64,
    pub(super) live_threads: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Message {
    Hello {
        request_id: u64,
    },
    HelloAck {
        request_id: u64,
    },
    ExecuteAck { request_id: u64 },
    ExecuteStart {
        request_id: u64,
        module_len: u64,
        metadata: ExecuteMetadata,
    },
    ModuleChunk {
        request_id: u64,
        sequence: u32,
        bytes: Vec<u8>,
    },
    ExecuteEnd {
        request_id: u64,
    },
    Cancel {
        request_id: u64,
    },
    Terminal {
        request_id: u64,
        kind: TerminalKind,
        detail: TerminalDetail,
        diagnostic: String,
        counters: FinalCounters,
    },
}

impl Message {
    fn request_id(&self) -> u64 {
        match self {
            Self::Hello { request_id }
            | Self::HelloAck { request_id }
            | Self::ExecuteAck { request_id }
            | Self::ExecuteEnd { request_id }
            | Self::Cancel { request_id } => *request_id,
            Self::ExecuteStart { request_id, .. }
            | Self::ModuleChunk { request_id, .. }
            | Self::Terminal { request_id, .. } => *request_id,
        }
    }
    fn kind(&self) -> Kind {
        match self {
            Self::Hello { .. } => Kind::Hello,
            Self::HelloAck { .. } => Kind::HelloAck,
            Self::ExecuteAck { .. } => Kind::ExecuteAck,
            Self::ExecuteStart { .. } => Kind::ExecuteStart,
            Self::ModuleChunk { .. } => Kind::ModuleChunk,
            Self::ExecuteEnd { .. } => Kind::ExecuteEnd,
            Self::Cancel { .. } => Kind::Cancel,
            Self::Terminal { .. } => Kind::Terminal,
        }
    }
}

pub(super) fn encode(message: &Message) -> Result<Vec<u8>, ProtocolError> {
    if message.request_id() == 0 {
        return Err(ProtocolError::InvalidRequestId);
    }
    let mut payload = Vec::new();
    put_u64(&mut payload, message.request_id());
    match message {
        Message::Hello { .. }
        | Message::HelloAck { .. }
        | Message::ExecuteAck { .. }
        | Message::ExecuteEnd { .. }
        | Message::Cancel { .. } => {}
        Message::ExecuteStart {
            module_len,
            metadata,
            ..
        } => {
            put_u64(&mut payload, *module_len);
            put_metadata(&mut payload, metadata);
        }
        Message::ModuleChunk {
            sequence, bytes, ..
        } => {
            put_u32(&mut payload, *sequence);
            payload.extend_from_slice(bytes);
        }
        Message::Terminal {
            kind,
            detail,
            diagnostic,
            counters,
            ..
        } => {
            detail.validate(*kind)?;
            if diagnostic.len() > MAX_DIAGNOSTIC_BYTES {
                return Err(ProtocolError::DiagnosticTooLarge);
            }
            payload.push(*kind as u8);
            payload.push(detail.root_outcome as u8);
            put_u32(&mut payload, detail.capacity_rejections);
            put_u32(&mut payload, detail.closing_rejections);
            put_u32(&mut payload, detail.fuel_rejections);
            put_u32(&mut payload, detail.epoch_rejections);
            put_i32(&mut payload, detail.status_code.unwrap_or(NO_STATUS_CODE));
            put_u16(&mut payload, diagnostic.len() as u16);
            payload.extend_from_slice(diagnostic.as_bytes());
            put_counters(&mut payload, counters);
        }
    }
    if payload.len() > MAX_FRAME_PAYLOAD {
        return Err(ProtocolError::FrameTooLarge);
    }
    let mut frame = Vec::with_capacity(HEADER_LEN + payload.len());
    frame.extend_from_slice(&MAGIC);
    put_u16(&mut frame, VERSION);
    frame.push(message.kind() as u8);
    put_u32(
        &mut frame,
        u32::try_from(payload.len()).map_err(|_| ProtocolError::FrameTooLarge)?,
    );
    frame.extend_from_slice(&payload);
    Ok(frame)
}

pub(super) fn decode(frame: &[u8]) -> Result<Message, ProtocolError> {
    if frame.len() < HEADER_LEN {
        return Err(ProtocolError::Truncated);
    }
    if frame[..4] != MAGIC {
        return Err(ProtocolError::BadMagic);
    }
    if get_u16(&frame[4..6])? != VERSION {
        return Err(ProtocolError::UnsupportedVersion);
    }
    let kind = Kind::try_from(frame[6])?;
    let length =
        usize::try_from(get_u32(&frame[7..11])?).map_err(|_| ProtocolError::LengthOverflow)?;
    if length > MAX_FRAME_PAYLOAD {
        return Err(ProtocolError::FrameTooLarge);
    }
    let end = HEADER_LEN
        .checked_add(length)
        .ok_or(ProtocolError::LengthOverflow)?;
    if frame.len() < end {
        return Err(ProtocolError::Truncated);
    }
    if frame.len() != end {
        return Err(ProtocolError::TrailingBytes);
    }
    let mut input = &frame[HEADER_LEN..];
    let request_id = take_u64(&mut input)?;
    if request_id == 0 {
        return Err(ProtocolError::InvalidRequestId);
    }
    let message = match kind {
        Kind::Hello => Message::Hello { request_id },
        Kind::HelloAck => Message::HelloAck { request_id },
        Kind::ExecuteAck => Message::ExecuteAck { request_id },
        Kind::ExecuteStart => {
            let module_len = take_u64(&mut input)?;
            let metadata = take_metadata(&mut input)?;
            if module_len > WORKER_PROTOCOL_MAX_MODULE_BYTES
                || module_len > metadata.max_module_bytes
            {
                return Err(ProtocolError::ModuleTooLarge);
            }
            Message::ExecuteStart {
                request_id,
                module_len,
                metadata,
            }
        }
        Kind::ModuleChunk => Message::ModuleChunk {
            request_id,
            sequence: take_u32(&mut input)?,
            // Move the bounded remainder out of the frame so the common
            // trailing-byte check below sees that this variant consumed it.
            bytes: std::mem::take(&mut input).to_vec(),
        },
        Kind::ExecuteEnd => Message::ExecuteEnd { request_id },
        Kind::Cancel => Message::Cancel { request_id },
        Kind::Terminal => {
            let terminal = TerminalKind::try_from(take_u8(&mut input)?)?;
            let detail = TerminalDetail {
                root_outcome: RootOutcome::try_from(take_u8(&mut input)?)?,
                capacity_rejections: take_u32(&mut input)?,
                closing_rejections: take_u32(&mut input)?,
                fuel_rejections: take_u32(&mut input)?,
                epoch_rejections: take_u32(&mut input)?,
                status_code: match take_i32(&mut input)? {
                    NO_STATUS_CODE => None,
                    code => Some(code),
                },
            };
            detail.validate(terminal)?;
            let text_len = usize::from(take_u16(&mut input)?);
            if text_len > MAX_DIAGNOSTIC_BYTES {
                return Err(ProtocolError::DiagnosticTooLarge);
            }
            let text = take(&mut input, text_len)?;
            let diagnostic = std::str::from_utf8(&text)
                .map_err(|_| ProtocolError::InvalidPayload)?
                .to_owned();
            Message::Terminal {
                request_id,
                kind: terminal,
                detail,
                diagnostic,
                counters: take_counters(&mut input)?,
            }
        }
    };
    if !input.is_empty() {
        return Err(ProtocolError::TrailingBytes);
    }
    Ok(message)
}

/// Reads a complete bounded frame. Header validation precedes allocation.
pub(super) fn read_message<R: Read>(reader: &mut R) -> Result<Message, ProtocolError> {
    let mut header = [0; HEADER_LEN];
    reader
        .read_exact(&mut header)
        .map_err(|_| ProtocolError::Truncated)?;
    if header[..4] != MAGIC {
        return Err(ProtocolError::BadMagic);
    }
    if get_u16(&header[4..6])? != VERSION {
        return Err(ProtocolError::UnsupportedVersion);
    }
    let _ = Kind::try_from(header[6])?;
    let length =
        usize::try_from(get_u32(&header[7..11])?).map_err(|_| ProtocolError::LengthOverflow)?;
    if length > MAX_FRAME_PAYLOAD {
        return Err(ProtocolError::FrameTooLarge);
    }
    let total = HEADER_LEN
        .checked_add(length)
        .ok_or(ProtocolError::LengthOverflow)?;
    let mut frame = Vec::with_capacity(total);
    frame.extend_from_slice(&header);
    frame.resize(total, 0);
    reader
        .read_exact(&mut frame[HEADER_LEN..])
        .map_err(|_| ProtocolError::Truncated)?;
    decode(&frame)
}

pub(super) fn write_message<W: Write>(
    writer: &mut W,
    message: &Message,
) -> Result<(), ProtocolError> {
    let frame = encode(message)?;
    writer
        .write_all(&frame)
        .map_err(|_| ProtocolError::InvalidPayload)?;
    writer.flush().map_err(|_| ProtocolError::InvalidPayload)
}

/// Assembles only an ExecuteStart/chunks/ExecuteEnd sequence for one request.
#[derive(Debug)]
pub(super) struct ModuleAssembler {
    request_id: u64,
    declared: usize,
    caller_max: usize,
    next: u32,
    ended: bool,
    terminal: bool,
    module: Vec<u8>,
}
impl ModuleAssembler {
    pub(super) fn start(
        request_id: u64,
        module_len: u64,
        caller_max: u64,
    ) -> Result<Self, ProtocolError> {
        if request_id == 0 {
            return Err(ProtocolError::InvalidRequestId);
        }
        if module_len > WORKER_PROTOCOL_MAX_MODULE_BYTES || module_len > caller_max {
            return Err(ProtocolError::ModuleTooLarge);
        }
        let declared = usize::try_from(module_len).map_err(|_| ProtocolError::LengthOverflow)?;
        let caller_max = usize::try_from(caller_max).map_err(|_| ProtocolError::LengthOverflow)?;
        Ok(Self {
            request_id,
            declared,
            caller_max,
            next: 0,
            ended: false,
            terminal: false,
            module: Vec::new(),
        })
    }
    pub(super) fn accept(&mut self, message: Message) -> Result<Option<Vec<u8>>, ProtocolError> {
        if message.request_id() != self.request_id {
            return Err(ProtocolError::WrongRequestId);
        }
        match message {
            Message::ModuleChunk {
                sequence, bytes, ..
            } => {
                if self.ended
                    || sequence != self.next
                    || bytes.len() > MAX_FRAME_PAYLOAD.saturating_sub(12)
                {
                    return Err(ProtocolError::Sequence);
                }
                let total = self
                    .module
                    .len()
                    .checked_add(bytes.len())
                    .ok_or(ProtocolError::LengthOverflow)?;
                if total > self.declared || total > self.caller_max {
                    return Err(ProtocolError::ModuleTooLarge);
                }
                self.module.extend_from_slice(&bytes);
                self.next = self
                    .next
                    .checked_add(1)
                    .ok_or(ProtocolError::LengthOverflow)?;
                Ok(None)
            }
            Message::ExecuteEnd { .. } => {
                if self.ended || self.module.len() != self.declared {
                    return Err(ProtocolError::UnexpectedMessage);
                }
                self.ended = true;
                Ok(Some(std::mem::take(&mut self.module)))
            }
            Message::Terminal { .. } => {
                if !self.ended {
                    Err(ProtocolError::UnexpectedMessage)
                } else if self.terminal {
                    Err(ProtocolError::DuplicateTerminal)
                } else {
                    self.terminal = true;
                    Ok(None)
                }
            }
            _ => Err(ProtocolError::UnexpectedMessage),
        }
    }
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn get_u16(input: &[u8]) -> Result<u16, ProtocolError> {
    Ok(u16::from_le_bytes(
        input.try_into().map_err(|_| ProtocolError::Truncated)?,
    ))
}
fn get_u32(input: &[u8]) -> Result<u32, ProtocolError> {
    Ok(u32::from_le_bytes(
        input.try_into().map_err(|_| ProtocolError::Truncated)?,
    ))
}
fn take(input: &mut &[u8], count: usize) -> Result<Vec<u8>, ProtocolError> {
    if input.len() < count {
        return Err(ProtocolError::Truncated);
    }
    let value = input[..count].to_vec();
    *input = &input[count..];
    Ok(value)
}
fn take_u8(input: &mut &[u8]) -> Result<u8, ProtocolError> {
    Ok(take(input, 1)?[0])
}
fn take_u16(input: &mut &[u8]) -> Result<u16, ProtocolError> {
    get_u16(&take(input, 2)?)
}
fn take_u32(input: &mut &[u8]) -> Result<u32, ProtocolError> {
    get_u32(&take(input, 4)?)
}
fn take_u64(input: &mut &[u8]) -> Result<u64, ProtocolError> {
    Ok(u64::from_le_bytes(
        take(input, 8)?
            .try_into()
            .map_err(|_| ProtocolError::Truncated)?,
    ))
}
fn take_i32(input: &mut &[u8]) -> Result<i32, ProtocolError> {
    Ok(i32::from_le_bytes(
        take(input, 4)?
            .try_into()
            .map_err(|_| ProtocolError::Truncated)?,
    ))
}
fn put_metadata(out: &mut Vec<u8>, value: &ExecuteMetadata) {
    for v in [
        value.max_wasm_stack_bytes,
        value.reserved_memory_bytes,
        value.maximum_active_roots,
        value.total_fuel,
        value.root_fuel,
        value.child_fuel,
        value.epoch_deadline_millis,
        value.epoch_tick_millis,
        value.maximum_epoch_registrations,
        value.max_module_bytes,
    ] {
        put_u64(out, v);
    }
    put_u32(out, value.max_shared_memory_pages);
    put_u64(out, value.max_guest_threads);
}
fn take_metadata(input: &mut &[u8]) -> Result<ExecuteMetadata, ProtocolError> {
    Ok(ExecuteMetadata {
        max_wasm_stack_bytes: take_u64(input)?,
        reserved_memory_bytes: take_u64(input)?,
        maximum_active_roots: take_u64(input)?,
        total_fuel: take_u64(input)?,
        root_fuel: take_u64(input)?,
        child_fuel: take_u64(input)?,
        epoch_deadline_millis: take_u64(input)?,
        epoch_tick_millis: take_u64(input)?,
        maximum_epoch_registrations: take_u64(input)?,
        max_module_bytes: take_u64(input)?,
        max_shared_memory_pages: take_u32(input)?,
        max_guest_threads: take_u64(input)?,
    })
}
fn put_counters(out: &mut Vec<u8>, value: &FinalCounters) {
    for v in [
        value.active_roots,
        value.live_stores,
        value.live_instances,
        value.active_epoch_registrations,
        value.live_threads,
    ] {
        put_u64(out, v);
    }
}
fn take_counters(input: &mut &[u8]) -> Result<FinalCounters, ProtocolError> {
    Ok(FinalCounters {
        active_roots: take_u64(input)?,
        live_stores: take_u64(input)?,
        live_instances: take_u64(input)?,
        active_epoch_registrations: take_u64(input)?,
        live_threads: take_u64(input)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn metadata() -> ExecuteMetadata {
        ExecuteMetadata {
            max_wasm_stack_bytes: 1,
            reserved_memory_bytes: 2,
            maximum_active_roots: 3,
            total_fuel: 4,
            root_fuel: 5,
            child_fuel: 6,
            epoch_deadline_millis: 7,
            epoch_tick_millis: 8,
            maximum_epoch_registrations: 9,
            max_module_bytes: 32 * 1024 * 1024,
            max_shared_memory_pages: 10,
            max_guest_threads: 11,
        }
    }
    #[test]
    fn round_trip_start_and_terminal() {
        let start = Message::ExecuteStart {
            request_id: 7,
            module_len: 3,
            metadata: metadata(),
        };
        assert_eq!(decode(&encode(&start).unwrap()).unwrap(), start);
        let terminal = Message::Terminal {
            request_id: 7,
            kind: TerminalKind::Completed,
            detail: TerminalDetail {
                root_outcome: RootOutcome::Started,
                ..TerminalDetail::none()
            },
            diagnostic: "ok".into(),
            counters: FinalCounters {
                active_roots: 0,
                live_stores: 0,
                live_instances: 0,
                active_epoch_registrations: 0,
                live_threads: 0,
            },
        };
        assert_eq!(decode(&encode(&terminal).unwrap()).unwrap(), terminal);
    }
    #[test]
    fn round_trip_module_chunk_consumes_its_payload() {
        let chunk = Message::ModuleChunk {
            request_id: 7,
            sequence: 3,
            bytes: vec![0, 1, 2, 3],
        };
        assert_eq!(decode(&encode(&chunk).unwrap()).unwrap(), chunk);
    }
    #[test]
    fn multi_chunk_module_is_ordered_and_exact() {
        let chunk = vec![7; MAX_FRAME_PAYLOAD - 12];
        let mut a = ModuleAssembler::start(
            1,
            (chunk.len() * 2) as u64,
            WORKER_PROTOCOL_MAX_MODULE_BYTES,
        )
        .unwrap();
        assert_eq!(
            a.accept(Message::ModuleChunk {
                request_id: 1,
                sequence: 0,
                bytes: chunk.clone()
            })
            .unwrap(),
            None
        );
        assert_eq!(
            a.accept(Message::ModuleChunk {
                request_id: 1,
                sequence: 1,
                bytes: chunk.clone()
            })
            .unwrap(),
            None
        );
        let module = a
            .accept(Message::ExecuteEnd { request_id: 1 })
            .unwrap()
            .unwrap();
        assert_eq!(module.len(), chunk.len() * 2);
    }
    #[test]
    fn rejects_bad_headers_and_trailing_data() {
        let mut frame = encode(&Message::Hello { request_id: 1 }).unwrap();
        frame[0] = 0;
        assert_eq!(decode(&frame), Err(ProtocolError::BadMagic));
        let mut frame = encode(&Message::Hello { request_id: 1 }).unwrap();
        frame[4] = 2;
        assert_eq!(decode(&frame), Err(ProtocolError::UnsupportedVersion));
        let mut frame = encode(&Message::Hello { request_id: 1 }).unwrap();
        frame[6] = 99;
        assert_eq!(decode(&frame), Err(ProtocolError::UnknownKind));
        let frame = encode(&Message::Hello { request_id: 1 }).unwrap();
        assert_eq!(decode(&frame[..8]), Err(ProtocolError::Truncated));
        let mut frame = frame;
        frame.push(0);
        assert_eq!(decode(&frame), Err(ProtocolError::TrailingBytes));
    }
    #[test]
    fn rejects_oversize_and_invalid_module_lengths() {
        assert!(matches!(
            ModuleAssembler::start(
                1,
                WORKER_PROTOCOL_MAX_MODULE_BYTES + 1,
                WORKER_PROTOCOL_MAX_MODULE_BYTES + 1
            ),
            Err(ProtocolError::ModuleTooLarge)
        ));
        assert!(matches!(
            ModuleAssembler::start(1, 17, 16),
            Err(ProtocolError::ModuleTooLarge)
        ));
        let start = Message::ExecuteStart {
            request_id: 1,
            module_len: 17,
            metadata: ExecuteMetadata {
                max_module_bytes: 16,
                ..metadata()
            },
        };
        assert_eq!(
            decode(&encode(&start).unwrap()),
            Err(ProtocolError::ModuleTooLarge)
        );
        let message = Message::ModuleChunk {
            request_id: 1,
            sequence: 0,
            bytes: vec![0; MAX_FRAME_PAYLOAD],
        };
        assert_eq!(encode(&message), Err(ProtocolError::FrameTooLarge));
    }
    #[test]
    fn rejects_sequence_id_end_and_duplicate_terminal_errors() {
        let mut a = ModuleAssembler::start(5, 1, 1).unwrap();
        assert_eq!(
            a.accept(Message::ModuleChunk {
                request_id: 4,
                sequence: 0,
                bytes: vec![1]
            }),
            Err(ProtocolError::WrongRequestId)
        );
        assert_eq!(
            a.accept(Message::ModuleChunk {
                request_id: 5,
                sequence: 1,
                bytes: vec![1]
            }),
            Err(ProtocolError::Sequence)
        );
        assert_eq!(
            a.accept(Message::ExecuteEnd { request_id: 5 }),
            Err(ProtocolError::UnexpectedMessage)
        );
        assert_eq!(
            a.accept(Message::ModuleChunk {
                request_id: 5,
                sequence: 0,
                bytes: vec![1]
            })
            .unwrap(),
            None
        );
        assert!(a
            .accept(Message::ExecuteEnd { request_id: 5 })
            .unwrap()
            .is_some());
        let terminal = Message::Terminal {
            request_id: 5,
            kind: TerminalKind::Completed,
            detail: TerminalDetail {
                root_outcome: RootOutcome::Started,
                ..TerminalDetail::none()
            },
            diagnostic: String::new(),
            counters: FinalCounters {
                active_roots: 0,
                live_stores: 0,
                live_instances: 0,
                active_epoch_registrations: 0,
                live_threads: 0,
            },
        };
        assert_eq!(a.accept(terminal.clone()).unwrap(), None);
        assert_eq!(a.accept(terminal), Err(ProtocolError::DuplicateTerminal));
    }
    #[test]
    fn diagnostic_is_bounded() {
        let message = Message::Terminal {
            request_id: 1,
            kind: TerminalKind::WorkerFailure,
            detail: TerminalDetail::none(),
            diagnostic: "x".repeat(MAX_DIAGNOSTIC_BYTES + 1),
            counters: FinalCounters {
                active_roots: 0,
                live_stores: 0,
                live_instances: 0,
                active_epoch_registrations: 0,
                live_threads: 0,
            },
        };
        assert_eq!(encode(&message), Err(ProtocolError::DiagnosticTooLarge));
    }
    #[test]
    fn typed_root_outcome_rejections_and_status_round_trip() {
        let counters = FinalCounters {
            active_roots: 0,
            live_stores: 0,
            live_instances: 0,
            active_epoch_registrations: 0,
            live_threads: 0,
        };
        let completed = Message::Terminal {
            request_id: 1,
            kind: TerminalKind::Completed,
            detail: TerminalDetail {
                root_outcome: RootOutcome::StartedWithThreadRejections,
                capacity_rejections: 1,
                closing_rejections: 2,
                fuel_rejections: 3,
                epoch_rejections: 4,
                status_code: None,
            },
            diagnostic: "context".into(),
            counters,
        };
        assert_eq!(decode(&encode(&completed).unwrap()).unwrap(), completed);
        let exit = Message::Terminal {
            request_id: 1,
            kind: TerminalKind::NonzeroExit,
            detail: TerminalDetail {
                status_code: Some(-9),
                ..TerminalDetail::none()
            },
            diagnostic: String::new(),
            counters,
        };
        assert_eq!(decode(&encode(&exit).unwrap()).unwrap(), exit);
    }
    #[test]
    fn terminal_rejects_illegal_machine_combinations() {
        let counters = FinalCounters {
            active_roots: 0,
            live_stores: 0,
            live_instances: 0,
            active_epoch_registrations: 0,
            live_threads: 0,
        };
        let missing_outcome = Message::Terminal {
            request_id: 1,
            kind: TerminalKind::Completed,
            detail: TerminalDetail::none(),
            diagnostic: String::new(),
            counters,
        };
        assert_eq!(
            encode(&missing_outcome),
            Err(ProtocolError::InvalidTerminal)
        );
        let missing_status = Message::Terminal {
            request_id: 1,
            kind: TerminalKind::NonzeroExit,
            detail: TerminalDetail::none(),
            diagnostic: String::new(),
            counters,
        };
        assert_eq!(encode(&missing_status), Err(ProtocolError::InvalidTerminal));
        let cancelled_with_status = Message::Terminal {
            request_id: 1,
            kind: TerminalKind::Cancelled,
            detail: TerminalDetail {
                status_code: Some(1),
                ..TerminalDetail::none()
            },
            diagnostic: String::new(),
            counters,
        };
        assert_eq!(
            encode(&cancelled_with_status),
            Err(ProtocolError::InvalidTerminal)
        );
    }
}
