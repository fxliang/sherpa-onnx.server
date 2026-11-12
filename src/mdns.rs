use std::collections::HashMap;
use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceInfo};
use tracing::{error, info};

pub const SERVICE_TYPE: &str = "_sherpa-asr._tcp.local.";

pub struct MdnsRegistration {
    daemon: ServiceDaemon,
    service_fullname: String,
}

impl MdnsRegistration {
    pub fn new(port: u16, model_name: Option<&str>, streaming: bool) -> Result<Self> {
        let daemon = ServiceDaemon::new()?;

        let hostname = hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "sherpa-onnx".to_string());

        let host_name = format!("{hostname}.local.");

        let mut properties: HashMap<String, String> = HashMap::new();
        properties.insert("path".to_string(), "/v1/asr".to_string());
        properties.insert(
            "mode".to_string(),
            if streaming {
                "streaming".to_string()
            } else {
                "offline".to_string()
            },
        );
        if let Some(model) = model_name {
            properties.insert("model".to_string(), model.to_string());
        }

        let service_info = ServiceInfo::new(
            SERVICE_TYPE,
            &hostname,
            &host_name,
            "",
            port,
            Some(properties),
        )?
        .enable_addr_auto();

        let service_fullname = service_info.get_fullname().to_string();

        daemon.register(service_info)?;

        info!(
            "mDNS: registered service '{}' type={} host={} port={}",
            service_fullname, SERVICE_TYPE, host_name, port
        );

        Ok(Self {
            daemon,
            service_fullname,
        })
    }
}

impl Drop for MdnsRegistration {
    fn drop(&mut self) {
        info!("mDNS: unregistering service '{}'", self.service_fullname);
        if let Err(e) = self.daemon.unregister(&self.service_fullname) {
            error!("mDNS: failed to unregister: {e}");
        }
        if let Err(e) = self.daemon.shutdown() {
            error!("mDNS: failed to shutdown daemon: {e}");
        }
    }
}
