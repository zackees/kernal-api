//! Host-neutral streaming/reaping prerequisites for the compiler guest grant.
//! This exercises the native facade, not the unfinished Wasm cache workflow.

use std::io::Write;
use std::time::Duration;

use kernal_api::{
    ProcessOutputChunk, ProcessOutputCompletion, ProcessOutputEvent, ProcessPostExitDrain,
    ProcessSession, ProcessSessionOptions, SpawnSpec, StreamMode,
};

const HELPER_ENV: &str = "KERNAL_API_STREAMING_PROCESS_HELPER";
const STREAM_BYTES: usize = 32 * 1024 * 1024;
const CHUNK_BYTES: usize = 4096;

#[test]
fn streaming_process_helper() {
    if std::env::var_os(HELPER_ENV).is_none() {
        return;
    }
    // Non-ASCII bytes distinguish payload from the child libtest harness.
    // Both pipes exceed OS buffering; no shell or platform utility is needed.
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    for _ in 0..STREAM_BYTES / CHUNK_BYTES {
        stdout.write_all(&[0xf1; CHUNK_BYTES]).unwrap();
        stderr.write_all(&[0xf2; CHUNK_BYTES]).unwrap();
    }
    stdout.flush().unwrap();
    stderr.flush().unwrap();
}

async fn session() -> ProcessSession {
    SpawnSpec::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("streaming_process_helper")
        .arg("--nocapture")
        .env(HELPER_ENV, "1")
        .stdin(StreamMode::Null)
        .stdout(StreamMode::Piped)
        .stderr(StreamMode::Piped)
        .spawn_session(ProcessSessionOptions {
            max_queued_chunks: 1,
            max_chunk_bytes: CHUNK_BYTES,
            post_exit_drain: ProcessPostExitDrain::AbandonAfter(Duration::from_secs(1)),
            kill_on_drop: true,
        })
        .await
        .unwrap()
}

fn runtime() -> kernal_api::async_engine::Runtime {
    kernal_api::async_engine::RuntimeBuilder::current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn tagged_output_streams_64_mib_without_whole_output_collection() {
    runtime().run(async {
        kernal_api::async_engine::timeout(Duration::from_secs(30), async {
            let session = session().await;
            let mut counts = [0usize; 2];
            let mut eof = [false; 2];
            while let Some(event) = session.next_output().await {
                match event {
                    ProcessOutputEvent::Chunk(chunk) => {
                        assert!(chunk.bytes().len() <= CHUNK_BYTES);
                        let (stream, payload) = match &chunk {
                            ProcessOutputChunk::Stdout(_) => (0, 0xf1),
                            ProcessOutputChunk::Stderr(_) => (1, 0xf2),
                        };
                        assert!(!eof[stream], "payload after stream completion");
                        for byte in chunk.bytes() {
                            if *byte == payload {
                                counts[stream] += 1;
                            } else {
                                assert_eq!(stream, 0, "unexpected stderr payload");
                                assert!(byte.is_ascii(), "wrong-stream payload");
                            }
                        }
                        assert!(counts[stream] <= STREAM_BYTES);
                    }
                    ProcessOutputEvent::Completion(completion) => {
                        let stream = match completion {
                            ProcessOutputCompletion::StdoutEof => 0,
                            ProcessOutputCompletion::StderrEof => 1,
                            other => panic!("unexpected output completion: {other:?}"),
                        };
                        assert!(!eof[stream], "duplicate completion");
                        eof[stream] = true;
                    }
                }
            }
            assert_eq!(counts, [STREAM_BYTES; 2]);
            assert_eq!(eof, [true; 2]);
            assert_eq!(session.wait().await.unwrap().exit_code(), Some(0));
        })
        .await
        .expect("bounded-output fixture deadline");
    });
}

#[test]
fn kill_reaps_without_draining_backpressured_output() {
    runtime().run(async {
        let session = session().await;
        kernal_api::async_engine::timeout(Duration::from_secs(10), async {
            // Wait for actual fixture output rather than racing process startup.
            loop {
                let event = session.next_output().await.expect("fixture output");
                if let ProcessOutputEvent::Chunk(chunk) = event {
                    if chunk.bytes().iter().any(|byte| matches!(byte, 0xf1 | 0xf2)) {
                        break;
                    }
                }
            }
            // Stop receiving: the one-chunk queue and pipes cannot retain the
            // fixture's remaining output. Lifecycle control must stay usable.
            kernal_api::async_engine::sleep(Duration::from_millis(50)).await;
            assert!(session.poll().await.unwrap().is_none());
            session.kill().await.unwrap();
            assert!(session.poll().await.unwrap().is_some());
            assert_ne!(session.wait().await.unwrap().exit_code(), Some(0));
        })
        .await
        .expect("kill and reap must not require output delivery");
    });
}
