use log::warn;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::Path};

use crate::proxy::ProxyRule;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub port: u16,
    pub proxy_rules: Vec<ProxyRule>,
    pub proxy_timeout: u64,
    pub worker_processes: usize,
    pub root: String,
    pub log: Option<String>,
    pub max_connections: usize,
    pub keep_alive_timeout: u64,
    pub send_timeout: u64,
    pub client_max_body_size: usize,
    pub worker_connections: usize,
    pub cache_size: usize,
    pub cache_ttl: u64,
    pub sendfile: bool,
    pub tcp_nodelay: bool,
    pub tcp_nopush: bool,
    pub backlog: i32,
}

impl Default for Config {
    fn default() -> Self {
        let num_cpus = num_cpus::get();
        Self {
            port: 7888,
            proxy_rules: Vec::new(),
            proxy_timeout: 30,
            worker_processes: if num_cpus > 0 { num_cpus } else { 1 },
            root: "./static".to_string(),
            log: Some("./logs/access.log".to_string()),
            max_connections: 100_000,
            keep_alive_timeout: 120,
            send_timeout: 60,
            client_max_body_size: 1024 * 1024, // 1MB
            worker_connections: 1024,
            cache_size: 100 * 1024 * 1024, // 100MB
            cache_ttl: 3600,               // 1 Hour
            sendfile: true,
            tcp_nodelay: true,
            tcp_nopush: false,
            backlog: 65536,
        }
    }
}

impl Config {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        // box = heap
        // dyn dispatch = overhead runtime
        // use type error statically typed (enum or something else)
        let content = fs::read_to_string(path)?;
        let mut config = Config::default();

        // parse key:value format
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some(pos) = line.find('=') {
                let key = line[..pos].trim();
                let value = line[pos + 1..].trim();

                match key {
                    "port" => config.port = value.parse()?,
                    "worker_processes" => {
                        if value == "auto" {
                            config.worker_processes = num_cpus::get();
                        } else {
                            config.worker_processes = value.parse()?;
                        }
                    }
                    "root" => config.root = value.to_string(),
                    "log" => config.log = Some(value.to_string()),
                    "max_connections" => config.max_connections = value.parse()?,
                    "keep_alive_timeout" => config.keep_alive_timeout = value.parse()?,
                    "send_timeout" => config.send_timeout = value.parse()?,
                    "client_max_body_size" => config.client_max_body_size = value.parse()?,
                    "worker_connections" => config.worker_connections = value.parse()?,
                    "cache_size" => config.cache_size = value.parse()?,
                    "cache_ttl" => config.cache_ttl = value.parse()?,
                    "sendfile" => config.sendfile = value.parse()?,
                    "tcp_nodelay" => config.tcp_nodelay = value.parse()?,
                    "tcp_nopush" => config.tcp_nopush = value.parse()?,
                    "backlog" => config.backlog = value.parse()?,
                    "proxy_timeout" => config.proxy_timeout = value.parse()?,
                    "proxy" => {
                        config.proxy_rules.push(Self::parse_proxy_rule(value)?);
                    }
                    _ => warn!("unknown configuration key: {key}"),
                }
            }
        }

        Ok(config)
    }

    fn parse_proxy_rule(rule_str: &str) -> Result<ProxyRule, Box<dyn std::error::Error>> {
        let parts: Vec<&str> = rule_str.split(',').collect();
        if parts.is_empty() {
            return Err("invalid proxy rule format".into());
        }

        // parse path=backend format
        let main_part = parts[0];
        let (path_prefix, backend_url) = if let Some(pos) = main_part.find('=') {
            (
                main_part[..pos].to_string(),
                main_part[pos + 1..].to_string(),
            )
        } else {
            return Err("proxy rule must contain path=backend".into());
        };

        let mut strip_prefix = false;
        let mut headers = HashMap::new();

        // parse
        for part in &parts[1..] {
            if let Some(pos) = part.find('=') {
                let key = part[..pos].trim();
                let value = part[pos + 1..].trim();

                match key {
                    "strip_prefix" => strip_prefix = value.parse().unwrap_or(false),
                    _ => {
                        headers.insert(key.to_string(), value.to_string());
                    }
                }
            }
        }

        Ok(ProxyRule {
            path_prefix,
            backend_url,
            strip_prefix,
            headers,
        })
    }
}
