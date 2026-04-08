//! Virtual pen tablet via uinput, using dt-input's VirtualTablet.
//!
//! Wraps a `dt_input::VirtualTablet` and translates WayVR's UV coordinates
//! (0.0–1.0) and trigger pressure into stylus input events.

use dt_input::{TabletDescriptor, VirtualTablet};
use dt_protocol::InputEvent;

pub struct TabletProvider {
    tablet: VirtualTablet,
    pen_down: bool,
}

impl TabletProvider {
    pub fn try_new() -> Option<Self> {
        // Dimensions are used for reported resolution; coordinate mapping is always 0–1 → 0–65535.
        let descriptor = TabletDescriptor {
            name: "WayVR Stylus".to_string(),
            vid: 0x0000,
            pid: 0x0000,
        };
        match VirtualTablet::new(32768, 32768, &descriptor) {
            Ok(tablet) => {
                log::info!("Tablet mode: initialized virtual stylus device.");
                Some(Self {
                    tablet,
                    pen_down: false,
                })
            }
            Err(e) => {
                log::error!("Tablet mode: could not create virtual tablet device: {e:?}");
                log::error!("Check uinput permissions: is the uinput module loaded and are you in the 'input' group?");
                None
            }
        }
    }

    /// Call each frame when the laser is hovering over a screen in tablet mode.
    ///
    /// `x`, `y` are normalized UV coords (0.0–1.0).
    /// `pressure` is the raw trigger value (0.0–1.0); values below `threshold`
    /// keep the pen hovering without a stroke.
    pub fn on_hover(&mut self, x: f32, y: f32, pressure: f32, threshold: f32) {
        let event = if pressure >= threshold {
            let mapped = (pressure - threshold) / (1.0 - threshold);
            if self.pen_down {
                InputEvent::StylusMove {
                    x,
                    y,
                    pressure: mapped,
                    tilt_x: 0.0,
                    tilt_y: 0.0,
                }
            } else {
                self.pen_down = true;
                InputEvent::StylusDown {
                    x,
                    y,
                    pressure: mapped,
                    tilt_x: 0.0,
                    tilt_y: 0.0,
                }
            }
        } else {
            // Hover — pen in proximity but not drawing
            if self.pen_down {
                self.pen_down = false;
                // Lift the pen first, then send a hover move
                if let Err(e) = self.tablet.process_event(&InputEvent::StylusUp) {
                    log::warn!("Tablet: StylusUp error: {e:?}");
                }
            }
            InputEvent::StylusMove {
                x,
                y,
                pressure: 0.0,
                tilt_x: 0.0,
                tilt_y: 0.0,
            }
        };

        if let Err(e) = self.tablet.process_event(&event) {
            log::warn!("Tablet: event error: {e:?}");
        }
    }

    /// Call when the laser leaves the screen, to lift the pen.
    pub fn on_left(&mut self) {
        if self.pen_down {
            self.pen_down = false;
        }
        if let Err(e) = self.tablet.process_event(&InputEvent::StylusUp) {
            log::warn!("Tablet: StylusUp (on_left) error: {e:?}");
        }
    }

    pub fn stylus_button(&mut self, button: u8, pressed: bool) {
        if let Err(e) = self.tablet.process_event(&InputEvent::StylusButton { button, pressed }) {
            log::warn!("Tablet: StylusButton error: {e:?}");
        }
    }
}
