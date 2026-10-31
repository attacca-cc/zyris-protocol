//! `terminal.poll` — the node-side counterpart of Attacca's `computer_terminal_poll`.
//!
//! Every test drives a real client and a real `PtyTerminal` through the testing duplex, because
//! what is worth pinning down here is the loop's behaviour against actual processes: how many
//! times it ran, what it kept, and what it did when an attempt hung.

use std::time::{Duration, Instant};

use zyris::{Node, NodeKind};
use zyris_caps::{PollResult, Terminal, TerminalClient, TerminalServer};
use zyris_capkit::PtyTerminal;

async fn connect() -> TerminalClient {
    let server = Node::builder()
        .name("poll-node")
        .kind(NodeKind::Service)
        .capability(TerminalServer(PtyTerminal::default()))
        .build()
        .unwrap();
    let client_node = Node::builder()
        .name("client")
        .kind(NodeKind::Cli)
        .build()
        .unwrap();
    let (conn, _server) = zyris::testing::duplex(&client_node, &server).await.unwrap();
    conn.wait_capability(Duration::from_secs(2)).await.unwrap()
}

/// A poll with everything left at its default except the parts a test is about.
async fn poll(
    term: &TerminalClient,
    command: &str,
    interval_secs: u64,
    timeout_secs: u64,
    success_exit_code: i64,
    success_output_contains: Option<&str>,
) -> PollResult {
    term.poll(
        Some(command.to_string()),
        None,
        None,
        None,
        None,
        Some(interval_secs),
        Some(timeout_secs),
        Some(success_exit_code),
        success_output_contains.map(str::to_string),
        None,
    )
    .await
    .unwrap()
}

/// A check that is already true ends on the first attempt — the loop does not sleep first.
#[cfg(unix)]
#[tokio::test]
async fn a_check_that_already_passes_runs_once() {
    let term = connect().await;

    let out = poll(&term, "echo ready", 1, 5, 0, None).await;

    assert!(out.met);
    assert_eq!(out.attempts, 1);
    assert_eq!(out.exit_code, 0);
    assert!(
        out.output.contains("ready"),
        "output was dropped: {:?}",
        out.output
    );
    assert!(!out.timed_out);
}

/// The point of the tool: a check that is not yet true is retried, and the poll returns as soon as
/// the condition holds rather than waiting out its budget.
#[cfg(unix)]
#[tokio::test]
async fn a_check_that_turns_true_is_polled_until_it_does() {
    let term = connect().await;
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("appeared");
    let command = format!("test -f {}", marker.display());

    let created = marker.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        std::fs::write(created, b"here").unwrap();
    });

    let started = Instant::now();
    let out = poll(&term, &command, 1, 10, 0, None).await;

    assert!(out.met, "the poll never noticed the file: {:?}", out.output);
    assert!(
        out.attempts >= 2,
        "it succeeded without retrying: {:?}",
        out
    );
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "it waited out the budget"
    );
    assert!(
        out.output.contains("--- attempt 1"),
        "the attempts are not separated in the output: {:?}",
        out.output
    );
}

/// Output can be the condition as well as the exit code — either one ends the poll, and a nonzero
/// exit that says "it is up" is still a success.
#[cfg(unix)]
#[tokio::test]
async fn a_matching_output_ends_the_poll_even_with_a_nonzero_exit() {
    let term = connect().await;

    let out = poll(
        &term,
        "echo SERVICE-UP; exit 3",
        1,
        5,
        0,
        Some("SERVICE-UP"),
    )
    .await;

    assert!(out.met);
    assert_eq!(out.attempts, 1);
    assert_eq!(
        out.exit_code, 3,
        "the attempt's own exit code must be reported as it was"
    );
}

/// Never true is a complete answer, not an error: the poll spends its budget, then says so with
/// what the attempts printed.
#[cfg(unix)]
#[tokio::test]
async fn a_check_that_never_passes_reports_the_budget_running_out() {
    let term = connect().await;

    let started = Instant::now();
    let out = poll(&term, "echo still-not-yet; exit 1", 1, 2, 0, None).await;

    assert!(!out.met);
    assert!(out.attempts >= 2, "it gave up after one attempt: {:?}", out);
    assert_eq!(out.exit_code, 1);
    assert!(
        started.elapsed() >= Duration::from_millis(1900),
        "it returned early"
    );
    assert!(
        out.output.contains("still-not-yet"),
        "the attempts' output is missing: {:?}",
        out.output
    );
    assert!(
        out.elapsed_ms >= 1900,
        "elapsed_ms does not match the budget: {out:?}"
    );
}

