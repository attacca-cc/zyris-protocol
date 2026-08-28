//! Picking one display out of a layout this crate was handed.
//!
//! The layout itself is not this crate's to produce: `input` needs to know where the monitors are
//! but has no business enumerating them, and the enumerators — `xcap`, `libwayshot` — drag in the
//! whole display stack. So [`EnigoInput`](crate::EnigoInput) takes a
//! [`Displays`](zyris_caps::Displays) from whoever builds it, and the *trait* lives in `zyris-caps`
//! because two implementations of it — `zyris-screen`'s `HostDisplays` and a plain `Vec<Display>`
//! — have to agree on what they are reporting. What is left here is the matching rule, which only
//! `input` performs.

use zyris::WireError;
use zyris_caps::{no_such_display, Display};

/// Id first, name second — the rule `screen_capture.screenshot` documents, applied to a layout
/// that has already been enumerated rather than to live platform handles. The `screen` backends do
/// their own matching against live `Monitor`/`OutputInfo` handles instead.
pub(crate) fn resolve<'a>(displays: &'a [Display], wanted: &str) -> zyris::Result<&'a Display> {
    if displays.is_empty() {
        return Err(WireError::new(
            zyris::ErrorCode::Internal,
            "no displays are attached",
        ));
    }
    displays
        .iter()
        .find(|d| d.id == wanted)
        .or_else(|| displays.iter().find(|d| d.name == wanted))
        .ok_or_else(|| no_such_display(wanted))
}
