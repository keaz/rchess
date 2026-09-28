//! Event collection for the main loop (spec 6.5). The app sees only
//! [`AppEvent`]s: terminal input, engine replies, and one `Tick` per batch.

use std::io;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use ratatui::crossterm::event::{self, Event};

use super::worker::EngineReply;

/// Upper bound on terminal events read in one batch, so a flood of input (a held
/// key, a long drag) cannot starve redraws and engine replies.
const MAX_TERM_EVENTS: usize = 256;

/// Everything the app reacts to.
#[derive(Clone, Debug, PartialEq)]
pub enum AppEvent {
    /// A terminal event: key, mouse, paste, resize or focus change.
    Term(Event),
    /// A reply from an engine thread; the app checks it with `worker::is_current`.
    Engine(EngineReply),
    /// Ends every batch: time has passed (spinner, Jev vs Jev step delay, signals).
    Tick,
}

/// One batch of events: every pending engine reply, then the terminal events that
/// arrive within `timeout` (all of those already available, without waiting any
/// further), then exactly one [`AppEvent::Tick`].
///
/// When an engine reply is pending the terminal is only checked, not waited on,
/// so the reply is shown without an extra `timeout` of delay.
///
/// # Errors
///
/// When crossterm cannot poll or read the terminal; the batch is lost and the
/// app should shut down.
pub fn collect(rx: &Receiver<EngineReply>, timeout: Duration) -> io::Result<Vec<AppEvent>> {
    collect_from(&mut Crossterm, rx, timeout)
}

/// Every engine reply already waiting on `rx`, oldest first. Never blocks.
#[must_use]
pub fn drain_engine(rx: &Receiver<EngineReply>) -> Vec<AppEvent> {
    rx.try_iter().map(AppEvent::Engine).collect()
}

/// Where terminal events come from: crossterm in the app, a script in tests.
trait TermSource {
    /// True when an event can be read without blocking, waiting up to `timeout`.
    fn poll(&mut self, timeout: Duration) -> io::Result<bool>;
    /// The next event; only called after `poll` returned true.
    fn read(&mut self) -> io::Result<Event>;
}

/// The real terminal, through crossterm's global event reader.
struct Crossterm;

impl TermSource for Crossterm {
    fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        event::poll(timeout)
    }

    fn read(&mut self) -> io::Result<Event> {
        event::read()
    }
}