/// An attempt that hangs is killed at the deadline instead of being waited on — the poll's budget
/// is the whole call's budget.
#[cfg(unix)]
#[tokio::test]
async fn an_attempt_that_hangs_is_killed_at_the_deadline() {
    let term = connect().await;

    let started = Instant::now();
    let out = poll(&term, "sleep 30", 1, 1, 0, None).await;

    assert!(!out.met);
    assert!(
        out.timed_out,
        "the hung attempt was not reported as killed: {:?}",
        out
    );
    assert_eq!(out.exit_code, -1);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the poll outlived its own budget by {:?}",
        started.elapsed()
    );
}

/// A killed attempt has no exit code of the command's making, so it must not satisfy the exit-code
/// check however `success_exit_code` is set — including -1, which is what a kill produces.
#[cfg(unix)]
#[tokio::test]
async fn a_killed_attempt_never_satisfies_the_exit_code_check() {
    let term = connect().await;

    let out = poll(&term, "sleep 30", 1, 1, -1, None).await;

    assert!(!out.met, "a kill was read as the command's own -1: {:?}", out);
    assert!(out.timed_out);
}

/// The output half is different: the substring really was printed, so the condition really is met —
/// and the result says both things rather than hiding the kill.
#[cfg(unix)]
#[tokio::test]
async fn output_from_a_killed_attempt_can_still_meet_the_condition() {
    let term = connect().await;

    let out = poll(&term, "echo SERVICE-UP; sleep 30", 1, 1, 0, Some("SERVICE-UP")).await;

    assert!(out.met);
    assert!(out.timed_out, "the result hides that the attempt was killed: {:?}", out);
    assert_eq!(out.exit_code, -1, "nothing finished, so there is no exit code to report");
}

/// When the final attempt is killed, the exit code reported is the last one that actually finished —
/// otherwise a sliver of leftover budget would erase the answer the previous attempt gave.
#[cfg(unix)]
#[tokio::test]
async fn the_last_finished_attempt_keeps_its_exit_code() {
    let term = connect().await;

    let out = poll(&term, "sleep 0.6; exit 1", 1, 2, 0, None).await;

    assert!(!out.met);
    assert!(out.timed_out, "the leftover-budget attempt was not reported as killed: {:?}", out);
    assert_eq!(out.exit_code, 1, "the useful exit code was overwritten by the kill: {:?}", out);
}

/// `argv` is the escaping-free path, and `cwd`/`env` mean what they mean for `exec`.
#[cfg(unix)]
#[tokio::test]
async fn argv_cwd_and_env_reach_the_process() {
    let term = connect().await;

    let out = term
        .poll(
            None,
            Some(vec!["sh".into(), "-c".into(), "printf '%s:%s' \"$FOO\" \"$(pwd)\"; exit 1".into()]),
            Some("/tmp".into()),
            Some([("FOO".to_string(), "bar".to_string())].into_iter().collect()),
            None,
            Some(1),
            Some(5),
            Some(5),
            None,
            None,
        )
        .await
        .unwrap();

    assert!(!out.met, "exit 1 was treated as success: {:?}", out);
    assert!(out.output.contains("bar:/tmp"), "argv/cwd/env did not reach it: {:?}", out.output);
}

/// `success_exit_code` is honoured on the exit-code path, not just through the needle.
#[cfg(unix)]
#[tokio::test]
async fn a_nonzero_success_exit_code_is_matched() {
    let term = connect().await;

    let out = poll(&term, "exit 7", 1, 5, 7, None).await;

    assert!(out.met);
    assert_eq!(out.attempts, 1);
    assert_eq!(out.exit_code, 7);
}

/// The result is bounded: a chatty check cannot hand back everything it ever printed, and the cut
/// is marked rather than silent.
#[cfg(unix)]
#[tokio::test]
async fn the_output_is_trimmed_to_the_requested_cap() {
    let term = connect().await;

    let out = term
        .poll(
            Some("i=0; while [ $i -lt 1000 ]; do printf x; i=$((i+1)); done".to_string()),
            None,
            None,
            None,
            None,
            Some(1),
            Some(5),
            Some(0),
            None,
            Some(200),
        )
        .await
        .unwrap();

    assert!(out.met);
    assert!(out.output_truncated, "the trim was not reported: {:?}", out);
    assert!(
        out.output.contains("[earlier output truncated]"),
        "got: {:?}",
        out.output
    );
    assert!(
        out.output.chars().count() < 300,
        "the cap was ignored: {} chars",
        out.output.chars().count()
    );
}
