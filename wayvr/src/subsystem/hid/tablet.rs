//! Pen-only virtual tablet device via uinput.
//!
//! Creates a single stylus/pen uinput device (no touch device) so the
//! Wayland compositor does not detect a touchscreen, which would interfere
//! with screencopy. Uses the same input_linux infrastructure as WayVR's
//! existing keyboard/mouse uinput devices.

use std::fs::File;

use glam::Vec2;
use input_linux::{
    AbsoluteAxis, AbsoluteInfo, AbsoluteInfoSetup, EventKind, InputId, InputProperty, Key,
    UInputHandle,
};
use libc::{input_event, timeval};

// Event type codes (same as hid/mod.rs)
const EV_SYN: u16 = 0x0;
const EV_KEY: u16 = 0x1;
const EV_ABS: u16 = 0x3;

// Absolute axis codes
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_PRESSURE: u16 = 0x18;
const ABS_TILT_X: u16 = 0x1a;
const ABS_TILT_Y: u16 = 0x1b;

// Key codes
const BTN_TOOL_PEN: u16 = 0x140;
const BTN_TOUCH: u16 = 0x14a;
const BTN_STYLUS: u16 = 0x14b;
const BTN_STYLUS2: u16 = 0x14c;

const ABS_MAX: i32 = 65535;
const PRESSURE_MAX: i32 = 65535;
const TILT_MAX: i32 = 63;

pub struct TabletProvider {
    pen_handle: UInputHandle<File>,
    pen_down: bool,
    in_proximity: bool,
}

impl TabletProvider {
    pub fn try_new() -> Option<Self> {
        let pen_file = File::create("/dev/uinput").ok()?;
        let pen_handle = UInputHandle::new(pen_file);

        // Key events: pen tool presence, touch contact, stylus buttons
        pen_handle.set_evbit(EventKind::Key).ok()?;
        pen_handle.set_keybit(Key::ButtonToolPen).ok()?;
        pen_handle.set_keybit(Key::ButtonTouch).ok()?;
        pen_handle.set_keybit(Key::ButtonStylus).ok()?;
        pen_handle.set_keybit(Key::ButtonStylus2).ok()?;

        // Absolute axes: position, pressure, tilt
        pen_handle.set_evbit(EventKind::Absolute).ok()?;

        // This behaves like a direct pen display: when the controller ray hits a
        // specific virtual screen, we send screen-local absolute coordinates for
        // that hovered surface rather than normalising across the whole desktop.
        pen_handle.set_propbit(InputProperty::Direct).ok()?;
        pen_handle.set_propbit(InputProperty::Pointer).ok()?;

        let abs_info = [
            AbsoluteInfoSetup {
                axis: AbsoluteAxis::X,
                info: AbsoluteInfo {
                    value: 0,
                    minimum: 0,
                    maximum: ABS_MAX,
                    fuzz: 0,
                    flat: 0,
                    resolution: 400,
                },
            },
            AbsoluteInfoSetup {
                axis: AbsoluteAxis::Y,
                info: AbsoluteInfo {
                    value: 0,
                    minimum: 0,
                    maximum: ABS_MAX,
                    fuzz: 0,
                    flat: 0,
                    resolution: 400,
                },
            },
            AbsoluteInfoSetup {
                axis: AbsoluteAxis::Pressure,
                info: AbsoluteInfo {
                    value: 0,
                    minimum: 0,
                    maximum: PRESSURE_MAX,
                    fuzz: 0,
                    flat: 0,
                    resolution: 0,
                },
            },
            AbsoluteInfoSetup {
                axis: AbsoluteAxis::TiltX,
                info: AbsoluteInfo {
                    value: 0,
                    minimum: -TILT_MAX,
                    maximum: TILT_MAX,
                    fuzz: 0,
                    flat: 0,
                    resolution: 0,
                },
            },
            AbsoluteInfoSetup {
                axis: AbsoluteAxis::TiltY,
                info: AbsoluteInfo {
                    value: 0,
                    minimum: -TILT_MAX,
                    maximum: TILT_MAX,
                    fuzz: 0,
                    flat: 0,
                    resolution: 0,
                },
            },
        ];

        let pen_id = InputId {
            bustype: 0x1c, // BUS_VIRTUAL — honest about being a virtual device
            vendor: 0x0000,
            product: 0x0000,
            version: 0x0100,
        };

        pen_handle
            .create(&pen_id, b"WayVR Stylus\0", 0, &abs_info)
            .ok()?;

        log::info!("Tablet mode: initialized virtual stylus device (pen-only, no touch).");
        Some(Self {
            pen_handle,
            pen_down: false,
            in_proximity: false,
        })
    }