fn collect_from(
    source: &mut impl TermSource,
    rx: &Receiver<EngineReply>,
    timeout: Duration,
) -> io::Result<Vec<AppEvent>> {
    let mut events = drain_engine(rx);
    let wait = if events.is_empty() {
        timeout
    } else {
        Duration::ZERO
    };
    if source.poll(wait)? {
        events.push(AppEvent::Term(source.read()?));
        let mut read = 1;
        while read < MAX_TERM_EVENTS && source.poll(Duration::ZERO)? {
            events.push(AppEvent::Term(source.read()?));
            read += 1;
        }
    }
    events.push(AppEvent::Tick);
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::mpsc;

    use ratatui::crossterm::event::KeyCode;

    use crate::tui::test_support::key_event;
    use crate::tui::worker::EngineOutcome;

    /// Serves queued events and records every poll timeout it was given.
    #[derive(Default)]
    struct Script {
        pending: VecDeque<io::Result<Event>>,
        polls: Vec<Duration>,
    }

    impl Script {
        fn with(events: impl IntoIterator<Item = Event>) -> Script {
            Script {
                pending: events.into_iter().map(Ok).collect(),
                polls: Vec::new(),
            }
        }
    }

    impl TermSource for Script {
        fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
            self.polls.push(timeout);
            Ok(!self.pending.is_empty())
        }

        fn read(&mut self) -> io::Result<Event> {
            self.pending
                .pop_front()
                .expect("read is only called after a successful poll")
        }
    }

    const TIMEOUT: Duration = Duration::from_millis(50);

    fn reply(generation: u64) -> EngineReply {
        EngineReply {
            generation,
            hash: 0,
            outcome: EngineOutcome::GameOver,
            exchange: None,
        }
    }

    fn key(c: char) -> Event {
        key_event(KeyCode::Char(c))
    }

    #[test]
    fn drain_engine_returns_all_pending_replies_in_order() {
        let (tx, rx) = mpsc::channel();
        for generation in 1..=3 {
            tx.send(reply(generation)).unwrap();
        }
        assert_eq!(
            drain_engine(&rx),
            vec![
                AppEvent::Engine(reply(1)),
                AppEvent::Engine(reply(2)),
                AppEvent::Engine(reply(3)),
            ]
        );
        assert!(drain_engine(&rx).is_empty(), "the channel is now empty");
    }

    #[test]
    fn drain_engine_does_not_block_on_an_empty_or_closed_channel() {
        let (tx, rx) = mpsc::channel::<EngineReply>();
        assert!(drain_engine(&rx).is_empty());
        tx.send(reply(1)).unwrap();
        drop(tx);
        assert_eq!(drain_engine(&rx), vec![AppEvent::Engine(reply(1))]);
        assert!(drain_engine(&rx).is_empty());
    }

    #[test]
    fn quiet_batch_waits_the_full_timeout_and_ends_with_a_tick() {
        let (_tx, rx) = mpsc::channel();
        let mut source = Script::default();
        let events = collect_from(&mut source, &rx, TIMEOUT).unwrap();
        assert_eq!(events, vec![AppEvent::Tick]);
        assert_eq!(source.polls, vec![TIMEOUT]);
    }

    #[test]
    fn engine_replies_come_first_then_terminal_events_then_one_tick() {
        let (tx, rx) = mpsc::channel();
        tx.send(reply(1)).unwrap();
        tx.send(reply(2)).unwrap();
        let mut source = Script::with([key('e'), key('4'), Event::Resize(80, 24)]);

        let events = collect_from(&mut source, &rx, TIMEOUT).unwrap();

        assert_eq!(
            events,
            vec![
                AppEvent::Engine(reply(1)),
                AppEvent::Engine(reply(2)),
                AppEvent::Term(key('e')),
                AppEvent::Term(key('4')),
                AppEvent::Term(Event::Resize(80, 24)),
                AppEvent::Tick,
            ]
        );
    }

    #[test]
    fn a_pending_reply_skips_the_wait() {
        let (tx, rx) = mpsc::channel();
        tx.send(reply(1)).unwrap();
        let mut source = Script::default();
        collect_from(&mut source, &rx, TIMEOUT).unwrap();
        assert_eq!(source.polls, vec![Duration::ZERO]);
    }

    #[test]
    fn only_the_first_poll_waits() {
        let (_tx, rx) = mpsc::channel();
        let mut source = Script::with([key('a'), key('b')]);
        collect_from(&mut source, &rx, TIMEOUT).unwrap();
        assert_eq!(
            source.polls,
            vec![TIMEOUT, Duration::ZERO, Duration::ZERO],
            "reads everything available, then stops without waiting again"
        );
    }

    #[test]
    fn a_flood_of_input_is_split_across_batches() {
        let (_tx, rx) = mpsc::channel();
        let mut source = Script::with((0..MAX_TERM_EVENTS + 10).map(|_| key('x')));

        let first = collect_from(&mut source, &rx, TIMEOUT).unwrap();
        assert_eq!(first.len(), MAX_TERM_EVENTS + 1);
        assert_eq!(first.last(), Some(&AppEvent::Tick));

        let second = collect_from(&mut source, &rx, TIMEOUT).unwrap();
        assert_eq!(second.len(), 11);
    }

    #[test]
    fn terminal_errors_are_returned() {
        let (_tx, rx) = mpsc::channel();
        let mut source = Script {
            pending: VecDeque::from([Err(io::Error::other("tty gone"))]),
            polls: Vec::new(),
        };
        let err = collect_from(&mut source, &rx, TIMEOUT).unwrap_err();
        assert_eq!(err.to_string(), "tty gone");
    }
}
