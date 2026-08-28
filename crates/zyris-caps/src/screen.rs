use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use zyris::{Datum, WireError};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Display {
    /// Stable identifier for this display, as reported by the platform.
    pub id: String,
    /// Human-readable name. Accepted by `screenshot` wherever `id` is.
    #[serde(default)]
    pub name: String,
    /// Origin of this display within the virtual desktop, so a caller holding a global
    /// coordinate can work out which display it falls on and subtract to get a `Region`.
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    /// Size in physical pixels: the pixels a `screenshot` of this display is made of, and the ones
    /// `input.move_to` takes. `x` and `y` are in the same space.
    pub width: u32,
    pub height: u32,
    /// Physical pixels per logical pixel. `1.0` on a display that is not scaled.
    ///
    /// Reported for a caller that needs to talk about the display the way its desktop environment
    /// does. It is not something to apply to the fields above — those are already physical.
    #[serde(default)]
    pub scale_factor: f32,
    #[serde(default)]
    pub primary: bool,
}

/// A region of a display, in display-local physical pixels — the same space as [`Display`] and as
/// the coordinates `input.move_to` takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    Png,
    Jpeg,
}

impl ImageFormat {
    pub fn media_type(self) -> &'static str {
        match self {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
        }
    }
}

#[zyris::capability(name = "screen_capture", version = 1)]
pub trait ScreenCapture {
    /// List available displays.
    async fn list_displays(&self) -> zyris::Result<Vec<Display>>;

    /// Capture a still image of a display (optionally cropped to a region).
    ///
    /// `display` matches a [`Display::id`] first and a [`Display::name`] second; `None` picks the
    /// primary display. `region` is in display-local physical pixels.
    ///
    /// `format` and `max_width` exist because a full-resolution PNG of a 4K display is several
    /// megabytes, and a `Datum::Image` travels inline in the response — `zyris::proto::
    /// INLINE_BLOB_MAX` puts the comfortable ceiling at 4 MiB, which a 4K JPEG (or an ordinary
    /// desktop's 4K PNG) fits at full resolution; a larger capture is detached onto its own stream
    /// and arrives as an attachment instead. `format` defaults to PNG;
    /// `max_width` downscales the capture before encoding, preserving aspect ratio, and is
    /// ignored when it is not smaller than the capture.
    ///
    /// An implementation is free to downscale further on its own to keep the response deliverable,
    /// so the image returned may be smaller than either the display or `max_width`. When it is,
    /// the returned `Datum::Image` describes what happened — its `description` names the display's
    /// real size and the factor to multiply image coordinates by. Read it before feeding a
    /// position from a screenshot into `input.move_to`: that call takes the same `display` and the
    /// same display-local physical pixels, so the downscale factor and the `region` origin are the
    /// whole conversion — the display's own position on the desktop is `move_to`'s to add, not
    /// yours, and [`Display::scale_factor`] plays no part.
    async fn screenshot(
        &self,
        display: Option<String>,
        region: Option<Region>,
        format: Option<ImageFormat>,
        max_width: Option<u32>,
    ) -> zyris::Result<Datum>;
}

// --- What two implementations of `screen_capture` have to agree on ---
//
// These are not behaviour and they are not an example: they are the part of the declaration that
// cannot live in prose. `screen_capture` enumerates displays itself and `input` does not, so the
// two capabilities are implemented in separate crates that must not depend on each other — and
// both need to say what a display layout *is*, what a missing one answers, and what a scale of
// zero means. Duplicated per implementation, the zero-scale guard would exist in one backend and
// not the other, which is the class of bug it was written to prevent. So they sit here, beside the
// `Display` they are about, in the crate both implementations already depend on.

/// Something that can report the monitor layout.
///
/// Blocking on purpose — every caller is already inside `spawn_blocking`, and the platform APIs
/// underneath are blocking anyway.
pub trait Displays: Send + Sync + 'static {
    fn displays(&self) -> zyris::Result<Vec<Display>>;
}

/// A layout that does not change: tests, and nodes that know their monitors up front.
impl Displays for Vec<Display> {
    fn displays(&self) -> zyris::Result<Vec<Display>> {
        Ok(self.clone())
    }
}

/// The one answer to "that display is not here", so two implementations cannot word it differently.
pub fn no_such_display(wanted: &str) -> WireError {
    WireError::invalid_params(format!("no display matches `{wanted}`"))
}

/// The scale factor to actually divide or multiply by.
///
/// A display with no scale is not a display scaled by zero, but that is what reaches us: `xcap`
/// reduces over an empty output list in its Wayland branch and hands back `0.0`. Every conversion
/// built on that either divides by zero or collapses the layout onto the origin, which is the same
/// class of wrong position this guard exists to prevent.
///
/// Ungated, unlike the `#[cfg(any(feature = "screen", target_os = "macos"))]` it carried inside the
/// implementations: `input` needs it on macOS too, and a declarations crate has no feature to hang
/// it on.
pub fn display_scale(scale_factor: f32) -> f32 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    }
}
