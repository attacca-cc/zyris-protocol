//! Capture over `wl_output` + `zwlr_screencopy`, the same pair `grim` uses.
//!
//! The dependency is `libwayshot-xcap` rather than upstream `libwayshot` only because `xcap`
//! already pulls that fork in: taking the other one would build a second copy of the whole Wayland
//! stack for the same protocol.

use image::RgbaImage;
use libwayshot::{output::OutputInfo, WayshotConnection};
use zyris_caps::{display_scale, no_such_display, Display};

use super::internal;

/// The registry name of the protocol this backend is built on. Compared as a string because the
/// question is whether the compositor advertises it at all, which the registry answers without
/// binding anything and therefore without taking a frame.
const SCREENCOPY: &str = "zwlr_screencopy_manager_v1";

/// Whether this session has a compositor that will hand over pixels.
///
/// GNOME and KDE do not implement `zwlr_screencopy`; they screenshot through a portal, which is
/// what the `xcap` backend already knows how to reach.
///
/// **Opening a connection and seeing outputs is a different question, and answering that one was a
/// bug.** `wl_output` is core Wayland, so on GNOME the connection opens and every monitor is
/// listed; the screencopy manager is bound later, inside the capture, and only there does it fail
/// with `ProtocolNotFound`. So this said yes on exactly the compositors the paragraph above says it
/// excludes, [`ScreenBackend::detect`] committed to this backend, and every screenshot the node
/// served failed — with a message about a wayland protocol, on a desktop that screenshots fine.
///
/// [`ScreenBackend::detect`]: super::ScreenBackend::detect
pub(super) fn is_available() -> bool {
    let Ok(conn) = WayshotConnection::new() else { return false };
    if conn.get_all_outputs().is_empty() {
        return false;
    }
    conn.globals.contents().with_list(|globals| {
        globals.iter().any(|global| global.interface == SCREENCOPY)
    })
}

/// Wayland has no notion of a primary output, so the one covering the origin stands in. That is
/// where a compositor puts the output a user thinks of as first, and it gives `screenshot(None)`
/// a stable answer rather than whichever output the compositor happened to advertise first.
fn covers_origin(output: &OutputInfo) -> bool {
    let region = output.logical_region.inner;
    let (x, y) = (region.position.x, region.position.y);
    x <= 0
        && y <= 0
        && x + region.size.width as i32 > 0
        && y + region.size.height as i32 > 0
}

/// Physical pixels, not the compositor's logical ones: this is the space `screenshot_single_output`
/// hands back and the space `input.move_to` moves in, and reporting the logical size here is what
/// puts a cursor at `1/scale` of where it was aimed.
///
/// The origin is the exact conversion only while every output shares a scale — a mixed-scale
/// wlroots layout has no single physical grid to place them on. `EnigoInput::move_to` refuses a
/// multi-output Wayland layout for a related reason, so nothing depends on the inexact case.
fn describe(output: &OutputInfo) -> Display {
    let region = output.logical_region.inner;
    // `OutputInfo::scale` is private upstream; it is this ratio.
    let factor = display_scale(match region.size.height {
        0 => 1.0,
        logical => output.physical_size.height as f32 / logical as f32,
    });
    Display {
        id: output.name.clone(),
        name: if output.description.is_empty() {
            output.name.clone()
        } else {
            output.description.clone()
        },
        x: (region.position.x as f32 * factor).round() as i32,
        y: (region.position.y as f32 * factor).round() as i32,
        width: output.physical_size.width,
        height: output.physical_size.height,
        scale_factor: factor,
        primary: covers_origin(output),
    }
}

fn select<'a>(outputs: &'a [OutputInfo], wanted: Option<&str>) -> zyris::Result<&'a OutputInfo> {
    let Some(wanted) = wanted else {
        return outputs
            .iter()
            .find(|o| covers_origin(o))
            .or_else(|| outputs.first())
            .ok_or_else(|| internal("no displays are attached"));
    };
    outputs
        .iter()
        .find(|o| o.name == wanted)
        .or_else(|| outputs.iter().find(|o| o.description == wanted))
        .ok_or_else(|| no_such_display(wanted))
}

pub(super) fn displays() -> zyris::Result<Vec<Display>> {
    let conn = WayshotConnection::new().map_err(internal)?;
    Ok(conn.get_all_outputs().iter().map(describe).collect())
}

pub(super) fn capture(display: Option<&str>) -> zyris::Result<(String, RgbaImage)> {
    let conn = WayshotConnection::new().map_err(internal)?;
    let outputs = conn.get_all_outputs();
    let output = select(outputs, display)?;
    // One whole output. Asking for a sub-region here would go down `capture_output_region`, whose
    // whole-output case is broken on at least Hyprland — cropping happens in the parent module.
    let image = conn.screenshot_single_output(output, false).map_err(internal)?;
    Ok((output.name.clone(), image.into_rgba8()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Saying this backend is available is a promise that a capture will work.
    ///
    /// The two questions came apart on GNOME: `wl_output` is core Wayland, so the connection opened
    /// and every monitor was listed, and the missing piece — `zwlr_screencopy` — was only ever
    /// looked for inside the capture. `detect` committed to this backend and every screenshot the
    /// node served failed. So the assertion is the promise itself rather than the mechanism: if
    /// this session says yes, taking a picture has to work.
    ///
    /// A compositor without the protocol reaches the `if` and stops, which is the whole point —
    /// there is nothing to capture and nothing to wait for. On one that has it this takes a frame,
    /// once, and that is the only place in this suite where the promise can actually be tested.
    #[test]
    fn saying_yes_here_means_a_capture_will_work() {
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return;
        }
        if is_available() {
            let taken = capture(None);
            assert!(
                taken.is_ok(),
                "this session advertises {SCREENCOPY} and `is_available` said yes, but capturing \
                 failed with {:?} — the probe and the capture disagree, which is the state that \
                 made every screenshot on GNOME fail",
                taken.err()
            );
        }
    }
}
