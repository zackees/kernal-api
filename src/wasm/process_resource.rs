//! One-shot native compiler grants and cancellation-safe direct-child cleanup.
//! ABI adapters must use this authority rather than owning native supervisors.

use super::*;
use crate::async_engine::{CancellationSource, CancellationToken, RuntimeHandle};
use crate::{ProcessSession, ProcessSessionOptions, SpawnSpec, StreamMode};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

const GRANT_KIND: u8 = 10;
const PROCESS_KIND: u8 = 11;
const PROCESS_RIGHT: u8 = 1;
const MAX_PROCESS_JOBS: usize = 4;
const MAX_PROCESS_OUTPUT_CHUNK: usize = 64 * 1024;
const MAX_PROCESS_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

#[path = "process_output.rs"]
mod output;

#[cfg(test)]
pub(super) struct SpawnCheckpoint {
    started: crate::async_engine::OneshotSender<Arc<ProcessSession>>,
    resume: crate::async_engine::OneshotReceiver<()>,
}

pub(super) struct CompilerGrant {
    spec: Option<SpawnSpec>,
    deadline: Duration,
}

pub(super) struct CompilerProcess {
    session: Option<Arc<ProcessSession>>,
    cancel: CancellationSource,
    output_busy: bool,
    output_bytes: usize,
    output_limit: usize,
}

impl Drop for CompilerProcess {
    fn drop(&mut self) {
        // Revocation can run under the authority mutex. Only signal here;
        // the tracked job performs process I/O and reaping outside that lock.
        self.cancel.cancel();
    }
}

impl OperationHub {
    pub(crate) fn grant_compiler(
        &self,
        store: u64,
        spec: SpawnSpec,
        deadline: Duration,
    ) -> Result<OpaqueToken, HubError> {
        // Host-owned, exact paths and environment; no ambient lookup or cwd.
        if !Path::new(&spec.program).is_absolute()
            || !spec
                .current_dir
                .as_ref()
                .is_some_and(|path| path.is_absolute())
            || !spec.clear_env
            || spec
                .lifetime_owner
                .is_some_and(|owner| owner != crate::platform::process::LifetimeOwner::Spawner)
            || deadline.is_zero()
            || deadline > Duration::from_secs(300)
        {
            return Err(HubError::Invalid);
        }
        let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
        let token = self.create_resource_value_locked(
            &mut state,
            store,
            GRANT_KIND,
            PROCESS_RIGHT,
            false,
            ResourceValue::CompilerGrant(CompilerGrant {
                spec: Some(
                    spec.stdin(StreamMode::Null)
                        .stdout(StreamMode::Piped)
                        .stderr(StreamMode::Piped)
                        .kill_when_owner_dies(true),
                ),
                deadline,
            }),
        )?;
        state
            .resources
            .get_mut(&token)
            .ok_or(HubError::Closed)?
            .reserved = false;
        Ok(token)
    }

    pub(crate) fn submit_compiler_spawn(
        self: &Arc<Self>,
        runtime: RuntimeHandle,
        store: u64,
        grant: OpaqueToken,
    ) -> Result<OpaqueToken, HubError> {
        let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
        if state.closed {
            return Err(HubError::Closed);
        }
        Self::poll_process_jobs(&mut state, &mut Context::from_waker(Waker::noop()));
        if state.process_job_failed {
            return Err(HubError::Closed);
        }
        if state.process_jobs.len() >= MAX_PROCESS_JOBS.min(self.maximum_resources) {
            return Err(HubError::Quota);
        }
        let slot = state.resources.get(&grant).ok_or(HubError::Invalid)?;
        Self::validate_resource(slot, store, GRANT_KIND, PROCESS_RIGHT)?;
        let ResourceValue::CompilerGrant(CompilerGrant { spec: Some(_), .. }) = &slot.value else {
            return Err(HubError::Closed);
        };
        let (operation, _) =
            self.submit_locked(&mut state, store, Some(grant), GRANT_KIND, PROCESS_RIGHT)?;
        let cancel = CancellationSource::new();
        let cancellation = cancel.token();
        let process = match self.create_resource_value_locked(
            &mut state,
            store,
            PROCESS_KIND,
            PROCESS_RIGHT,
            false,
            ResourceValue::CompilerProcess(CompilerProcess {
                session: None,
                cancel,
                output_busy: false,
                output_bytes: 0,
                output_limit: MAX_PROCESS_OUTPUT_BYTES,
            }),
        ) {
            Ok(token) => token,
            Err(error) => {
                state.operations.remove(&operation);
                return Err(error);
            }
        };
        state
            .operations
            .get_mut(&operation)
            .ok_or(HubError::Closed)?
            .created_resource = Some(process);
        state
            .operations
            .get_mut(&operation)
            .ok_or(HubError::Closed)?
            .is_compiler_spawn = true;
        let ResourceValue::CompilerGrant(grant) = &mut state
            .resources
            .get_mut(&grant)
            .ok_or(HubError::Closed)?
            .value
        else {
            return Err(HubError::WrongKind);
        };
        // Consume only after all quotas have been reserved. Never resurrect a
        // command whose launch may have begun, including native spawn errors.
        let spec = grant.spec.take().ok_or(HubError::Closed)?;
        let deadline = grant.deadline;
        let admitted = Instant::now();
        let hub = Arc::clone(self);
        let job = runtime.launch(async move {
            hub.supervise_compiler(spec, operation, process, cancellation, admitted, deadline)
                .await
        });
        state.process_jobs.push(job);
        Ok(operation)
    }

