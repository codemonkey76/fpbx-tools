use std::collections::HashMap;


use fpbx_core::{SshHostEntry, WorkerSlot};

#[derive(Debug, Clone, PartialEq)]
pub enum AppScreen {
    Source,
    Dest,
    Routes,
    Gateways,
    Confirm,
    Progress,
    Done,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct OutboundRoute {
    pub dialplan_uuid: String,
    pub app_uuid: String,
    pub dialplan_name: String,
    pub dialplan_description: String,
    pub dialplan_order: String,
    pub dialplan_enabled: String,
    pub details: Vec<RouteDetail>,
    pub selected: bool,
}

#[derive(Debug, Clone)]
pub struct RouteDetail {
    pub dialplan_detail_uuid: String,
    pub dialplan_detail_tag: String,
    pub dialplan_detail_type: String,
    pub dialplan_detail_data: String,
    pub dialplan_detail_break: String,
    pub dialplan_detail_inline: String,
    pub dialplan_detail_group: String,
    pub dialplan_detail_order: String,
    pub dialplan_detail_enabled: String,
}

#[derive(Debug, Clone)]
pub struct Gateway {
    pub uuid: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct GatewayMapping {
    pub source: Gateway,
    pub dest_options: Vec<Gateway>,
    pub selected_idx: Option<usize>,
    pub list_state: usize,
}

impl GatewayMapping {
    pub fn resolved_dest_uuid(&self) -> Option<&str> {
        self.selected_idx
            .and_then(|i| self.dest_options.get(i))
            .map(|g| g.uuid.as_str())
    }
}

pub struct App {
    pub screen: AppScreen,
    pub should_quit: bool,

    // SSH config.
    pub ssh_hosts: HashMap<String, SshHostEntry>,

    // Source screen.
    pub src_host_input: String,
    pub src_user_input: String,
    pub src_active_field: usize,
    pub src_verifying: bool,
    pub src_verify_msg: Option<String>,
    pub src_verify_ok: bool,

    // Dest screen.
    pub dst_host_input: String,
    pub dst_user_input: String,
    pub dst_active_field: usize,
    pub dst_verifying: bool,
    pub dst_verify_msg: Option<String>,
    pub dst_verify_ok: bool,

    // Routes screen.
    pub routes: Vec<OutboundRoute>,
    pub routes_list_idx: usize,
    pub loading_routes: bool,

    // Gateways screen.
    pub gateway_mappings: Vec<GatewayMapping>,
    pub gateway_focus_idx: usize,

    // Progress.
    pub worker: Option<WorkerSlot>,
}
