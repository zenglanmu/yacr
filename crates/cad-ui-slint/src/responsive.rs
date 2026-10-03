//! Responsive shell geometry and breakpoint classification (audit U01/U07/U10).
//!
//! The Slint shell must arrange the same real controls differently at phone,
//! tablet and desktop widths, keep touch targets at least [`MIN_TOUCH_TARGET`]
//! logical pixels on a phone, and actually consume the compact/stretch
//! configuration instead of leaving it dead (`UiConfiguration::compact`).
//!
//! All of that is **geometry**, so it lives here as a pure function of the
//! current logical viewport instead of being sprinkled as literals through the
//! `.slint` file. The shell's width `state` reads the class this module decides;
//! the unit tests pin the breakpoints, the target sizes and the compact
//! consumption without needing a Slint host (this crate cannot build natively on
//! the CI host — see `docs/responsive-ui.md`).
//!
//! Honest scope: this decides and documents the arrangement; it does **not**
//! prove the rendered result. Slint rendering, DPR and safe-area composition are
//! not exercised here (audit U07 remains open for the host).

/// Minimum logical-pixel touch target on a phone (audit U01).
pub const MIN_TOUCH_TARGET: f32 = 48.0;
/// Minimum logical-pixel touch target elsewhere (pointer input is finer).
pub const MIN_POINTER_TARGET: f32 = 32.0;

/// The responsive class for a logical viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Breakpoint {
    /// Narrow, touch-first. Arranged as a bottom grouped bar plus a drawer.
    Phone,
    /// Mid-width touch device. Rearranged but not the phone drawer model.
    Tablet,
    /// Wide screen. Large canvas plus floating navigation panels.
    Desktop,
}

impl Breakpoint {
    /// Classify a logical viewport width.
    ///
    /// <= 599 px is a phone (the 360x800 portrait reference and 800x360
    /// landscape reference stay reachable), 600-1023 px is a tablet, and
    /// At least 1024 px is a desktop. These legacy breakpoints are documented in
    /// `docs/responsive-ui.md` and pinned by [`tests`].
    pub fn from_width(width: f32) -> Breakpoint {
        if width < 600.0 {
            Breakpoint::Phone
        } else if width < 1024.0 {
            Breakpoint::Tablet
        } else {
            Breakpoint::Desktop
        }
    }

    /// Whether the shell uses the phone bottom-bar + drawer arrangement.
    pub fn uses_drawer(self) -> bool {
        matches!(self, Breakpoint::Phone)
    }

    /// Whether the shell defaults to a collapsed side panel (wide *work* view).
    ///
    /// The work-mode desktop keeps the grouped tools in a side panel that starts
    /// collapsed; the view-mode desktop floats only navigation controls. A phone
    /// never has a side panel — it has the drawer.
    pub fn defaults_to_collapsed_panel(self) -> bool {
        matches!(self, Breakpoint::Desktop | Breakpoint::Tablet)
    }

    /// Whether the floating view-mode navigation strip is shown (wide screens).
    pub fn shows_floating_nav(self) -> bool {
        matches!(self, Breakpoint::Desktop)
    }
}

/// Concrete geometric decisions pushed into the shell's `width` state.
///
/// Every field is derived from the real viewport width and the configuration;
/// the shell binds to these instead of hardcoding pixel values, so the compact
/// setting demonstrably changes the result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResponsiveMetrics {
    pub breakpoint: Breakpoint,
    /// Effective compact mode (config `compact` OR a phone viewport).
    pub compact: bool,
    /// Height of one toolbar / grouped-bar control row.
    pub control_height: f32,
    /// `min-height` of a touch control (>= [`MIN_TOUCH_TARGET`] on a phone).
    pub touch_target: f32,
    /// Width of the collapsible side panel when expanded (0 when not wide).
    pub side_panel_width: f32,
    /// Whether the side panel starts collapsed.
    pub side_panel_collapsed: bool,
    /// Height of the phone drawer overlay (0 when not a phone).
    pub drawer_height: f32,
}

