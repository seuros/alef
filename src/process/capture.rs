//! Bounded draining of a child's output pipes.
//!
//! A deadline on the child is not a deadline on its pipes: the child hands its stdout and stderr
//! to every descendant it starts, so a reader that waits for end of stream waits for the longest
//! lived descendant, not for the command. Everything here exists to put a ceiling on that wait.

use std::io::Read;

/// How long the output pipes may still be drained once the child itself is no longer running.
///
/// A timeout bounds the *command*, and the child inherits its stdout and stderr to every
/// descendant it starts. A descendant that outlives the command -- a Gradle daemon, an MSBuild
/// node, anything a hook backgrounded and never waited on -- keeps the write end of those pipes
/// open, so a `read_to_string` that waits for end of stream waits for that descendant, not for
/// the command. That is how a hook under a 1800s budget ran for over half an hour and was never
/// killed: the wait had already succeeded, and the unbounded drain that followed it answered to
/// nothing. Everything after the child exits is a leaked writer, so it gets a fixed grace to
/// flush what is already buffered and is then torn down with the rest of the group. ~keep
pub(crate) const OUTPUT_DRAIN_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// How much of a stream is moved into the shared buffer per read. Large enough that a megabyte of
/// compiler output costs tens of locks rather than thousands. ~keep
const OUTPUT_CHUNK_BYTES: usize = 16 * 1024;

/// A detached reader thread and the channel that reports it reached end of stream.
pub(crate) struct StreamDrain {
    finished: std::sync::mpsc::Receiver<std::io::Result<()>>,
}

/// Runs `read_to_end_of_stream` on a thread that is deliberately never joined.
///
/// When a leaked descendant holds the write end open the read never returns, and joining it is
/// exactly the unbounded wait [`OUTPUT_DRAIN_GRACE`] exists to stop. The thread exits on its own
/// once the last writer closes. ~keep
pub(crate) fn spawn_drain<F>(read_to_end_of_stream: F) -> StreamDrain
where
    F: FnOnce() -> std::io::Result<()> + Send + 'static,
{
    let (sender, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(read_to_end_of_stream());
    });
    StreamDrain { finished }
}

/// Waits for every drain in `drains` to reach end of stream, sharing one `budget` between them.
///
/// `Ok(false)` means a writer still held one of the pipes when the budget ran out -- the caller's
/// cue that a descendant outlived the command and the process group needs tearing down.
///
/// # Errors
///
/// Returns an error when a stream failed to read or its reader thread died without reporting.
pub(crate) fn wait_for_drains<'a>(
    drains: impl IntoIterator<Item = &'a StreamDrain>,
    budget: std::time::Duration,
) -> std::io::Result<bool> {
    let deadline = std::time::Instant::now() + budget;
    let mut complete = true;
    for drain in drains {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match drain.finished.recv_timeout(remaining) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Err(error),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => complete = false,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(std::io::Error::other("output reader thread panicked"));
            }
        }
    }
    Ok(complete)
}

/// One drained stream. The bytes live behind a lock the reader thread appends to as they arrive,
/// rather than in a `String` the thread only hands over at end of stream, so a drain that gives up
/// on a stuck pipe still returns everything the command actually wrote. ~keep
pub(crate) struct OutputReader {
    buffer: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    drain: StreamDrain,
}

/// A command's output, and whether every stream reached end of stream before the drain budget ran
/// out.
pub(crate) struct DrainedOutput {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) complete: bool,
}

pub(crate) fn output_reader(mut stream: impl Read + Send + 'static) -> OutputReader {
    let buffer = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = std::sync::Arc::clone(&buffer);
    let drain = spawn_drain(move || {
        let mut chunk = [0_u8; OUTPUT_CHUNK_BYTES];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => break Ok(()),
                Ok(count) => lock(&sink).extend_from_slice(&chunk[..count]),
                Err(error) => break Err(error),
            }
        }
    });
    OutputReader { buffer, drain }
}

fn lock(buffer: &std::sync::Mutex<Vec<u8>>) -> std::sync::MutexGuard<'_, Vec<u8>> {
    buffer.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Drains both streams, giving up `budget` after the first byte is waited on.
///
/// Output is decoded lossily: a toolchain that emits a stray non-UTF-8 byte reports a broken
/// snippet, and losing the whole diagnostic to a decode error helps nobody. ~keep
///
/// # Errors
///
/// Returns an error when a stream failed to read or its reader thread died without reporting.
pub(crate) fn collect_output_within(
    stdout: Option<OutputReader>,
    stderr: Option<OutputReader>,
    budget: std::time::Duration,
) -> std::io::Result<DrainedOutput> {
    let readers = [stdout, stderr];
    let complete = wait_for_drains(readers.iter().flatten().map(|reader| &reader.drain), budget)?;
    let mut streams = readers
        .into_iter()
        .map(|reader| reader.map_or_else(String::new, |reader| decode(&reader)));
    Ok(DrainedOutput {
        stdout: streams.next().unwrap_or_default(),
        stderr: streams.next().unwrap_or_default(),
        complete,
    })
}

fn decode(reader: &OutputReader) -> String {
    String::from_utf8_lossy(&lock(&reader.buffer)).into_owned()
}

#[cfg(test)]
mod tests {
    use super::{OUTPUT_DRAIN_GRACE, collect_output_within, output_reader, spawn_drain, wait_for_drains};
    use std::time::{Duration, Instant};

    #[test]
    fn a_stream_that_reaches_end_of_stream_reports_the_drain_complete() {
        let reader = output_reader(std::io::Cursor::new(b"finished".to_vec()));

        let drained = collect_output_within(Some(reader), None, OUTPUT_DRAIN_GRACE).expect("a readable stream");

        assert!(drained.complete);
        assert_eq!(drained.stdout, "finished");
        assert_eq!(drained.stderr, "");
    }

    /// The bound the whole module exists for, measured rather than asserted structurally: a reader
    /// that never reaches end of stream must give the budget back, not the caller's whole run. ~keep
    #[test]
    fn a_stream_that_never_ends_gives_the_budget_back_and_reports_incomplete() {
        let (holder, receiver) = std::sync::mpsc::channel::<()>();
        let stalled = spawn_drain(move || {
            let _ = receiver.recv();
            Ok(())
        });
        let budget = Duration::from_millis(200);
        let started = Instant::now();

        let complete = wait_for_drains([&stalled], budget).expect("a stalled drain is not an error");
        let elapsed = started.elapsed();

        assert!(!complete, "a stream with a live writer must not report complete");
        assert!(elapsed >= budget, "the drain returned before its budget elapsed");
        assert!(
            elapsed < budget * 10,
            "the drain took {elapsed:?}, which is not bounded by its {budget:?} budget"
        );
        drop(holder);
    }

    /// One budget covers every stream, so two stalled pipes cost the same wall clock as one. ~keep
    #[test]
    fn two_stalled_streams_share_a_single_budget() {
        let mut holders = Vec::new();
        let mut drains = Vec::new();
        for _ in 0..2 {
            let (holder, receiver) = std::sync::mpsc::channel::<()>();
            holders.push(holder);
            drains.push(spawn_drain(move || {
                let _ = receiver.recv();
                Ok(())
            }));
        }
        let budget = Duration::from_millis(200);
        let started = Instant::now();

        let complete = wait_for_drains(drains.iter(), budget).expect("stalled drains are not an error");
        let elapsed = started.elapsed();

        assert!(!complete);
        assert!(
            elapsed < budget * 2,
            "two stalled streams took {elapsed:?}, so the budget was charged per stream"
        );
        drop(holders);
    }
}
