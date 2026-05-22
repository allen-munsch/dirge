use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossterm::ExecutableCommand;
use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};

/// Shared shutdown signal between the input-reader background thread
/// in `ui::mod` and `TerminalGuard::drop`. The reader polls this with
/// each `event::poll` tick; the guard sets it before tearing down so
/// the reader exits its loop cooperatively instead of dying mid-read
/// when the process unwinds. Without this flag the reader stays
/// blocked in `event::read()` while the guard's drain pass is also
/// holding crossterm's internal mutex — the two race for terminal-
/// response bytes (OSC 11, primary DA, CPR). Either path consumes
/// them, but the race is real and the outcome is timing-dependent.
pub(crate) static EVENT_READER_SHUTDOWN: AtomicBool = AtomicBool::new(false);

pub struct TerminalGuard;

impl TerminalGuard {
    pub fn new() -> std::io::Result<Self> {
        // Reset the shutdown flag in case the binary previously held a
        // guard in the same process (test harness, embedded use).
        EVENT_READER_SHUTDOWN.store(false, Ordering::Relaxed);
        let mut stdout = std::io::stdout();
        stdout.execute(EnterAlternateScreen)?;
        stdout.execute(Clear(ClearType::All))?;
        stdout.execute(EnableMouseCapture)?;
        // Bracketed paste lets the terminal deliver a multi-line paste as a
        // single Event::Paste, rather than a flood of keystroke events. The
        // input editor relies on this to compress long pastes into a
        // `[N lines pasted]` placeholder.
        stdout.execute(EnableBracketedPaste)?;
        // Hide the hardware cursor by default. While the agent streams output,
        // the renderer issues many MoveTo calls and the visible cursor would
        // flicker across the screen. draw_bottom re-shows it only after
        // positioning it at the input prompt.
        stdout.execute(Hide)?;
        terminal::enable_raw_mode()?;
        Ok(TerminalGuard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        // Signal the background event-reader thread to exit its loop.
        // It picks this up at the next `event::poll` tick (up to ~50ms),
        // breaks out of its outer loop, and releases crossterm's
        // internal mutex so our drain below can run without contention.
        EVENT_READER_SHUTDOWN.store(true, Ordering::Relaxed);
        let mut stdout = std::io::stdout();
        // The shutdown order matters. Each escape-emitting transition
        // — DisableMouseCapture, DisableBracketedPaste, and especially
        // LeaveAlternateScreen — provokes some terminals (iTerm2,
        // tmux state machines, foot, kitty) to reply with synchronous
        // status bytes: OSC 11 bg-color (`\x1b]11;rgb:…\x1b\\`),
        // primary DA (`\x1b[?64;…c`), and cursor-position reports
        // (`\x1b[…R`). If raw mode is already off when those bytes
        // arrive on stdin, the TTY line discipline echoes them
        // straight to the user's shell prompt as visible garbage.
        //
        // The fix is to keep raw mode on past every escape-emitting
        // transition AND drain after each, then finally disable raw
        // mode last. Previous ordering disabled raw mode BEFORE
        // leaving the alt screen, so the alt-screen-exit's responses
        // always leaked.
        // Note: `Show` was previously sent here, on the alt screen.
        // `LeaveAlternateScreen` (`?1049l`) restores the MAIN screen's
        // saved DECTCEM state, so a Show issued to the alt buffer is
        // discarded by the leave. Moved to after `LeaveAlternateScreen`
        // (review #5).
        let _ = stdout.execute(DisableBracketedPaste);
        let _ = stdout.execute(DisableMouseCapture);
        let _ = stdout.flush();
        // Drain pass 1: catches responses to the three mode-resets
        // above. Start with a long first poll (80ms) to cover the
        // background reader's worst-case 50ms poll latency, then
        // short polls until deadline. Total budget here is ~150ms
        // — slow links (SSH-over-VPN, tmux-in-tmux) need more than
        // the previous 80ms window.
        drain_events(Duration::from_millis(150));
        // NOW leave the alt screen while still in raw mode. Some
        // terminals only emit the bg-color OSC 11 response on this
        // specific transition; leaving alt screen after `disable_raw`
        // was the original leak.
        let _ = stdout.execute(LeaveAlternateScreen);
        let _ = stdout.flush();
        // Drain pass 2: catches responses to LeaveAlternateScreen.
        drain_events(Duration::from_millis(100));

        // Pass 3 — direct stdin sweep. Crossterm's event::poll/read
        // only parses bytes it can convert into known Event variants
        // (keys, mouse, paste, resize). Unrecognized escape sequences
        // — OSC 11 bg-color responses (`\x1b]11;rgb:…`), primary DA
        // (`\x1b[?64;…c`), cursor-position reports (`\x1b[…R`) — are
        // NOT crossterm events and stay sitting in the OS stdin
        // buffer after event::read() returns. If we disable raw
        // mode with those bytes still in the buffer, the shell
        // inherits them and the kernel's TTY line discipline echoes
        // the unprintable-escape payload to the user's prompt.
        //
        // The previous fix (4ebcc66) kept raw mode on past
        // LeaveAlternateScreen so timing was right, but it relied
        // on crossterm to consume the bytes — which it doesn't for
        // these specific sequences. Issue a single libc::read on
        // stdin in non-blocking mode to physically drain the buffer
        // regardless of crossterm's parsing.
        #[cfg(unix)]
        drain_raw_stdin(Duration::from_millis(80));

        // Raw mode last — by now everything the terminal would
        // unsolicit has been parsed and discarded.
        let _ = terminal::disable_raw_mode();
        // Restore cursor visibility AFTER the alt-screen exit so the
        // Show applies to the main screen (the user's shell), not
        // the alt buffer we're about to tear down. Some prompt
        // themes leave the cursor hidden; without this the user
        // sees a missing cursor in their shell.
        let _ = stdout.execute(Show);
        let _ = stdout.flush();
    }
}

/// Drain pending terminal events from crossterm's queue until either
/// nothing is pending or the budget expires. Uses an initial longer
/// poll (covers the background reader's poll latency) followed by
/// short polls. Errors are swallowed — drain is best-effort and the
/// process is exiting either way.
///
/// Early-break policy: only short-circuit on `Ok(false)` AFTER we've
/// already consumed at least one event. The first `Ok(false)` can
/// otherwise mean "the background reader currently owns crossterm's
/// internal mutex on its own poll cycle" rather than "the terminal
/// is quiet" — exiting then would let a delayed response (OSC 11
/// bg-color, primary DA) sneak through after we tear down raw mode.
/// Honoring the full budget on the first quiet poll costs at most
/// the remaining time; that's fine for a shutdown path.
/// Physically drain stdin via direct `libc::read` while in raw mode,
/// discarding any bytes (whether or not crossterm understood them).
/// The crossterm event drain only consumes parseable Events; OSC 11
/// background-color responses, primary DA, and cursor-position
/// reports are unrecognized escape sequences that crossterm leaves
/// in the OS stdin buffer. If those bytes are still there when raw
/// mode flips off, the shell inherits them and the kernel echoes
/// the payload to the user's prompt.
///
/// Implementation:
///   - fd 0 (stdin) flipped to O_NONBLOCK via fcntl
///   - loop: read into a buffer; EWOULDBLOCK = empty, count silently
///   - poll for new arrivals every ~5ms until the budget expires
///     OR two consecutive empty reads (terminal quiesced)
///   - restore the original flags
///
/// Errors are swallowed. The process is exiting; we just want
/// best-effort silence on the shell after we leave.
#[cfg(unix)]
fn drain_raw_stdin(budget: Duration) {
    let fd = 0; // stdin
    // Save original flags so we can restore on exit.
    let original_flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if original_flags < 0 {
        return;
    }
    // Set non-blocking.
    let nb_flags = original_flags | libc::O_NONBLOCK;
    if unsafe { libc::fcntl(fd, libc::F_SETFL, nb_flags) } < 0 {
        return;
    }

    let deadline = std::time::Instant::now() + budget;
    let mut buf = [0u8; 1024];
    let mut empty_polls = 0;
    while std::time::Instant::now() < deadline {
        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n > 0 {
            // Discarded `n` bytes of terminal telemetry — typically
            // OSC 11 reply, DA1, CPR. Count toward "saw activity"
            // for the early-break.
            empty_polls = 0;
            continue;
        }
        if n == 0 {
            // EOF on stdin (rare; pty closed). No more drain to do.
            break;
        }
        // n < 0 → errno. EWOULDBLOCK / EAGAIN means "nothing pending
        // right now"; keep polling until the budget or two
        // consecutive empties. EINTR retries immediately.
        let err = std::io::Error::last_os_error().raw_os_error();
        match err {
            // EAGAIN and EWOULDBLOCK are the same value on macOS /
            // glibc Linux, so match only EAGAIN. EWOULDBLOCK
            // pattern is technically redundant on POSIX but the
            // earlier draft listed both for clarity.
            Some(e) if e == libc::EAGAIN || e == libc::EWOULDBLOCK => {
                empty_polls += 1;
                if empty_polls >= 2 {
                    break;
                }
                // Short sleep so we don't busy-spin while waiting
                // for the next round-trip from a slow terminal.
                std::thread::sleep(Duration::from_millis(5));
            }
            Some(libc::EINTR) => continue,
            _ => break,
        }
    }

    // Restore blocking mode so subsequent stdin readers (the shell)
    // see normal semantics.
    let _ = unsafe { libc::fcntl(fd, libc::F_SETFL, original_flags) };
}

fn drain_events(budget: Duration) {
    let deadline = std::time::Instant::now() + budget;
    let mut first = true;
    let mut saw_event = false;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let wait = if first {
            // First poll absorbs the background reader's worst-case
            // 50ms poll tick + a margin for the terminal round-trip.
            remaining.min(Duration::from_millis(80))
        } else {
            remaining.min(Duration::from_millis(5))
        };
        first = false;
        match event::poll(wait) {
            Ok(true) => {
                saw_event = true;
                if event::read().is_err() {
                    break;
                }
            }
            Ok(false) => {
                // Quiet poll. Only break if we've already consumed
                // at least one event — otherwise keep polling
                // until the deadline; the silence may just mean
                // the reader thread still owned the mutex on this
                // tick.
                if saw_event {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}
