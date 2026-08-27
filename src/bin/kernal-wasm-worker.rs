//! Private one-request Wasm worker. Parent supervision and forced tree reaping
//! are deliberately phase-D responsibilities.

#[path = "../wasm/worker_protocol.rs"]
mod worker_protocol;

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use kernal_api::async_engine::{CancellationSource, RuntimeBuilder};
use kernal_api::wasm::{
    SketchCompiler, SketchCompilerConfig, SketchEpochLimits, SketchExecutionError,
    SketchExecutionLimits, SketchFuelLimits, SketchModulePolicy, ThreadedRootOutcome,
};
use worker_protocol::{
    read_message, write_message, ExecuteMetadata, FinalCounters, Message, ModuleAssembler,
    ProtocolError, RootOutcome, TerminalDetail, TerminalKind,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("kernal-wasm-worker: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut input = io::stdin();
    let mut output = io::stdout();
    let Some((request_id, metadata, module)) = run_with_io(&mut input, &mut output)? else {
        return Ok(());
    };
    execute_request(request_id, metadata, module, &mut output)
}

/// Consume the bounded pre-execution request sequence and write only its
/// protocol-level response. Keeping this independent of stdin's cancellation
/// reader makes the framing state machine testable without a child process.
fn run_with_io<R: io::Read, W: io::Write>(
    input: &mut R,
    output: &mut W,
) -> Result<Option<(u64, ExecuteMetadata, Vec<u8>)>, String> {
    let hello = read_message(input).map_err(protocol_text)?;
    let request_id = match hello {
        Message::Hello { request_id } => request_id,
        _ => return Err("expected hello".into()),
    };
    write_message(output, &Message::HelloAck { request_id }).map_err(protocol_text)?;
    let start = read_message(input).map_err(protocol_text)?;
    let (module_len, metadata) = match start {
        Message::ExecuteStart {
            request_id: id,
            module_len,
            metadata,
        } if id == request_id => (module_len, metadata),
        _ => {
            protocol_terminal(output, request_id, "expected matching execute-start")?;
            return Ok(None);
        }
    };
    let mut assembler = ModuleAssembler::start(request_id, module_len, metadata.max_module_bytes)
        .map_err(protocol_text)?;
    let module = loop {
        let message = read_message(input).map_err(protocol_text)?;
        if let Some(module) = assembler.accept(message).map_err(protocol_text)? {
            break module;
        }
    };
    Ok(Some((request_id, metadata, module)))
}

fn execute_request(
    request_id: u64,
    metadata: ExecuteMetadata,
    module: Vec<u8>,
    output: &mut impl io::Write,
) -> Result<(), String> {
    let (config, policy) = reconstruct(metadata).map_err(|text| {
        protocol_terminal(output, request_id, &text)
            .err()
            .unwrap_or(text)
    })?;
    let cancellation = CancellationSource::new();
    let token = cancellation.token();
    let control_done = Arc::new(AtomicBool::new(false));
    let control_done_thread = Arc::clone(&control_done);
    let source = cancellation.clone();
    std::thread::spawn(move || {
        let mut control = io::stdin();
        match read_message(&mut control) {
            Ok(Message::Cancel { request_id: id }) if id == request_id => source.cancel(),
            Ok(_) | Err(_) => source.cancel(),
        }
        control_done_thread.store(true, Ordering::Release);
    });
    let runtime = RuntimeBuilder::current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let compiler = match SketchCompiler::new(config) {
        Ok(value) => value,
        Err(error) => return protocol_terminal(output, request_id, error.to_string().as_str()),
    };
    let sketch = match compiler.admit(&module, policy) {
        Ok(value) => value,
        Err(error) => return protocol_terminal(output, request_id, error.to_string().as_str()),
    };
    let result = runtime.run(async {
        sketch
            .execute_threaded_root_cancellable(runtime.handle(), token)
            .await
    });
    let _ = sketch.close_threaded_root();
    drop(sketch);
    let counters = snapshot(&compiler);
    let (kind, detail, diagnostic) = map_result(result);
    let (kind, detail, diagnostic) = if counters_are_zero(counters) {
        (kind, detail, diagnostic)
    } else {
        (
            TerminalKind::WorkerFailure,
            TerminalDetail::none(),
            "nonzero-worker-counters".into(),
        )
    };
    write_message(
        output,
        &Message::Terminal {
            request_id,
            kind,
            detail,
            diagnostic: bound(diagnostic),
            counters,
        },
    )
    .map_err(protocol_text)?;
    let _ = control_done.load(Ordering::Acquire);
    Ok(())
}

fn reconstruct(
    metadata: ExecuteMetadata,
) -> Result<(SketchCompilerConfig, SketchModulePolicy), String> {
    let roots =
        usize::try_from(metadata.maximum_active_roots).map_err(|_| "active-roots-overflow")?;
    let stack = usize::try_from(metadata.max_wasm_stack_bytes).map_err(|_| "stack-overflow")?;
    let threads =
        usize::try_from(metadata.max_guest_threads).map_err(|_| "thread-limit-overflow")?;
    let fuel = SketchFuelLimits::new(metadata.total_fuel, metadata.root_fuel, metadata.child_fuel)
        .map_err(|e| e.to_string())?;
    let epoch = SketchEpochLimits::new(
        Duration::from_millis(metadata.epoch_deadline_millis),
        Duration::from_millis(metadata.epoch_tick_millis),
        usize::try_from(metadata.maximum_epoch_registrations)
            .map_err(|_| "epoch-registration-overflow")?,
    )
    .map_err(|e| e.to_string())?;
    let limits = SketchExecutionLimits::new(metadata.reserved_memory_bytes, roots)
        .map_err(|e| e.to_string())?
        .with_fuel_limits(fuel)
        .map_err(|e| e.to_string())?
        .with_epoch_limits(epoch)
        .map_err(|e| e.to_string())?;
    let config = SketchCompilerConfig::new(stack)
        .map_err(|e| e.to_string())?
        .with_execution_limits(limits)
        .map_err(|e| e.to_string())?;
    let policy = SketchModulePolicy::threaded_rust_v1(
        usize::try_from(metadata.max_module_bytes).map_err(|_| "module-limit-overflow")?,
        metadata.max_shared_memory_pages,
    )
    .map_err(|e| e.to_string())?
    .with_max_guest_threads(threads)
    .map_err(|e| e.to_string())?;
    Ok((config, policy))
}

fn map_result(
    result: Result<ThreadedRootOutcome, SketchExecutionError>,
) -> (TerminalKind, TerminalDetail, String) {
    match result {
        Ok(ThreadedRootOutcome::Started) => (
            TerminalKind::Completed,
            TerminalDetail {
                root_outcome: RootOutcome::Started,
                ..TerminalDetail::none()
            },
            "started".into(),
        ),
        Ok(ThreadedRootOutcome::Exited) => (
            TerminalKind::Completed,
            TerminalDetail {
                root_outcome: RootOutcome::Exited,
                ..TerminalDetail::none()
            },
            "exited".into(),
        ),
        Ok(ThreadedRootOutcome::StartedWithThreadRejections(r)) => (
            TerminalKind::Completed,
            rejections(RootOutcome::StartedWithThreadRejections, r),
            "started".into(),
        ),
        Ok(ThreadedRootOutcome::ExitedWithThreadRejections(r)) => (
            TerminalKind::Completed,
            rejections(RootOutcome::ExitedWithThreadRejections, r),
            "exited".into(),
        ),
        Err(SketchExecutionError::Cancelled) => (
            TerminalKind::Cancelled,
            TerminalDetail::none(),
            "cancelled".into(),
        ),
        Err(SketchExecutionError::DeadlineExceeded) => (
            TerminalKind::DeadlineExceeded,
            TerminalDetail::none(),
            "deadline-exceeded".into(),
        ),
        Err(SketchExecutionError::OutOfFuel) => (
            TerminalKind::OutOfFuel,
            TerminalDetail::none(),
            "out-of-fuel".into(),
        ),
        Err(SketchExecutionError::Trapped) => (
            TerminalKind::Trapped,
            TerminalDetail::none(),
            "trapped".into(),
        ),
        Err(SketchExecutionError::NonzeroExit { code }) => (
            TerminalKind::NonzeroExit,
            TerminalDetail {
                status_code: Some(code),
                ..TerminalDetail::none()
            },
            "nonzero-exit".into(),
        ),
        Err(SketchExecutionError::ChildNonzeroExit { code }) => (
            TerminalKind::ChildFailure,
            TerminalDetail {
                status_code: Some(code),
                ..TerminalDetail::none()
            },
            "child-nonzero-exit".into(),
        ),
        Err(SketchExecutionError::ChildTrapped)
        | Err(SketchExecutionError::ChildPanicked)
        | Err(SketchExecutionError::ChildOutcomes { .. }) => (
            TerminalKind::ChildFailure,
            TerminalDetail::none(),
            "child-failure".into(),
        ),
        Err(error) => (
            TerminalKind::WorkerFailure,
            TerminalDetail::none(),
            error.code().into(),
        ),
    }
}
fn rejections(
    root_outcome: RootOutcome,
    value: kernal_api::wasm::ThreadSpawnRejectionSummary,
) -> TerminalDetail {
    TerminalDetail {
        root_outcome,
        capacity_rejections: value.capacity(),
        closing_rejections: value.closing(),
        fuel_rejections: value.fuel(),
        epoch_rejections: value.epoch(),
        status_code: None,
    }
}

fn snapshot(compiler: &SketchCompiler) -> FinalCounters {
    let value = compiler.execution_limits_snapshot();
    FinalCounters {
        active_roots: value.active_root_executions() as u64,
        live_stores: value.live_stores() as u64,
        live_instances: value.live_instances() as u64,
        active_epoch_registrations: value.active_epoch_registrations() as u64,
        live_threads: value.live_guest_threads() as u64,
    }
}
fn counters_are_zero(value: FinalCounters) -> bool {
    value.active_roots == 0
        && value.live_stores == 0
        && value.live_instances == 0
        && value.active_epoch_registrations == 0
        && value.live_threads == 0
}
fn bound(mut text: String) -> String {
    text.truncate(1024);
    text
}
fn protocol_text(error: ProtocolError) -> String {
    format!("protocol:{error:?}")
}
fn protocol_terminal(
    output: &mut impl io::Write,
    request_id: u64,
    text: &str,
) -> Result<(), String> {
    write_message(
        output,
        &Message::Terminal {
            request_id,
            kind: TerminalKind::ProtocolFailure,
            detail: TerminalDetail::none(),
            diagnostic: bound(text.into()),
            counters: FinalCounters {
                active_roots: 0,
                live_stores: 0,
                live_instances: 0,
                active_epoch_registrations: 0,
                live_threads: 0,
            },
        },
    )
    .map_err(protocol_text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn frame(message: &Message) -> Vec<u8> {
        let mut bytes = Vec::new();
        write_message(&mut bytes, message).unwrap();
        bytes
    }

    #[test]
    fn in_memory_handshake_then_wrong_start_writes_typed_protocol_terminal() {
        let request_id = 41;
        let mut input = frame(&Message::Hello { request_id });
        input.extend(frame(&Message::Cancel { request_id }));
        let mut output = Vec::new();

        assert!(run_with_io(&mut Cursor::new(input), &mut output)
            .unwrap()
            .is_none());

        let mut output = Cursor::new(output);
        assert_eq!(
            read_message(&mut output).unwrap(),
            Message::HelloAck { request_id }
        );
        let Message::Terminal {
            request_id: terminal_request,
            kind,
            detail,
            counters,
            ..
        } = read_message(&mut output).unwrap()
        else {
            panic!("expected terminal");
        };
        assert_eq!(terminal_request, request_id);
        assert_eq!(kind, TerminalKind::ProtocolFailure);
        assert_eq!(detail, TerminalDetail::none());
        assert!(counters_are_zero(counters));
    }

    #[test]
    fn result_mapping_uses_typed_status_and_root_outcome_fields() {
        let (kind, detail, _) = map_result(Ok(ThreadedRootOutcome::Started));
        assert_eq!(kind, TerminalKind::Completed);
        assert_eq!(detail.root_outcome, RootOutcome::Started);
        let (kind, detail, _) = map_result(Err(SketchExecutionError::NonzeroExit { code: -7 }));
        assert_eq!(kind, TerminalKind::NonzeroExit);
        assert_eq!(detail.status_code, Some(-7));
    }
}
