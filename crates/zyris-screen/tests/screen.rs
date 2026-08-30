use std::io::Write;
use std::sync::Once;
use std::time::Duration;

use zyris::{Blob, Datum, Node, NodeKind};
use zyris_caps::{ImageFormat, ScreenCapture, ScreenCaptureClient, ScreenCaptureServer};
use zyris_screen::{image, HostScreenCapture, ScreenBackend};

/// Set this in a job that is supposed to have a compositor, and a missing display becomes a
/// failure instead of a silent pass.
///
/// The tests below are the only thing in this repository that can see whether a display's
/// advertised size is the size of its picture, and they need a real screen to see it. On a
/// headless runner they return early — so a headless green and a real green are the same green,
/// and the coordinate bug this variable exists because of sat through a release behind one.
const REQUIRE: &str = "ZYRIS_SCREEN_REQUIRE_DISPLAY";

/// Whether a missing display is a failure rather than a skip.
///
/// A value that means "no" does not turn it on. `REQUIRE=0` reads as "no" to everyone who types
/// it, and a guard that took it for "yes" would fail the job of the one person who was trying to
/// opt out — which is the only person who would ever set it to that.
fn skip_is_fatal(setting: Option<&str>) -> bool {
    !matches!(setting.map(str::trim), None | Some("") | Some("0") | Some("false"))
}

/// What a skipped run says.
///
/// Built as a value so the wording is testable. Softening this into a line a reader skims past is
/// how it stops working, and there is nothing else in a green log to notice.
fn skip_notice() -> String {
    format!(
        "\n\
         ================================================================\n\
         SKIPPED, NOT PASSED — zyris-screen's coordinate contract\n\
         \n\
         There is no display here, so the tests that check whether a\n\
         display's advertised size is the size of its picture did not\n\
         run. The `ok` beside them means they returned early, not that\n\
         the contract holds.\n\
         \n\
         Set {REQUIRE}=1 on a job that has a\n\
         compositor to make this a failure instead.\n\
         ================================================================\n"
    )
}

/// CI and headless dev boxes have no compositor to capture. Ask the backend directly rather than
/// letting the capability report a failure we cannot tell apart from a real bug.
///
/// Says so on the way past, once per binary — and through [`std::io::stderr`] rather than
/// `eprintln!`. **That is not a long way of writing `eprintln!`.** libtest captures the `print!`
/// and `eprint!` macros and replays them only for tests that *fail*, so an `eprintln!` here is
/// invisible in exactly the green log it is written for. Measured, on this suite's own harness: a
/// passing test's `eprintln!` produced no output at all under plain `cargo test`, while a write to
/// the `stderr()` handle came through. The handle is not routed through that capture, so this
/// survives without anyone having to pass `--nocapture`.
fn has_a_display() -> bool {
    let found = match ScreenBackend::detect() {
        #[cfg(target_os = "linux")]
        ScreenBackend::Wayland => true,
        ScreenBackend::Xcap => matches!(zyris_screen::xcap::Monitor::all(), Ok(m) if !m.is_empty()),
    };
    if !found {
        static SAID_IT: Once = Once::new();
        SAID_IT.call_once(|| {
            let mut stderr = std::io::stderr();
            let _ = stderr.write_all(skip_notice().as_bytes());
            let _ = stderr.flush();
        });
        assert!(
            !skip_is_fatal(std::env::var(REQUIRE).ok().as_deref()),
            "{REQUIRE} is set, so this job is meant to be testing the coordinate contract \
             against a real screen — and there is no display here, so it tested nothing"
        );
    }
    found
}

async fn serve(screen: HostScreenCapture) -> ScreenCaptureClient {
    eprintln!("screen backend: {:?}", screen.backend());
    let server = Node::builder()
        .name("screen-node")
        .kind(NodeKind::Service)
        .capability(ScreenCaptureServer(screen))
        .build()
        .unwrap();
    let client_node = Node::builder().name("client").kind(NodeKind::Cli).build().unwrap();
    let (conn, _server) = zyris::testing::duplex(&client_node, &server).await.unwrap();
    conn.wait_capability(Duration::from_secs(2)).await.unwrap()
}

async fn connect() -> ScreenCaptureClient {
    serve(HostScreenCapture::default()).await
}

fn inline(datum: &Datum) -> (&str, &[u8]) {
    match datum {
        Datum::Image { media_type, blob: Blob::Inline(bytes), .. } => (media_type, bytes),
        other => panic!("expected an inline image datum, got {other:?}"),
    }
}

fn description(datum: &Datum) -> &str {
    match datum {
        Datum::Image { description: Some(text), .. } => text,
        other => panic!("expected an image datum with a description, got {other:?}"),
    }
}

#[tokio::test]
async fn lists_displays() {
    if !has_a_display() {
        return;
    }
    let screen = connect().await;

    let displays = screen.list_displays().await.unwrap();
    assert!(!displays.is_empty());
    for display in &displays {
        assert!(!display.id.is_empty());
        assert!(display.width > 0 && display.height > 0);
    }
}

#[tokio::test]
async fn screenshots_the_primary_display_as_png() {
    if !has_a_display() {
        return;
    }
    let screen = connect().await;

    let datum = screen.screenshot(None, None, None, None).await.unwrap();
    let (media_type, bytes) = inline(&datum);
    assert_eq!(media_type, "image/png");
    assert_eq!(
        image::guess_format(bytes).unwrap(),
        image::ImageFormat::Png
    );
}