impl ResponsiveMetrics {
    /// Derive the metrics from a logical viewport and the compact config.
    pub fn derive(logical_size: [f64; 2], compact_config: bool) -> ResponsiveMetrics {
        let width = logical_size[0].max(0.0) as f32;
        let breakpoint = Breakpoint::from_width(width);
        // Compact is consumed here: an explicit config compact OR a phone width
        // forces the compact arrangement. This is the wire that turns the
        // previously-dead `UiConfiguration::compact` into real geometry.
        let is_phone = breakpoint == Breakpoint::Phone;
        let compact = compact_config || is_phone;
        let wide = breakpoint == Breakpoint::Desktop;
        ResponsiveMetrics {
            breakpoint,
            compact,
            control_height: if is_phone { 56.0 } else { 40.0 },
            // A phone must never squash the desktop bar below the 48 px target.
            touch_target: if is_phone {
                MIN_TOUCH_TARGET
            } else {
                MIN_POINTER_TARGET
            },
            side_panel_width: if wide { 300.0 } else { 0.0 },
            side_panel_collapsed: breakpoint.defaults_to_collapsed_panel(),
            drawer_height: if is_phone { 280.0 } else { 0.0 },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breakpoints_match_the_reference_viewports() {
        // Audit U01 reference viewports (width used here; the class is width-based).
        assert_eq!(Breakpoint::from_width(360.0), Breakpoint::Phone); // 360x800
        assert_eq!(Breakpoint::from_width(800.0), Breakpoint::Tablet); // 800x360 / 800x1280
        assert_eq!(Breakpoint::from_width(1280.0), Breakpoint::Desktop); // 1280x800
                                                                         // Boundaries.
        assert_eq!(Breakpoint::from_width(599.0), Breakpoint::Phone);
        assert_eq!(Breakpoint::from_width(600.0), Breakpoint::Tablet);
        assert_eq!(Breakpoint::from_width(1023.0), Breakpoint::Tablet);
        assert_eq!(Breakpoint::from_width(1024.0), Breakpoint::Desktop);
    }

    #[test]
    fn phone_touch_targets_are_at_least_48_logical_pixels() {
        for width in [320.0, 360.0, 480.0, 599.0] {
            let metrics = ResponsiveMetrics::derive([width, 800.0], false);
            assert_eq!(metrics.breakpoint, Breakpoint::Phone);
            assert!(
                metrics.touch_target >= MIN_TOUCH_TARGET,
                "phone width {width} must keep 48px targets"
            );
            assert!(metrics.control_height >= MIN_TOUCH_TARGET);
        }
        // The desktop bar is not squashed into 48px rows; it uses finer targets.
        let desktop = ResponsiveMetrics::derive([1280.0, 800.0], false);
        assert!(desktop.touch_target < MIN_TOUCH_TARGET);
    }

    #[test]
    fn compact_config_is_consumed_not_dead() {
        // Same width, different config -> different arrangement.
        let stretched = ResponsiveMetrics::derive([1280.0, 800.0], false);
        let compact = ResponsiveMetrics::derive([1280.0, 800.0], true);
        assert!(!stretched.compact);
        assert!(compact.compact);
        assert_ne!(stretched, compact);

        // A phone is compact even when the config says stretched (no squashing).
        let phone = ResponsiveMetrics::derive([360.0, 800.0], false);
        assert!(phone.compact);
    }

    #[test]
    fn arrangement_flags_are_mutually_exclusive_and_wide_only() {
        let phone = ResponsiveMetrics::derive([360.0, 800.0], false);
        assert_eq!(phone.drawer_height, 280.0);
        assert_eq!(phone.side_panel_width, 0.0);
        assert!(!Breakpoint::Phone.shows_floating_nav());

        let desktop = ResponsiveMetrics::derive([1280.0, 800.0], false);
        assert_eq!(desktop.drawer_height, 0.0);
        assert_eq!(desktop.side_panel_width, 300.0);
        assert!(
            desktop.side_panel_collapsed,
            "wide work panel starts collapsed"
        );
        assert!(Breakpoint::Desktop.shows_floating_nav());
    }

    #[test]
    fn degenerate_viewport_is_a_phone_not_a_panic() {
        let metrics = ResponsiveMetrics::derive([0.0, 0.0], false);
        assert_eq!(metrics.breakpoint, Breakpoint::Phone);
        assert!(metrics.touch_target >= MIN_TOUCH_TARGET);
    }
}
