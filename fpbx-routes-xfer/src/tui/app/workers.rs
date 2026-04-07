use anyhow::Result;
use fpbx_core::ssh::SshSession;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use super::types::{Gateway, GatewayMapping, OutboundRoute, RouteDetail, WorkerState};

pub(super) fn fetch_outbound_routes(host: &str, user: &str) -> Result<Vec<OutboundRoute>, String> {
    let session = SshSession::connect(host, user).map_err(|e| e.to_string())?;

    let sql = "SELECT dialplan_uuid, dialplan_name, COALESCE(dialplan_description,''), \
               dialplan_order, dialplan_enabled, COALESCE(app_uuid,'') \
               FROM v_dialplans dp \
               WHERE dp.dialplan_context = 'global' \
               AND dp.domain_uuid IS NULL \
               AND EXISTS ( \
                 SELECT 1 FROM v_dialplan_details dd \
                 WHERE dd.dialplan_uuid = dp.dialplan_uuid \
                 AND dd.dialplan_detail_type = 'bridge' \
                 AND dd.dialplan_detail_data LIKE '%/gateway/%' \
               ) \
               ORDER BY dialplan_name";

    let cmd = format!(
        "sudo -u postgres psql -d fusionpbx -t -A -F'|' -P pager=off -c \"{}\"",
        sql
    );
    let out = session.exec_ok(&cmd).map_err(|e| e.to_string())?;

    let mut routes = Vec::new();
    for line in out.lines() {
        let p: Vec<&str> = line.splitn(6, '|').collect();
        if p.len() < 6 {
            continue;
        }
        let uuid = p[0].trim().to_string();

        let detail_sql = format!(
            "SELECT dialplan_detail_uuid, dialplan_detail_tag, dialplan_detail_type, \
             dialplan_detail_data, COALESCE(dialplan_detail_break,''), \
             COALESCE(dialplan_detail_inline,''), COALESCE(dialplan_detail_group,'0'), \
             dialplan_detail_order, COALESCE(dialplan_detail_enabled,'true') \
             FROM v_dialplan_details \
             WHERE dialplan_uuid = '{}' \
             ORDER BY dialplan_detail_order",
            uuid
        );
        let detail_cmd = format!(
            "sudo -u postgres psql -d fusionpbx -t -A -F'|' -P pager=off -c \"{}\"",
            detail_sql
        );
        let detail_out = session.exec_ok(&detail_cmd).unwrap_or_default();
        let mut details = Vec::new();
        for dline in detail_out.lines() {
            let dp: Vec<&str> = dline.splitn(9, '|').collect();
            if dp.len() < 9 {
                continue;
            }
            details.push(RouteDetail {
                dialplan_detail_uuid: dp[0].trim().to_string(),
                dialplan_detail_tag: dp[1].trim().to_string(),
                dialplan_detail_type: dp[2].trim().to_string(),
                dialplan_detail_data: dp[3].trim().to_string(),
                dialplan_detail_break: dp[4].trim().to_string(),
                dialplan_detail_inline: dp[5].trim().to_string(),
                dialplan_detail_group: dp[6].trim().to_string(),
                dialplan_detail_order: dp[7].trim().to_string(),
                dialplan_detail_enabled: dp[8].trim().to_string(),
            });
        }

        routes.push(OutboundRoute {
            dialplan_uuid: uuid,
            dialplan_name: p[1].trim().to_string(),
            dialplan_description: p[2].trim().to_string(),
            dialplan_order: p[3].trim().to_string(),
            dialplan_enabled: p[4].trim().to_string(),
            app_uuid: p[5].trim().to_string(),
            details,
            selected: true,
        });
    }
    Ok(routes)
}

pub(super) fn fetch_gateways(host: &str, user: &str) -> Result<Vec<Gateway>, String> {
    let session = SshSession::connect(host, user).map_err(|e| e.to_string())?;
    let cmd = "sudo -u postgres psql -d fusionpbx -t -A -F'|' -P pager=off -c \
               \"SELECT gateway_uuid, gateway FROM v_gateways ORDER BY gateway\"";
    let out = session.exec_ok(cmd).map_err(|e| e.to_string())?;
    let mut gateways = Vec::new();
    for line in out.lines() {
        let p: Vec<&str> = line.splitn(2, '|').collect();
        if p.len() < 2 {
            continue;
        }
        gateways.push(Gateway {
            uuid: p[0].trim().to_string(),
            name: p[1].trim().to_string(),
        });
    }
    Ok(gateways)
}

pub(super) fn build_mappings(
    src_host: &str,
    src_user: &str,
    dst_host: &str,
    dst_user: &str,
    src_uuids: &[String],
) -> Result<Vec<GatewayMapping>, String> {
    let src_gws = fetch_gateways(src_host, src_user)?;
    let dst_gws = fetch_gateways(dst_host, dst_user)?;

    let mut mappings = Vec::new();
    for uuid in src_uuids {
        let src_gw = match src_gws.iter().find(|g| &g.uuid == uuid) {
            Some(g) => g.clone(),
            None => continue,
        };
        let auto_match = dst_gws.iter().position(|g| g.name == src_gw.name);
        let selected_idx = auto_match;
        let list_state = auto_match.unwrap_or(0);
        mappings.push(GatewayMapping {
            source: src_gw,
            dest_options: dst_gws.clone(),
            selected_idx,
            list_state,
        });
    }
    Ok(mappings)
}

