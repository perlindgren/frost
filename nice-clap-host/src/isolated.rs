//! Optional, rack-owned process isolation. Native plugin code stays in helpers.
mod client;
mod helper;
mod wire;
use super::*;
pub use client::finish_shutdown;
pub(crate) use client::{RemoteEntry, RemoteOwner};
pub use helper::run_helper;
use rack_ipc::{AudioMemory, Layout};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant};
use wire::{Class, Command, Config, Reply};

/// Geometry belongs to the helper's independent top-level editor window.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Geometry {
    pub position: Option<[i32; 2]>,
    pub size: [u32; 2],
}
impl LiveSession {
    /// Poll a full remote capture without entering foreign code or waiting on
    /// the rack UI thread. Keep servicing until ready before retiring helpers.
    pub fn isolated_snapshot_ready(&mut self) -> Result<bool, String> {
        let mut ready = true;
        for owner in self.remote_owners.values_mut() {
            ready &= owner.snapshot_ready()?;
        }
        Ok(ready)
    }

    pub fn isolated_editor_errors(&mut self) -> Vec<String> {
        self.remote_owners
            .iter_mut()
            .filter_map(|(&id, owner)| {
                owner
                    .editor_error
                    .take()
                    .map(|error| format!("Editor {id:?}: {error}"))
            })
            .collect()
    }
    pub fn closed_isolated_editors(&mut self) -> Vec<nice_rack_engine::NodeId> {
        self.remote_owners
            .iter_mut()
            .filter_map(|(&id, owner)| std::mem::take(&mut owner.closed).then_some(id))
            .collect()
    }
    pub fn editor_is_isolated(&self, id: nice_rack_engine::NodeId) -> bool {
        self.remote_owners.contains_key(&id)
    }
    pub fn open_isolated_editor(
        &mut self,
        id: nice_rack_engine::NodeId,
        geometry: Geometry,
    ) -> Result<(), String> {
        self.remote_owners
            .get_mut(&id)
            .ok_or("Isolated plugin not found")?
            .open(geometry)
    }
    pub fn isolated_geometry(&self, id: nice_rack_engine::NodeId) -> Option<Geometry> {
        self.remote_owners.get(&id)?.geometry
    }
}
