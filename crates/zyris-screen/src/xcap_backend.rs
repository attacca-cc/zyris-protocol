use image::RgbaImage;
use xcap::{Monitor, XCapError};
use zyris::WireError;
use zyris_caps::{display_scale, no_such_display, Display};

use super::internal;

/// A display's identity survives one call, not a reboot: `id()` is what the platform reports, and
/// the index is only there so a monitor whose id lookup fails is still addressable.
fn monitor_id(monitor: &Monitor, index: usize) -> String {
    match monitor.id() {
        Ok(id) => id.to_string(),
        Err(_) => format!("display-{index}"),
    }
}

fn monitor_name(monitor: &Monitor) -> String {
    monitor
        .friendly_name()
        .or_else(|_| monitor.name())
        .unwrap_or_default()
}

/// Which coordinate space `xcap` reported this session's geometry in, and so what has to be done
/// to reach the space it captures in.
///
/// `xcap` does not report one space, and on two of its branches the space it picks is not the one
/// it captures in. The rule is carried as a value rather than as a `cfg` fork because no machine
/// is X11 *and* Wayland *and* macOS *and* Windows. Gated, each platform's arithmetic could only
/// ever be executed on that platform — and CI's Windows job builds without running this crate's
/// tests, so the Windows rule had never been executed anywhere at all. As a value, every branch is
/// exercised by the tests at the bottom of this file on whichever runner has a toolchain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reported {
    /// Windows: geometry is already the picture.
    ///
    /// `Monitor::width` is `dmPelsWidth` from `EnumDisplaySettingsW` and `x`/`y` are `dmPosition`,
    /// both device pixels; the GDI path allocates a `CreateCompatibleBitmap` of exactly that size
    /// and the WGC path copies a `D3D11_BOX` of exactly that size. Only the scale is normalised.
    AsCaptured,
    /// macOS, and X11: geometry is the picture divided by the scale, so multiplying returns it.
    ///
    /// On macOS `Monitor::width` is `CGDisplayBounds` in Quartz points and `scale_factor` is
    /// defined as `pixel_width / width` — so the product is an identity rather than an estimate,
    /// and it lands on the backing store, which is what `CGWindowListCreateImage` returns with
    /// `CGWindowImageOption::Default`. Note that in a *scaled* Retina mode the backing store is
    /// not the panel: macOS renders large and downsamples, so a 1680x1050 logical mode captures at
    /// 3360x2100 on a 2880x1800 panel. The contract is the size of the picture, so the backing
    /// store is the right answer here and the panel would be the wrong one.
    ///
    /// On X11 `Monitor::width` is the RandR monitor geometry divided by `Xft.dpi / 96`, while
    /// `capture_image` passes the *undivided* RandR rectangle to `xorg_capture`. Multiplying
    /// returns the RandR geometry, which is what the picture is made of.
    ScaledDown,
    /// Linux in a Wayland session: the two numbers come from two different windowing systems and
    /// no arithmetic on the pair can reach the picture. The size is measured instead.
    ///
    /// # What was wrong here, and how it was measured
    ///
    /// This function used to multiply on every platform but Windows, and `tests/screen.rs` was
    /// left deliberately failing on a real GNOME screen with a note saying so. Measured
    /// 2026-08-28 on one 1920x1080 panel at 125%:
    ///
    /// ```text
    /// xcap reports    2457x1382, scale_factor 1.25
    /// multiplying     3071x1728
    /// a capture is    1920x1080     <- the panel, and what a caller actually receives
    /// ```
    ///
    /// That note concluded "whatever `2457x1382` is, it is not RandR geometry divided by `1.25`".
    /// **It is exactly that.** The premise that fails is the unexamined one: under Xwayland, RandR
    /// geometry is not the panel. mutter hands Xwayland an *integer* scale — `ceil` of the highest
    /// monitor scale, in `meta_xwayland_get_effective_scale` — so the X screen is the compositor's
    /// logical size times that integer, while `xcap`'s `scale_factor` comes from `wl_output`'s
    /// real fractional one. Numerator and denominator are from different windowing systems:
    ///
    /// ```text
    /// panel              1920x1080   the mode, and what a capture is
    /// logical (stage)    1536x864    1920x1080 / 1.25
    /// X screen (RandR)   3072x1728   1536x864 * ceil(1.25), i.e. * 2
    /// xcap reports       2457x1382   3072/1.25 and 1728/1.25, truncated by `as u32`
    /// multiplying        3071x1728   back onto the X screen, less what the truncation lost
    /// ```
    ///
    /// Measured across all six scales this panel offers, `advertised / panel` came to 1.000,
    /// 1.599, 1.500, 1.333, 1.199, 1.000 for scales 1, 1.25, 4/3, 1.5, 5/3 and 2. That is
    /// `ceil(s) / s` — not a constant, and not a rounding artefact. It is 1 at every *integer*
    /// scale, which is why this survived to a release: the multiply is right on X11 always, right
    /// on macOS always, and right on Wayland at 100% and 200%.
    ///
    /// So the size is read from `wl_output`'s current mode instead — what `libwayshot` reports as
    /// `physical_size`, which measured 1920x1080 at all six scales. That is right on KDE too,
    /// where `xcap` issue #165 is this same defect showing its other face, and it costs no
    /// dependency: this crate already links the `libwayshot` fork `xcap` itself pulls in, and
    /// `WayshotConnection::new` binds only `wl_output` and `zxdg_output_manager_v1`, both of which
    /// mutter advertises. A compositor that does not answer falls back to the multiply, which is
    /// where this started and cannot be made worse by trying.
    ///
    /// # Still wrong, and known to be
    ///
    /// - **A second monitor.** `xcap`'s `capture_monitor` hands `wayland_capture` the raw RandR
    ///   rectangle, and its GNOME branch then crops the compositor's PNG from `(0, 0)` rather than
    ///   from that rectangle's origin. A second monitor's advertised size is now right and its
    ///   pixels still are not, so `each_display_captures_at_its_advertised_size` still fails on a
    ///   multi-monitor GNOME session. That one is upstream's.
    /// - **Rotation.** `wl_output`'s `Mode` is pre-transform — `libwayshot` transposes it itself
    ///   before building a frame — while RandR's monitor geometry is post-transform. On a rotated
    ///   output this substitution is a regression against the multiply, and it is unverified:
    ///   there was no rotated screen here to measure. The contract test is where it would surface.
    Xwayland,
}

