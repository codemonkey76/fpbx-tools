use std::collections::HashMap;

use super::types::SshHostEntry;

pub(super) fn parse_ssh_config() -> HashMap<String, SshHostEntry> {
    let mut map = HashMap::new();
    let config_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".ssh")
        .join("config");
    let Ok(content) = std::fs::read_to_string(&config_path) else {
        return map;
    };
    let mut current_alias: Option<String> = None;
    let mut current_hostname: Option<String> = None;
    let mut current_user: Option<String> = None;

    let flush = |map: &mut HashMap<String, SshHostEntry>,
                 alias: &mut Option<String>,
                 hostname: &mut Option<String>,
                 user: &mut Option<String>| {
        if let (Some(a), Some(h), Some(u)) = (alias.take(), hostname.take(), user.take()) {
            map.insert(
                a.to_lowercase(),
                SshHostEntry {
                    hostname: h,
                    user: u,
                },
            );
        }
    };

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, val) = match line.split_once(|c: char| c.is_whitespace()) {
            Some(pair) => (pair.0.to_lowercase(), pair.1.trim().to_string()),
            None => continue,
        };
        match key.as_str() {
            "host" => {
                flush(
                    &mut map,
                    &mut current_alias,
                    &mut current_hostname,
                    &mut current_user,
                );
                if !val.contains('*') {
                    current_alias = Some(val);
                }
            }
            "hostname" => {
                current_hostname = Some(val);
            }
            "user" => {
                current_user = Some(val);
            }
            _ => {}
        }
    }
    flush(
        &mut map,
        &mut current_alias,
        &mut current_hostname,
        &mut current_user,
    );
    map
}