pub(super) fn run_transfer(
    dst_host: &str,
    dst_user: &str,
    routes: &[OutboundRoute],
    uuid_remap: &HashMap<String, String>,
    wstate: &Arc<Mutex<WorkerState>>,
) -> Result<()> {
    let log = |msg: &str, progress: f64| {
        let mut w = wstate.lock().unwrap();
        w.log.push(msg.to_string());
        w.current_task = msg.to_string();
        w.progress = progress;
    };

    log("Connecting to destination server…", 0.05);
    let session = SshSession::connect(dst_host, dst_user)?;

    let total = routes.len() as f64;
    for (i, route) in routes.iter().enumerate() {
        let progress = 0.1 + (i as f64 / total) * 0.8;
        log(&format!("Transferring {}…", route.dialplan_name), progress);

        let del_sql = format!(
            "DELETE FROM v_dialplans WHERE dialplan_name = '{}' \
             AND dialplan_context = 'global' AND domain_uuid IS NULL",
            route.dialplan_name.replace('\'', "''")
        );
        let del_cmd = format!(
            "sudo -u postgres psql -d fusionpbx -t -A -P pager=off -c \"{}\"",
            del_sql
        );
        let _ = session.exec(&del_cmd);

        let insert_sql = format!(
            "INSERT INTO v_dialplans \
             (domain_uuid, dialplan_uuid, app_uuid, dialplan_context, dialplan_name, \
              dialplan_order, dialplan_enabled, dialplan_description) \
             VALUES (NULL, '{}', '{}', 'global', '{}', {}, '{}', '{}')",
            route.dialplan_uuid,
            route.app_uuid,
            route.dialplan_name.replace('\'', "''"),
            route.dialplan_order,
            route.dialplan_enabled,
            route.dialplan_description.replace('\'', "''"),
        );
        let insert_cmd = format!(
            "sudo -u postgres psql -d fusionpbx -t -A -P pager=off -c \"{}\"",
            insert_sql
        );
        session
            .exec_ok(&insert_cmd)
            .map_err(|e| anyhow::anyhow!("insert dialplan {}: {}", route.dialplan_name, e))?;

        for detail in &route.details {
            let data = if detail.dialplan_detail_type == "bridge"
                && detail.dialplan_detail_data.contains("/gateway/")
            {
                remap_bridge_uuid(&detail.dialplan_detail_data, uuid_remap)
            } else {
                detail.dialplan_detail_data.clone()
            };

            let detail_sql = format!(
                "INSERT INTO v_dialplan_details \
                 (domain_uuid, dialplan_uuid, dialplan_detail_uuid, dialplan_detail_tag, \
                  dialplan_detail_type, dialplan_detail_data, dialplan_detail_break, \
                  dialplan_detail_inline, dialplan_detail_group, dialplan_detail_order, \
                  dialplan_detail_enabled) \
                 VALUES (NULL, '{}', '{}', '{}', '{}', '{}', '{}', '{}', {}, {}, '{}')",
                route.dialplan_uuid,
                detail.dialplan_detail_uuid,
                detail.dialplan_detail_tag.replace('\'', "''"),
                detail.dialplan_detail_type.replace('\'', "''"),
                data.replace('\'', "''"),
                detail.dialplan_detail_break.replace('\'', "''"),
                detail.dialplan_detail_inline.replace('\'', "''"),
                detail.dialplan_detail_group,
                detail.dialplan_detail_order,
                detail.dialplan_detail_enabled,
            );
            let detail_cmd = format!(
                "sudo -u postgres psql -d fusionpbx -t -A -P pager=off -c \"{}\"",
                detail_sql
            );
            session
                .exec_ok(&detail_cmd)
                .map_err(|e| anyhow::anyhow!("insert detail: {}", e))?;
        }
    }

    log("Reloading FusionPBX XML on destination…", 0.95);
    let _ = session.exec("fs_cli -x 'reloadxml' 2>/dev/null || true");

    Ok(())
}

pub(super) fn remap_bridge_uuid(bridge_data: &str, uuid_remap: &HashMap<String, String>) -> String {
    let parts: Vec<&str> = bridge_data.split('/').collect();
    if let Some(gw_pos) = parts.iter().position(|&p| p == "gateway")
        && let Some(uuid) = parts.get(gw_pos + 1)
        && let Some(new_uuid) = uuid_remap.get(*uuid)
    {
        let mut new_parts = parts.clone();
        new_parts[gw_pos + 1] = new_uuid.as_str();
        return new_parts.join("/");
    }
    bridge_data.to_string()
}

pub(super) fn extract_gateway_uuid(bridge_data: &str) -> Option<String> {
    let parts: Vec<&str> = bridge_data.split('/').collect();
    parts
        .iter()
        .position(|&p| p == "gateway")
        .and_then(|i| parts.get(i + 1))
        .map(|s| s.to_string())
}