/// `xcap`'s own `wayland_detect`, spelled identically on purpose.
///
/// The question is not "is this a Wayland session" but "which of *`xcap`'s* branches produced the
/// numbers being corrected", and `xcap` answers that from these two variables and nothing else —
/// so a cleverer probe here would be a different question with the same name. Copied from
/// `xcap/src/linux/utils.rs`; if it changes upstream this has to change with it.
///
/// Takes its arguments rather than reading the environment so that it is testable on every
/// platform and touches no state the rest of a threaded test binary shares.
fn looks_like_wayland(session_type: &str, wayland_display: &str) -> bool {
    session_type == "wayland" || wayland_display.to_lowercase().contains("wayland")
}

fn wayland_session() -> bool {
    fn var(name: &str) -> String {
        std::env::var(name).unwrap_or_default()
    }
    looks_like_wayland(&var("XDG_SESSION_TYPE"), &var("WAYLAND_DISPLAY"))
}

/// Which rule this platform calls for, as a pure function of the two things that decide it.
///
/// A `match` on a string rather than a `#[cfg]` fork so that every row is compiled and executed
/// everywhere. The point is small and was expensive: gated, the Windows rule could only ever be
/// run on Windows, and CI's Windows job builds this crate without running its tests — so that rule
/// had never been executed anywhere. Now an edit that breaks it goes red on the Linux runner.
fn rule_for(target_os: &str, wayland: bool) -> Reported {
    match target_os {
        "windows" => Reported::AsCaptured,
        // `xcap` reads the Wayland variables only from its Linux module, so nothing off Linux
        // reaches the branch this is about, however the environment happens to be set.
        "linux" if wayland => Reported::Xwayland,
        _ => Reported::ScaledDown,
    }
}

fn reported() -> Reported {
    rule_for(std::env::consts::OS, wayland_session())
}

/// Where a display is and how big, according to something other than `xcap`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Geometry {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale_factor: f32,
}

/// Match by name, because that is the one field the two enumerations spell the same way.
///
/// `xcap`'s Linux `friendly_name()` is the RandR output name and the Wayland backend's `id` is
/// `wl_output`'s name; both are the connector, `eDP-1`. The `id`s are not comparable — `xcap`'s is
/// an XRandR resource number — which is also why the corrected `Display` keeps `xcap`'s, so that
/// `select` and `capture` still resolve what `list_displays` handed out.
fn geometry_of(name: &str, measured: &[Display]) -> Option<Geometry> {
    if name.is_empty() {
        return None;
    }
    measured
        .iter()
        .find(|m| m.id == name)
        .map(|m| Geometry {
            x: m.x,
            y: m.y,
            width: m.width,
            height: m.height,
            scale_factor: m.scale_factor,
        })
}

