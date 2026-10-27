//! A companion overlay can provide the environment while WayVR keeps its panels.
//! This changes neither compositor focus nor any client's controller input.
use super::monado_state::MonadoState;
use libmonado::ClientLogic;
use std::time::{Duration, Instant};

pub struct BackgroundOwner {
    pub active: bool,
    next_poll: Instant,
    names: Vec<String>,
}
impl Default for BackgroundOwner {
    fn default() -> Self {
        Self {
            active: false,
            next_poll: Instant::now(),
            // Empty disables cooperation. This is an explicit app-name allowlist,
            // never "any overlay" (which would include WayVR itself).
            names: std::env::var("WAYVR_BACKGROUND_OVERLAYS")
                .unwrap_or_else(|_| "prim".to_owned())
                .split(',')
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect(),
        }
    }
}
impl BackgroundOwner {
    pub fn update(&mut self, monado: Option<&mut MonadoState>) {
        if Instant::now() < self.next_poll {
            return;
        }
        self.next_poll = Instant::now() + Duration::from_millis(500);
        let active = monado
            .and_then(|m| self.find_owner(m).ok())
            .unwrap_or(false);
        if active != self.active {
            log::info!("Companion overlay background owner active={active}");
            self.active = active;
        }
    }
    fn find_owner(&self, monado: &mut MonadoState) -> anyhow::Result<bool> {
        use libmonado::ClientState;
        for mut client in monado.ipc.clients()? {
            let state = client.state()?;
            if state.contains(ClientState::ClientSessionActive)
                && state.contains(ClientState::ClientSessionOverlay)
                && state.contains(ClientState::ClientSessionVisible)
                && self.names.contains(&client.name()?.to_lowercase())
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