    async fn supervise_compiler(
        self: Arc<Self>,
        spec: SpawnSpec,
        operation: OpaqueToken,
        process: OpaqueToken,
        cancellation: CancellationToken,
        admitted: Instant,
        deadline: Duration,
    ) -> Result<(), HubError> {
        if cancellation.is_cancelled() {
            return Ok(());
        }
        if admitted.elapsed() >= deadline {
            let _ = self.terminal(
                operation,
                TerminalResult {
                    terminal: Terminal::TimedOut,
                    resource: None,
                },
            );
            return Ok(());
        }
        // Do not drop a spawning future: if revocation races native creation,
        // retain responsibility for any returned child and explicitly reap it.
        // An uninterruptible native spawn is not claimed to be contained here.
        #[cfg(test)]
        self.process_spawn_attempts.fetch_add(1, Ordering::SeqCst);
        let session = match spec
            .spawn_session(ProcessSessionOptions {
                max_queued_chunks: 1,
                max_chunk_bytes: MAX_PROCESS_OUTPUT_CHUNK,
                kill_on_drop: true,
                ..ProcessSessionOptions::default()
            })
            .await
        {
            Ok(session) => Arc::new(session),
            Err(_) => {
                let _ = self.terminal(
                    operation,
                    TerminalResult {
                        terminal: Terminal::Rejected,
                        resource: None,
                    },
                );
                return Ok(());
            }
        };
        #[cfg(test)]
        {
            let checkpoint = {
                let mut checkpoint = self.process_spawn_checkpoint.lock().unwrap();
                checkpoint.take()
            };
            if let Some(checkpoint) = checkpoint {
                let _ = checkpoint.started.send(Arc::clone(&session));
                let _ = checkpoint.resume.await;
            }
        }
        let published = if admitted.elapsed() >= deadline {
            let _ = self.terminal(
                operation,
                TerminalResult {
                    terminal: Terminal::TimedOut,
                    resource: None,
                },
            );
            Err(HubError::Closed)
        } else {
            self.publish_compiler(operation, process, Arc::clone(&session))
        };
        if published.is_ok() {
            let remaining = deadline.saturating_sub(admitted.elapsed());
            if crate::async_engine::timeout(remaining, cancellation.cancelled())
                .await
                .is_err()
            {
                let _ = self.revoke_external_resource(process, Terminal::TimedOut);
            }
        }
        // Revocation, failed publication, and deadline all use the same reaper.
        // This is direct-child cleanup, never a descendant-tree guarantee.
        session.kill().await.map_err(|_| HubError::Closed)?;
        session.wait().await.map_err(|_| HubError::Closed)?;
        Ok(())
    }

    fn publish_compiler(
        &self,
        operation: OpaqueToken,
        process: OpaqueToken,
        session: Arc<ProcessSession>,
    ) -> Result<(), HubError> {
        let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
        if state.closed
            || state
                .operations
                .get(&operation)
                .is_none_or(|op| op.terminal.is_some())
        {
            return Err(HubError::Closed);
        }
        let slot = state.resources.get_mut(&process).ok_or(HubError::Closed)?;
        let ResourceValue::CompilerProcess(value) = &mut slot.value else {
            return Err(HubError::WrongKind);
        };
        value.session = Some(session);
        let notify = Self::terminal_locked(
            &mut state,
            operation,
            TerminalResult {
                terminal: Terminal::Completed,
                resource: Some(process),
            },
        )?;
        drop(state);
        if let Some(notify) = notify {
            notify.notify_one();
        }
        Ok(())
    }

    pub(crate) fn abandon_compiler_spawn(
        &self,
        store: u64,
        operation: OpaqueToken,
    ) -> Result<(), HubError> {
        let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
        let op = state.operations.get(&operation).ok_or(HubError::Invalid)?;
        if op.owner.store != store {
            return Err(HubError::Stale);
        }
        if !op.is_compiler_spawn {
            return Err(HubError::WrongKind);
        }
        let process = op.created_resource.ok_or(HubError::WrongKind)?;
        let op = state
            .operations
            .remove(&operation)
            .ok_or(HubError::Invalid)?;
        let notifications = if state.resources.contains_key(&process) {
            Self::close_resource_with_terminal_locked(&mut state, process, Terminal::Closed)?
        } else {
            Vec::new()
        };
        drop(state);
        op.notify.notify_one();
        for notify in notifications {
            notify.notify_one();
        }
        Ok(())
    }

    fn poll_process_jobs(state: &mut State, context: &mut Context<'_>) {
        state
            .process_jobs
            .retain_mut(|job| match Pin::new(job).poll(context) {
                Poll::Pending => true,
                Poll::Ready(Ok(Ok(()))) => false,
                Poll::Ready(_) => {
                    state.process_job_failed = true;
                    false
                }
            });
    }

    pub(crate) async fn join_process_jobs(&self) -> Result<(), HubError> {
        // A Task has one join waker. Serialize observers without taking its
        // handle out of State; cancellation releases this permit, not the job.
        let _join = self.process_join.acquire().await;
        std::future::poll_fn(|context| {
            let mut state = match self.state.lock() {
                Ok(state) => state,
                Err(_) => return Poll::Ready(Err(HubError::Closed)),
            };
            if !state.closed {
                return Poll::Ready(Err(HubError::WrongRights));
            }
            Self::poll_process_jobs(&mut state, context);
            if !state.process_jobs.is_empty() {
                return Poll::Pending;
            }
            Poll::Ready(if state.process_job_failed {
                Err(HubError::Closed)
            } else {
                Ok(())
            })
        })
        .await
    }
}

#[cfg(test)]
#[path = "process_resource_tests.rs"]
mod tests;