/// Put a display in the space it captures in.
///
/// Pure, and takes the rule as an argument, so every platform's branch runs in every test run.
fn correct(reported: Reported, display: Display, measured: Option<Geometry>) -> Display {
    let factor = display_scale(display.scale_factor);
    match (reported, measured) {
        // Nothing to undo.
        (Reported::AsCaptured, _) => Display { scale_factor: factor, ..display },
        // Geometry replaced wholesale rather than corrected: there is no factor relating what
        // `xcap` said to what a capture is, which is the entire finding. The scale comes from the
        // same measurement — `xcap` reduces its own over every output with `max`, so on a
        // mixed-scale desktop it reports one display's scale for all of them.
        (Reported::Xwayland, Some(g)) => Display {
            x: g.x,
            y: g.y,
            width: g.width,
            height: g.height,
            scale_factor: display_scale(g.scale_factor),
            ..display
        },
        // `ScaledDown`, and `Xwayland` with nothing to measure. The fallback is deliberately the
        // behaviour this file had before the fix: a compositor that will not answer leaves us
        // exactly where we were, which is wrong on GNOME at a fractional scale and right
        // everywhere else `xcap` divides.
        _ => Display {
            x: (display.x as f32 * factor).round() as i32,
            y: (display.y as f32 * factor).round() as i32,
            width: (display.width as f32 * factor).round() as u32,
            height: (display.height as f32 * factor).round() as u32,
            scale_factor: factor,
            ..display
        },
    }
}

