//! ext-workspace-v1: bars and docks that aren't Rivet's own ask here which workspaces there
//! are, and for one to be shown.
//!
//! Each display is a group and its workspaces are the group's. After every batch of events
//! what each client was last told is compared with the layout, and it is sent the difference.

use std::collections::HashMap;

use smithay::output::Output;
use smithay::reexports::wayland_protocols::ext::workspace::v1::server::ext_workspace_group_handle_v1::{self, ExtWorkspaceGroupHandleV1, GroupCapabilities};
use smithay::reexports::wayland_protocols::ext::workspace::v1::server::ext_workspace_handle_v1::{self, ExtWorkspaceHandleV1, WorkspaceCapabilities};
use smithay::reexports::wayland_protocols::ext::workspace::v1::server::ext_workspace_manager_v1::{self, ExtWorkspaceManagerV1};
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use crate::state::State;

#[derive(Default)]
pub struct Workspaces {
    managers: Vec<Manager>,
    /// Workspaces asked for, waiting for their client's commit.
    asked: Vec<(ClientId, u64)>,
}

struct Manager {
    obj: ExtWorkspaceManagerV1,
    /// By the display's name.
    groups: HashMap<String, Group>,
    /// By the workspace's id.
    wss: HashMap<u64, Ws>,
}

struct Group {
    obj: ExtWorkspaceGroupHandleV1,
    outputs: Vec<WlOutput>,
}

struct Ws {
    obj: ExtWorkspaceHandleV1,
    group: Option<String>,
    idx: Option<u32>,
    name: String,
    active: Option<bool>,
}

pub struct Global;
pub struct ManagerData;
pub struct GroupData;
/// The workspace's id.
pub struct WsData(u64);

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<State, ExtWorkspaceManagerV1, _>(1, Global);
}

impl GlobalDispatch2<ExtWorkspaceManagerV1, State> for Global {
    fn bind(&self, state: &mut State, _: &DisplayHandle, _: &Client, resource: New<ExtWorkspaceManagerV1>, data_init: &mut DataInit<'_, State>) {
        let obj = data_init.init(resource, ManagerData);
        // The next refresh tells it everything.
        state.workspaces.managers.push(Manager { obj, groups: HashMap::new(), wss: HashMap::new() });
    }
}

impl Dispatch2<ExtWorkspaceManagerV1, State> for ManagerData {
    fn request(&self, state: &mut State, client: &Client, resource: &ExtWorkspaceManagerV1, request: ext_workspace_manager_v1::Request, _: &DisplayHandle, _: &mut DataInit<'_, State>) {
        match request {
            ext_workspace_manager_v1::Request::Commit => {
                let id = client.id();
                let mut asked = Vec::new();
                state.workspaces.asked.retain(|(c, ws)| {
                    if *c == id {
                        asked.push(*ws);
                    }
                    *c != id
                });
                // The last one asked for is the one left showing.
                for ws in asked {
                    show(state, ws);
                }
            }
            ext_workspace_manager_v1::Request::Stop => {
                state.workspaces.managers.retain(|m| m.obj != *resource);
                resource.finished();
            }
            _ => {}
        }
    }

    fn destroyed(&self, state: &mut State, client: ClientId, resource: &ExtWorkspaceManagerV1) {
        state.workspaces.managers.retain(|m| m.obj != *resource);
        state.workspaces.asked.retain(|(c, _)| *c != client);
    }
}