    /// Call each frame when the laser is hovering over a screen in tablet mode.
    ///
    /// `pos` is the hovered screen-local coordinate in normalized UV space.
    /// `pressure` is already normalized to 0.0–1.0 by the caller.
    pub fn on_hover(&mut self, pos: Vec2, pressure: f32, tilt: Vec2) {
        let time = get_time();
        let abs_x = (pos.x * ABS_MAX as f32).clamp(0.0, ABS_MAX as f32) as i32;
        let abs_y = (pos.y * ABS_MAX as f32).clamp(0.0, ABS_MAX as f32) as i32;
        let tilt_x = tilt.x.clamp(-TILT_MAX as f32, TILT_MAX as f32) as i32;
        let tilt_y = tilt.y.clamp(-TILT_MAX as f32, TILT_MAX as f32) as i32;

        if !self.in_proximity {
            self.in_proximity = true;
            self.emit(&[
                ev(time, EV_KEY, BTN_TOOL_PEN, 1),
                ev(time, EV_SYN, 0, 0),
            ]);
        }

        if pressure > 0.0 {
            let mapped = (pressure * PRESSURE_MAX as f32).clamp(0.0, PRESSURE_MAX as f32) as i32;
            if !self.pen_down {
                self.pen_down = true;
                self.emit(&[ev(time, EV_KEY, BTN_TOUCH, 1)]);
            }
            self.emit(&[
                ev(time, EV_ABS, ABS_X, abs_x),
                ev(time, EV_ABS, ABS_Y, abs_y),
                ev(time, EV_ABS, ABS_PRESSURE, mapped),
                ev(time, EV_ABS, ABS_TILT_X, tilt_x),
                ev(time, EV_ABS, ABS_TILT_Y, tilt_y),
                ev(time, EV_SYN, 0, 0),
            ]);
        } else {
            if self.pen_down {
                self.pen_down = false;
                self.emit(&[
                    ev(time, EV_KEY, BTN_TOUCH, 0),
                    ev(time, EV_ABS, ABS_PRESSURE, 0),
                    ev(time, EV_SYN, 0, 0),
                ]);
            }
            self.emit(&[
                ev(time, EV_ABS, ABS_X, abs_x),
                ev(time, EV_ABS, ABS_Y, abs_y),
                ev(time, EV_ABS, ABS_TILT_X, tilt_x),
                ev(time, EV_ABS, ABS_TILT_Y, tilt_y),
                ev(time, EV_SYN, 0, 0),
            ]);
        }
    }

    /// Call when the laser leaves the screen, to lift the pen out of proximity.
    pub fn on_left(&mut self) {
        if !self.in_proximity {
            return;
        }
        let time = get_time();
        if self.pen_down {
            self.pen_down = false;
            self.emit(&[
                ev(time, EV_KEY, BTN_TOUCH, 0),
                ev(time, EV_ABS, ABS_PRESSURE, 0),
                ev(time, EV_SYN, 0, 0),
            ]);
        }
        self.in_proximity = false;
        self.emit(&[
            ev(time, EV_KEY, BTN_TOOL_PEN, 0),
            ev(time, EV_SYN, 0, 0),
        ]);
    }

    pub fn stylus_button(&mut self, button: u8, pressed: bool) {
        let code = match button {
            0 => BTN_STYLUS,
            1 => BTN_STYLUS2,
            _ => {
                log::warn!("Tablet: unknown stylus button {button}");
                return;
            }
        };
        let time = get_time();
        self.emit(&[ev(time, EV_KEY, code, pressed.into()), ev(time, EV_SYN, 0, 0)]);
    }

    fn emit(&self, events: &[input_event]) {
        if let Err(e) = self.pen_handle.write(events) {
            log::warn!("Tablet: uinput write error: {e}");
        }
    }
}

#[inline]
fn get_time() -> timeval {
    let mut time = timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    unsafe { libc::gettimeofday(&raw mut time, std::ptr::null_mut()) };
    time
}

#[inline]
const fn ev(time: timeval, type_: u16, code: u16, value: i32) -> input_event {
    input_event {
        time,
        type_,
        code,
        value,
    }
}