/// Ask `wl_output` for the sizes `xcap` cannot work out, once per enumeration rather than once per
/// monitor: this opens a Wayland connection and round-trips the registry.
#[cfg(target_os = "linux")]
fn measured_geometry(reported: Reported) -> Vec<Display> {
    if reported != Reported::Xwayland {
        return Vec::new();
    }
    match super::wayland_backend::displays() {
        Ok(outputs) => outputs,
        Err(err) => {
            tracing::warn!(
                %err,
                "could not read wl_output for display sizes; falling back to xcap's geometry, \
                 which is the compositor's logical layout scaled by Xwayland and is not what a \
                 capture of these displays will be"
            );
            Vec::new()
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn measured_geometry(_reported: Reported) -> Vec<Display> {
    Vec::new()
}

fn describe(monitor: &Monitor, index: usize, reported: Reported, measured: &[Display]) -> Display {
    let name = monitor_name(monitor);
    let geometry = geometry_of(&name, measured);
    correct(
        reported,
        Display {
            id: monitor_id(monitor, index),
            name,
            x: monitor.x().unwrap_or(0),
            y: monitor.y().unwrap_or(0),
            width: monitor.width().unwrap_or(0),
            height: monitor.height().unwrap_or(0),
            scale_factor: monitor.scale_factor().unwrap_or(1.0),
            primary: monitor.is_primary().unwrap_or(false),
        },
        geometry,
    )
}

fn select<'a>(monitors: &'a [Monitor], wanted: Option<&str>) -> zyris::Result<(&'a Monitor, usize)> {
    if let Some(wanted) = wanted {
        let found = monitors
            .iter()
            .enumerate()
            .find(|(index, m)| monitor_id(m, *index) == wanted)
            .or_else(|| {
                monitors
                    .iter()
                    .enumerate()
                    .find(|(_, m)| monitor_name(m) == wanted)
            });
        return match found {
            Some((index, monitor)) => Ok((monitor, index)),
            None => Err(no_such_display(wanted)),
        };
    }
    let index = monitors
        .iter()
        .position(|m| m.is_primary().unwrap_or(false))
        .unwrap_or(0);
    Ok((&monitors[index], index))
}

pub(super) fn displays() -> zyris::Result<Vec<Display>> {
    let monitors = Monitor::all().map_err(internal)?;
    let reported = reported();
    let measured = measured_geometry(reported);
    Ok(monitors
        .iter()
        .enumerate()
        .map(|(index, monitor)| describe(monitor, index, reported, &measured))
        .collect())
}

pub(super) fn capture(display: Option<&str>) -> zyris::Result<(String, RgbaImage)> {
    let monitors = Monitor::all().map_err(internal)?;
    if monitors.is_empty() {
        return Err(internal("no displays are attached"));
    }
    let (monitor, index) = select(&monitors, display)?;
    let id = monitor_id(monitor, index);
    let image = monitor.capture_image().map_err(|err| match err {
        XCapError::InvalidCaptureRegion(msg) => WireError::invalid_params(msg),
        other => internal(other),
    })?;
    Ok((id, image))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One display, at whatever `xcap` said its geometry and scale were.
    fn reports(width: u32, height: u32, scale_factor: f32) -> Display {
        Display {
            id: "33".into(),
            name: "eDP-1".into(),
            x: 1920,
            y: -360,
            width,
            height,
            scale_factor,
            primary: false,
        }
    }

    fn measures(width: u32, height: u32, scale_factor: f32) -> Vec<Display> {
        vec![Display {
            id: "eDP-1".into(),
            name: "Built-in".into(),
            x: 0,
            y: 0,
            width,
            height,
            scale_factor,
            primary: true,
        }]
    }

    /// Which rule applies where, as a table rather than as six platforms nobody has.
    ///
    /// This is the mapping that used to be a `#[cfg]` fork, where each row could only ever be
    /// executed on the platform it was about — and CI's Windows job runs no test in this crate, so
    /// the Windows row had never been executed at all.
    #[test]
    fn every_platform_gets_the_rule_its_own_apis_call_for() {
        assert_eq!(rule_for("windows", false), Reported::AsCaptured);
        // `xcap` consults `XDG_SESSION_TYPE`/`WAYLAND_DISPLAY` only in its Linux module, so a
        // stray variable elsewhere does not change which of its branches produced the numbers.
        assert_eq!(rule_for("windows", true), Reported::AsCaptured);
        assert_eq!(rule_for("macos", false), Reported::ScaledDown);
        assert_eq!(rule_for("macos", true), Reported::ScaledDown);
        assert_eq!(rule_for("linux", false), Reported::ScaledDown);
        assert_eq!(rule_for("linux", true), Reported::Xwayland);
    }

    /// `xcap`'s test, not ours: the exact predicate it uses to pick the branch we are correcting.
    #[test]
    fn a_wayland_session_is_recognised_the_way_xcap_recognises_one() {
        assert!(looks_like_wayland("wayland", ""));
        assert!(looks_like_wayland("", "wayland-0"));
        // Upstream lowercases the display but compares the session type exactly. Matching that is
        // the whole point — a probe that is right in general but different from `xcap`'s answers a
        // different question from the one being asked.
        assert!(looks_like_wayland("", "WAYLAND-1"));
        assert!(!looks_like_wayland("Wayland", ""));
        assert!(!looks_like_wayland("x11", ":0"));
        assert!(!looks_like_wayland("", ""));
    }

    /// X11: `Monitor::width` is the RandR geometry divided by `Xft.dpi / 96`, and the capture is
    /// the undivided RandR rectangle. Multiplying returns it.
    #[test]
    fn an_x11_display_is_reported_in_the_pixels_it_captures_in() {
        let display = correct(Reported::ScaledDown, reports(1920, 1080, 2.0), None);
        assert_eq!((display.width, display.height), (3840, 2160));
        assert_eq!((display.x, display.y), (3840, -720));
        assert_eq!(display.scale_factor, 2.0);
    }

    /// macOS, and the reason "the picture" is the contract rather than "the panel".
    ///
    /// In a scaled Retina mode macOS renders to an oversized backing store and downsamples it onto
    /// the panel, and `CGWindowListCreateImage` hands back the backing store. So a 1680x1050
    /// logical mode captures at 3360x2100 while the panel is 2880x1800, and 3360x2100 is the right
    /// answer: it is what the caller receives, what a region is cropped from, and what
    /// `input.move_to` has to be told about. Aiming at the panel here would be a different bug
    /// with the same shape as the one on GNOME.
    #[test]
    fn a_mac_reports_points_and_captures_its_backing_store() {
        let display = correct(Reported::ScaledDown, reports(1680, 1050, 2.0), None);
        assert_eq!((display.width, display.height), (3360, 2100));
    }

    /// Windows: `dmPelsWidth` is already device pixels and both capture backends produce exactly
    /// that many, so touching the geometry is what would break it.
    #[test]
    fn windows_geometry_is_already_physical() {
        let display = correct(Reported::AsCaptured, reports(1920, 1080, 2.0), None);
        assert_eq!((display.width, display.height), (1920, 1080));
        assert_eq!((display.x, display.y), (1920, -360));
        assert_eq!(display.scale_factor, 2.0);
    }

    /// The defect, frozen at the numbers it was measured at.
    ///
    /// GNOME/mutter, Wayland, one 1920x1080 panel at 125%: `xcap` reports 2457x1382 with a scale
    /// of 1.25 and a capture is 1920x1080. Multiplying gives 3071x1728 — the Xwayland screen,
    /// which is a real number that really exists and is not the one anybody receives. This runs on
    /// a headless runner, which is where the real contract test cannot.
    #[test]
    fn an_xwayland_display_is_the_size_wl_output_says_it_is() {
        let measured = measures(1920, 1080, 1.25);
        let display = correct(
            Reported::Xwayland,
            reports(2457, 1382, 1.25),
            geometry_of("eDP-1", &measured),
        );
        assert_eq!((display.width, display.height), (1920, 1080));
        assert_eq!((display.x, display.y), (0, 0));
        assert_eq!(display.scale_factor, 1.25);
        // The identity `select` and `capture` resolve by is `xcap`'s, not `wl_output`'s: those two
        // still go through `Monitor::all`, so handing out the Wayland name would hand out an id
        // that cannot be used to ask for a screenshot.
        assert_eq!(display.id, "33");
    }

    /// A compositor that will not answer leaves us exactly where this file was before the fix.
    ///
    /// Deliberately not "leave `xcap`'s numbers alone": those are right nowhere, whereas the
    /// multiply is right on X11 and on Wayland at every integer scale. Falling back to the old
    /// behaviour cannot make a session worse than it already was.
    #[test]
    fn an_xwayland_session_with_nothing_to_measure_keeps_the_old_arithmetic() {
        let display = correct(Reported::Xwayland, reports(2457, 1382, 1.25), None);
        assert_eq!((display.width, display.height), (3071, 1728));
    }

    /// A display that is not scaled must come out byte-identical under every rule.
    ///
    /// This is the configuration almost every machine is in, so it is the one a regression would
    /// be quietest in: the two rules that do arithmetic multiply by one, and the Xwayland rule
    /// substitutes a measurement that agrees. Nothing here is allowed to move a pixel.
    #[test]
    fn a_display_that_is_not_scaled_is_left_exactly_where_it_is() {
        let unscaled = reports(1920, 1080, 1.0);
        for rule in [Reported::AsCaptured, Reported::ScaledDown, Reported::Xwayland] {
            assert_eq!(
                correct(rule, unscaled.clone(), None),
                unscaled,
                "{rule:?} moved a display that is not scaled"
            );
        }
        // And with `wl_output` actually answering, which is the live path on an unscaled GNOME
        // session: the measurement agrees with `xcap` there, so it is still a no-op.
        let measured = measures(1920, 1080, 1.0);
        let corrected = correct(
            Reported::Xwayland,
            unscaled.clone(),
            geometry_of("eDP-1", &measured),
        );
        assert_eq!((corrected.width, corrected.height), (1920, 1080));
        assert_eq!(corrected.scale_factor, 1.0);
    }

    /// `xcap` hands back `0.0` where it cannot work a scale out — it reduces over an empty output
    /// list with `max`. Multiplying by it would put every display at the origin with no size at
    /// all, and substituting it would hand a caller a scale it cannot divide by.
    #[test]
    fn a_scale_of_zero_is_a_scale_of_one() {
        let expected = reports(1920, 1080, 1.0);
        assert_eq!(correct(Reported::ScaledDown, reports(1920, 1080, 0.0), None), expected);
        assert_eq!(correct(Reported::ScaledDown, reports(1920, 1080, f32::NAN), None), expected);
        assert_eq!(correct(Reported::AsCaptured, reports(1920, 1080, 0.0), None), expected);

        let measured = measures(1920, 1080, 0.0);
        let corrected = correct(
            Reported::Xwayland,
            reports(2457, 1382, 1.25),
            geometry_of("eDP-1", &measured),
        );
        assert_eq!(corrected.scale_factor, 1.0);
    }

    /// The two enumerations agree on the connector name and on nothing else. Matching on anything
    /// but the name silently measures the wrong monitor, or none.
    #[test]
    fn a_measurement_is_found_by_connector_name_and_not_otherwise() {
        let measured = measures(1920, 1080, 1.25);
        assert!(geometry_of("eDP-1", &measured).is_some());
        // `xcap`'s id is an XRandR resource number and the Wayland `name` field is a description.
        assert!(geometry_of("33", &measured).is_none());
        assert!(geometry_of("Built-in", &measured).is_none());
        assert!(geometry_of("HDMI-1", &measured).is_none());
        // `monitor_name` falls back to `String::default()`, and an empty name must not match an
        // output whose name is also empty — that would be a coincidence, not an identification.
        assert!(geometry_of("", &measures(1920, 1080, 1.25)).is_none());
        assert!(geometry_of("eDP-1", &[]).is_none());
    }
}
