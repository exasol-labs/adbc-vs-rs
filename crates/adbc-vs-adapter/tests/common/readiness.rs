//! Bounded waits for a part of the stack to become usable.
//!
//! Every wait here ends one of two ways: the thing became ready, or the run
//! fails. There is deliberately no third outcome — no return value, no
//! environment variable, and no configuration turns an unavailable container
//! into a skipped or a quietly passing test. A wait that cannot be satisfied
//! panics, and the panic names what was waited for and how long it was given.

use std::net::{TcpStream, ToSocketAddrs};
use std::thread;
use std::time::{Duration, Instant};

/// How long a container of the stack has to start answering.
///
/// Generous because `exasol/docker-db` needs minutes from a cold volume, and a
/// deadline shorter than a legitimate start-up would turn a healthy stack into
/// a failing run.
pub const SERVICE_DEADLINE: Duration = Duration::from_secs(300);

/// How long BucketFS has to make a written object, or the extracted contents
/// of a written archive, readable to a UDF.
pub const BUCKETFS_DEADLINE: Duration = Duration::from_secs(300);

/// How often a bounded wait re-checks.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// How long one connection attempt may hang before it counts as "not yet".
///
/// A port that accepts the TCP connection but never completes it would
/// otherwise consume the whole deadline in a single attempt.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// Waits until `ready` answers true, failing the run at `deadline`.
///
/// `subject` names what is being waited for, in the panic message a timeout
/// raises.
pub fn wait_until(subject: &str, deadline: Duration, mut ready: impl FnMut() -> bool) {
    let started = Instant::now();
    loop {
        if ready() {
            return;
        }

        let elapsed = started.elapsed();
        assert!(elapsed < deadline, "{}", timeout_message(subject, deadline));
        thread::sleep(POLL_INTERVAL.min(deadline - elapsed));
    }
}

/// Waits until `service` accepts a connection at `address`, failing the run at
/// `deadline`.
///
/// `address` is a `host:port` pair. Accepting a connection is the weakest
/// readiness a service can show, so callers that need more — a database that
/// answers a query rather than merely listening — wait again on that.
pub fn wait_for_service(service: &str, address: &str, deadline: Duration) {
    wait_until(&format!("{service} at {address}"), deadline, || {
        accepts_connection(address)
    });
}

fn accepts_connection(address: &str) -> bool {
    let Ok(resolved) = address.to_socket_addrs() else {
        return false;
    };
    resolved.into_iter().any(|socket| {
        TcpStream::connect_timeout(&socket, CONNECT_TIMEOUT)
            .is_ok_and(|connection| connection.shutdown(std::net::Shutdown::Both).is_ok())
    })
}

fn timeout_message(subject: &str, deadline: Duration) -> String {
    format!("{subject} did not become ready within {deadline:?}")
}

#[cfg(test)]
mod tests {
    use std::any::Any;
    use std::cell::Cell;
    use std::net::TcpListener;
    use std::panic;

    use super::*;

    /// A deadline long enough to survive a scheduling hiccup and short enough
    /// that a test which means to reach it costs a fraction of a second.
    const TEST_DEADLINE: Duration = Duration::from_millis(300);

    /// An address on the loopback interface that nothing listens on: a port
    /// the operating system just handed out, released before it is used.
    fn closed_address() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("the loopback interface binds");
        listener
            .local_addr()
            .expect("a bound listener has a local address")
            .to_string()
    }

    fn panic_message(payload: &(dyn Any + Send)) -> String {
        if let Some(owned) = payload.downcast_ref::<String>() {
            return owned.clone();
        }
        if let Some(borrowed) = payload.downcast_ref::<&str>() {
            return (*borrowed).to_string();
        }
        panic!("the wait panicked without a message");
    }

    #[test]
    fn readiness_helper_panics_on_unreachable_endpoint() {
        let address = closed_address();

        let payload = panic::catch_unwind(|| wait_for_service("mysql", &address, TEST_DEADLINE))
            .expect_err("waiting on an endpoint nothing listens on must fail the run");

        let message = panic_message(payload.as_ref());
        assert!(
            message.contains("mysql"),
            "the panic must name the service waited for: {message}"
        );
        assert!(
            message.contains("300ms"),
            "the panic must name the deadline that elapsed: {message}"
        );
        assert!(
            message.contains(&address),
            "the panic must name the endpoint that stayed closed: {message}"
        );
    }

    #[test]
    fn wait_for_service_returns_once_the_endpoint_accepts() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("the loopback interface binds");
        let address = listener
            .local_addr()
            .expect("a bound listener has a local address")
            .to_string();

        wait_for_service("a listening endpoint", &address, TEST_DEADLINE);
    }

    #[test]
    fn wait_until_returns_without_sleeping_when_the_condition_already_holds() {
        let started = Instant::now();

        wait_until("an already-ready subject", TEST_DEADLINE, || true);

        assert!(
            started.elapsed() < POLL_INTERVAL,
            "a satisfied condition must not cost a poll interval"
        );
    }

    #[test]
    fn wait_until_returns_once_the_condition_starts_holding() {
        let attempts = Cell::new(0);

        wait_until("a subject that becomes ready", TEST_DEADLINE, || {
            attempts.set(attempts.get() + 1);
            attempts.get() > 1
        });

        assert_eq!(attempts.get(), 2, "the wait must re-check after a poll");
    }

    #[test]
    fn wait_until_panics_within_its_deadline_when_the_condition_never_holds() {
        let started = Instant::now();

        let payload = panic::catch_unwind(|| wait_until("a subject", TEST_DEADLINE, || false))
            .expect_err("a condition that never holds must fail the run");

        assert!(
            panic_message(payload.as_ref()).contains("a subject"),
            "the panic must name the subject waited for"
        );
        assert!(
            started.elapsed() < TEST_DEADLINE * 4,
            "the wait must give up at its deadline, not run on"
        );
    }

    #[test]
    fn timeout_message_names_the_subject_and_the_deadline() {
        assert_eq!(
            timeout_message("exasol at localhost:8563", Duration::from_secs(300)),
            "exasol at localhost:8563 did not become ready within 300s"
        );
    }

    #[test]
    fn a_closed_address_accepts_nothing() {
        assert!(!accepts_connection(&closed_address()));
    }

    #[test]
    fn an_unresolvable_address_accepts_nothing() {
        assert!(!accepts_connection("not a host:port pair"));
    }
}
