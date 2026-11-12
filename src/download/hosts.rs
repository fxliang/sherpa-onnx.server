use std::collections::HashMap;

pub fn parse_hosts_text(text: &str) -> HashMap<String, String> {
    let mut hosts = HashMap::new();
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        let ip = parts[0];
        for host in &parts[1..] {
            hosts.insert(host.to_string(), ip.to_string());
        }
    }
    hosts
}