/// Every display must capture at the size it advertises. A backend that enumerates in one
/// coordinate space and captures in another passes `lists_displays` and still returns the wrong
/// monitor's pixels; a size mismatch is the cheapest way to catch that.
///
/// The budget is off here so the assertion stays about the backend. Auto-fit is what
/// `the_default_budget_holds_on_a_real_screen` is for.
#[tokio::test]
async fn each_display_captures_at_its_advertised_size() {
    if !has_a_display() {
        return;
    }
    let screen = serve(HostScreenCapture::default().without_budget()).await;

    for display in screen.list_displays().await.unwrap() {
        let datum = screen
            .screenshot(Some(display.id.clone()), None, None, None)
            .await
            .unwrap();
        let (_, bytes) = inline(&datum);
        let decoded = image::load_from_memory(bytes).unwrap();
        assert_eq!(
            (decoded.width(), decoded.height()),
            (display.width, display.height),
            "display {} ({}) at ({}, {})",
            display.id,
            display.name,
            display.x,
            display.y
        );
    }
}

/// The point of the auto-fit pass: a caller that passes nothing still gets something that will
/// survive the trip, and is told what the scaling did to its coordinates.
#[tokio::test]
async fn the_default_budget_holds_on_a_real_screen() {
    if !has_a_display() {
        return;
    }
    let screen = connect().await;

    for display in screen.list_displays().await.unwrap() {
        let datum = screen
            .screenshot(Some(display.id.clone()), None, None, None)
            .await
            .unwrap();
        let (_, bytes) = inline(&datum);
        assert!(
            bytes.len() <= zyris::proto::INLINE_BLOB_MAX,
            "{} is {} bytes, over the {} budget",
            display.id,
            bytes.len(),
            zyris::proto::INLINE_BLOB_MAX
        );

        let decoded = image::load_from_memory(bytes).unwrap();
        let text = description(&datum);
        assert!(text.contains(&display.id), "{text}");
        assert!(
            text.contains(&format!("{}x{}", display.width, display.height)),
            "description must name the display's real size: {text}"
        );
        if decoded.width() != display.width {
            assert!(
                text.contains("multiply image coordinates by"),
                "a scaled capture must say so: {text}"
            );
        }
        eprintln!(
            "{}: {}x{} -> {}x{} in {} bytes",
            display.id,
            display.width,
            display.height,
            decoded.width(),
            decoded.height(),
            bytes.len()
        );
    }
}

#[tokio::test]
async fn max_width_downscales_and_format_selects_jpeg() {
    if !has_a_display() {
        return;
    }
    let screen = connect().await;

    let datum = screen
        .screenshot(None, None, Some(ImageFormat::Jpeg), Some(320))
        .await
        .unwrap();
    let (media_type, bytes) = inline(&datum);
    assert_eq!(media_type, "image/jpeg");

    let decoded = image::load_from_memory(bytes).unwrap();
    assert_eq!(decoded.width(), 320);
    assert!(bytes.len() < zyris::proto::INLINE_BLOB_MAX);
}

#[tokio::test]
async fn a_region_crops_and_an_unknown_display_is_rejected() {
    if !has_a_display() {
        return;
    }
    let screen = connect().await;
    let displays = screen.list_displays().await.unwrap();
    let display = displays.iter().find(|d| d.primary).unwrap_or(&displays[0]);

    let region = zyris_caps::Region { x: 0, y: 0, width: 64, height: 48 };
    let datum = screen
        .screenshot(Some(display.id.clone()), Some(region), None, None)
        .await
        .unwrap();
    let (_, bytes) = inline(&datum);
    let decoded = image::load_from_memory(bytes).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (64, 48));

    let err = screen
        .screenshot(Some("no-such-display".into()), None, None, None)
        .await
        .unwrap_err();
    assert!(format!("{err}").contains("no-such-display"), "{err}");
}

// --- The guards on the guard ---
//
// These three need no display, so they are what CI actually runs here. They do not check the
// coordinate contract — nothing headless can — they check that a run which *could not* check it
// says so, loudly enough and in every test, and that the way to demand a real screen works.

/// The notice has to keep saying the thing that makes a reader stop scrolling.
#[test]
fn a_skipped_run_says_it_did_not_pass() {
    let notice = skip_notice();
    assert!(notice.contains("SKIPPED, NOT PASSED"), "{notice}");
    assert!(notice.contains(REQUIRE), "{notice}");
    assert!(notice.contains("did not"), "{notice}");
}

/// Demanding a real screen is opt-in, and opting *out* has to be possible with the value everyone
/// reaches for. A guard that read `REQUIRE=0` as "yes" would fail the one job trying to say no.
#[test]
fn only_a_setting_that_means_yes_turns_a_skip_into_a_failure() {
    assert!(!skip_is_fatal(None));
    assert!(!skip_is_fatal(Some("")));
    assert!(!skip_is_fatal(Some("  ")));
    assert!(!skip_is_fatal(Some("0")));
    assert!(!skip_is_fatal(Some("false")));

    assert!(skip_is_fatal(Some("1")));
    assert!(skip_is_fatal(Some("required")));
    assert!(skip_is_fatal(Some("true")));
}

/// Every test in this file needs a screen, and every one of them has to say so.
///
/// A new test written without the guard does not skip on a headless box — it fails there, on a
/// contributor's laptop and on CI, for a reason that has nothing to do with what it was testing.
/// The needles are split so this test does not count itself.
#[test]
fn every_test_that_needs_a_screen_asks_for_one() {
    let src = include_str!("screen.rs");
    let needing = src.matches(concat!("#[tokio", "::test]")).count();
    let asking = src.matches(concat!("if !has_a_display", "() {")).count();
    assert_eq!(
        needing, asking,
        "{needing} tests take a screen but {asking} guard on having one — a test that skips \
         quietly is the whole problem, and one that forgets to skip is a red build on every \
         headless machine"
    );
}
