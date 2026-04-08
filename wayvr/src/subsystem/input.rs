use super::hid::{self, HidProvider, VirtualKey};

#[cfg(feature = "tablet")]
use super::hid::tablet::TabletProvider;

use crate::{backend::wayvr::WvrServerState, subsystem::hid::XkbKeymap};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum KeyboardFocus {
    PhysicalScreen,
    WayVR, // (wayland window id data is handled internally),
}

pub struct HidWrapper {
    pub keyboard_focus: KeyboardFocus,
    pub inner: Box<dyn HidProvider>,
    pub keymap: Option<XkbKeymap>,
    #[cfg(feature = "tablet")]
    pub tablet: Option<TabletProvider>,
    pub tablet_mode: bool,
}

impl HidWrapper {
    pub fn new() -> Self {
        Self {
            keyboard_focus: KeyboardFocus::PhysicalScreen,
            inner: hid::initialize(),
            keymap: None,
            #[cfg(feature = "tablet")]
            tablet: None,
            tablet_mode: false,
        }
    }

    /// Initialize the virtual tablet device. Call once after config is loaded.
    #[cfg(feature = "tablet")]
    pub fn init_tablet(&mut self) {
        self.tablet = TabletProvider::try_new();
        if self.tablet.is_none() {
            log::warn!("Tablet mode requested but virtual tablet device could not be created; falling back to mouse.");
            self.tablet_mode = false;
        }
    }

    pub fn set_desktop_extent(&mut self, extent: glam::Vec2) {
        self.inner.set_desktop_extent(extent);
    }

    pub fn set_desktop_origin(&mut self, origin: glam::Vec2) {
        self.inner.set_desktop_origin(origin);
    }

    #[cfg(feature = "tablet")]
    pub fn update_tablet_buttons(
        &mut self,
        stylus_button: bool,
        stylus_button_prev: bool,
        stylus_button2: bool,
        stylus_button2_prev: bool,
    ) -> bool {
        if !self.tablet_mode {
            return false;
        }

        let Some(tablet) = self.tablet.as_mut() else {
            return false;
        };

        let mut changed = false;

        if stylus_button != stylus_button_prev {
            tablet.stylus_button(0, stylus_button);
            changed = true;
        }
        if stylus_button2 != stylus_button2_prev {
            tablet.stylus_button(1, stylus_button2);
            changed = true;
        }
        changed
    }

    pub fn send_key_routed(
        &self,
        wvr_server: Option<&mut WvrServerState>,
        key: VirtualKey,
        down: bool,
    ) {
        match self.keyboard_focus {
            KeyboardFocus::PhysicalScreen => self.inner.send_key(key, down),
            KeyboardFocus::WayVR => {
                if let Some(wvr_server) = wvr_server {
                    wvr_server.send_key(key as u32, down);
                }
            }
        }
    }

    pub fn keymap_changed(&mut self, wvr_server: Option<&mut WvrServerState>, keymap: &XkbKeymap) {
        if let Some(wvr_server) = wvr_server {
            let _ = wvr_server
                .set_keymap(&keymap.inner)
                .inspect_err(|e| log::error!("Could not set WayVR keymap: {e:?}"));
        } else {
            self.keymap = Some(keymap.clone());
        }

        log::info!(
            "Keymap changed: {}",
            keymap.inner.layouts().next().unwrap_or("Unknown")
        );
    }

    pub fn set_modifiers_routed(&mut self, wvr_server: Option<&mut WvrServerState>, mods: u8) {
        match self.keyboard_focus {
            KeyboardFocus::PhysicalScreen => self.inner.set_modifiers(mods),
            KeyboardFocus::WayVR => {
                if let Some(wvr_server) = wvr_server {
                    wvr_server.set_modifiers(mods);
                }
            }
        }
    }
}