impl Dispatch2<ExtWorkspaceGroupHandleV1, State> for GroupData {
    // Softcore makes workspaces as they are needed: there is always an empty one last.
    fn request(&self, _: &mut State, _: &Client, _: &ExtWorkspaceGroupHandleV1, _: ext_workspace_group_handle_v1::Request, _: &DisplayHandle, _: &mut DataInit<'_, State>) {}
}

impl Dispatch2<ExtWorkspaceHandleV1, State> for WsData {
    fn request(&self, state: &mut State, client: &Client, _: &ExtWorkspaceHandleV1, request: ext_workspace_handle_v1::Request, _: &DisplayHandle, _: &mut DataInit<'_, State>) {
        if let ext_workspace_handle_v1::Request::Activate = request {
            state.workspaces.asked.push((client.id(), self.0));
        }
    }
}

/// Shows a workspace on its display, and gives that display the focus.
fn show(state: &mut State, id: u64) {
    let found = state.layout.mons.iter().enumerate().find_map(|(mi, m)| m.wss.iter().position(|ws| ws.id == id).map(|wi| (mi, wi)));
    let Some((mi, wi)) = found else { return };
    state.layout.active_mon = mi;
    state.layout.focus_workspace(wi);
    state.layer_focus = None;
    state.queue_redraw();
}

/// Tells every client what has changed since it was last told.
pub fn refresh(state: &mut State) {
    if state.workspaces.managers.is_empty() {
        return;
    }
    // Each display with its workspaces: id, place in the stack, showing or not.
    let want: Vec<(String, Option<Output>, Vec<(u64, u32, bool, Option<String>)>)> = state
        .layout
        .mons
        .iter()
        .filter(|m| !m.orphan)
        .map(|m| {
            let output = state.outputs.iter().find(|o| o.name() == m.name).cloned();
            (m.name.clone(), output, m.wss.iter().enumerate().map(|(wi, ws)| (ws.id, wi as u32, wi == m.active, ws.name.clone())).collect())
        })
        .collect();
    let dh = state.dh.clone();
    state.workspaces.managers.retain(|m| m.obj.is_alive());
    for m in &mut state.workspaces.managers {
        let Some(client) = m.obj.client() else { continue };
        let version = m.obj.version();
        let mut changed = false;

        for (name, output, _) in &want {
            if !m.groups.contains_key(name) {
                let Ok(obj) = client.create_resource::<ExtWorkspaceGroupHandleV1, _, State>(&dh, version, GroupData) else { continue };
                m.obj.workspace_group(&obj);
                obj.capabilities(GroupCapabilities::empty());
                m.groups.insert(name.clone(), Group { obj, outputs: Vec::new() });
                changed = true;
            }
            // The client may bind the display after the group, or more than once.
            let Some(g) = m.groups.get_mut(name) else { continue };
            let bound: Vec<WlOutput> = output.as_ref().map(|o| o.client_outputs(&client).collect()).unwrap_or_default();
            for o in &bound {
                if !g.outputs.contains(o) {
                    g.obj.output_enter(o);
                    changed = true;
                }
            }
            for o in &g.outputs {
                if !bound.contains(o) && o.is_alive() {
                    g.obj.output_leave(o);
                    changed = true;
                }
            }
            g.outputs = bound;
        }

        for (name, _, wss) in &want {
            for (id, idx, active, called) in wss {
                let (id, idx, active) = (*id, *idx, *active);
                if !m.wss.contains_key(&id) {
                    let Ok(obj) = client.create_resource::<ExtWorkspaceHandleV1, _, State>(&dh, version, WsData(id)) else { continue };
                    m.obj.workspace(&obj);
                    obj.capabilities(WorkspaceCapabilities::Activate);
                    m.wss.insert(id, Ws { obj, group: None, idx: None, name: String::new(), active: None });
                    changed = true;
                }
                let Some(ws) = m.wss.get_mut(&id) else { continue };
                let called = called.clone().unwrap_or_else(|| (idx + 1).to_string());
                if ws.idx != Some(idx) || ws.name != called {
                    ws.obj.name(called.clone());
                    ws.name = called;
                    ws.obj.coordinates(idx.to_ne_bytes().to_vec());
                    ws.idx = Some(idx);
                    changed = true;
                }
                if ws.active != Some(active) {
                    ws.obj.state(if active { ext_workspace_handle_v1::State::Active } else { ext_workspace_handle_v1::State::empty() });
                    ws.active = Some(active);
                    changed = true;
                }
                if ws.group.as_ref() != Some(name) {
                    if let Some(old) = ws.group.take().and_then(|old| m.groups.get(&old)) {
                        old.obj.workspace_leave(&ws.obj);
                    }
                    if let Some(g) = m.groups.get(name) {
                        g.obj.workspace_enter(&ws.obj);
                        ws.group = Some(name.clone());
                    }
                    changed = true;
                }
            }
        }

        // What has gone: a workspace leaves its group before it is removed, and a group is
        // empty before it is.
        let groups = &m.groups;
        m.wss.retain(|id, ws| {
            let stays = want.iter().any(|(_, _, wss)| wss.iter().any(|w| w.0 == *id));
            if !stays {
                if let Some(g) = ws.group.as_ref().and_then(|g| groups.get(g)) {
                    g.obj.workspace_leave(&ws.obj);
                }
                ws.obj.removed();
                changed = true;
            }
            stays
        });
        m.groups.retain(|name, g| {
            let stays = want.iter().any(|(n, _, _)| n == name);
            if !stays {
                g.obj.removed();
                changed = true;
            }
            stays
        });

        if changed {
            m.obj.done();
        }
    }
}
