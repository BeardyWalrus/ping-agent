//! Home Assistant integration over MQTT. The device and its sensors are created
//! automatically through Home Assistant's MQTT discovery; after that every ping
//! result is published as one JSON state message.

use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rumqttc::{Client, Event, LastWill, MqttOptions, Packet, QoS};
use serde::{Deserialize, Serialize};

use crate::secret;

pub const TOPIC_PREFIX: &str = "pingagent";
pub const DISCOVERY_PREFIX: &str = "homeassistant";
pub const PROJECT_URL: &str = "https://github.com/BeardyWalrus/ping-agent";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MqttConfig {
    pub enabled: bool,
    /// Broker host name or IP (usually your Home Assistant box).
    pub host: String,
    pub port: u16,
    pub username: String,
    /// Stored protected with Windows DPAPI as "dpapi:<base64>"; see `secret`.
    pub password: String,
    /// How this PC appears in Home Assistant. Empty means the computer name.
    pub device_name: String,
}

impl Default for MqttConfig {
    fn default() -> Self {
        MqttConfig {
            enabled: false,
            host: String::new(),
            port: 1883,
            username: String::new(),
            password: String::new(),
            device_name: String::new(),
        }
    }
}

impl MqttConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        if self.host.trim().is_empty() {
            return Err(
                "Enter the MQTT broker's address (usually your Home Assistant host).".into(),
            );
        }
        if self.port == 0 {
            return Err("The MQTT port must be between 1 and 65535.".into());
        }
        Ok(())
    }

    pub fn effective_device_name(&self) -> String {
        let n = self.device_name.trim();
        if !n.is_empty() {
            return n.to_string();
        }
        std::env::var("COMPUTERNAME")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "pc".to_string())
    }
}

/// Lower-case `[a-z0-9_]` identifier, for topics and unique ids.
pub fn sanitize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_us = true;
    for c in s.trim().chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_us = false;
        } else if !last_us {
            out.push('_');
            last_us = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        "unknown".to_string()
    } else {
        out
    }
}

/// Everything needed to name topics and entities for one PC + target pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub device_name: String,
    pub device_id: String,
    pub target_host: String,
    pub target_id: String,
}

impl Identity {
    pub fn new(cfg: &MqttConfig, target_host: &str) -> Identity {
        let device_name = cfg.effective_device_name();
        Identity {
            device_id: sanitize(&device_name),
            device_name,
            target_host: target_host.to_string(),
            target_id: sanitize(target_host),
        }
    }

    pub fn availability_topic(&self) -> String {
        format!("{TOPIC_PREFIX}/{}/availability", self.device_id)
    }

    pub fn state_topic(&self) -> String {
        format!("{TOPIC_PREFIX}/{}/{}/state", self.device_id, self.target_id)
    }

    /// Prefix shared by this target's entity ids and discovery topics.
    fn object_base(&self) -> String {
        format!("pingagent_{}_{}", self.device_id, self.target_id)
    }

    fn unique_id(&self, suffix: &str) -> String {
        format!("{}_{suffix}", self.object_base())
    }
}

/// One ping result plus the rolling figures, as published to the state topic.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub latency_ms: Option<u64>,
    pub reachable: bool,
    /// "reply", "timeout" or "error"
    pub result: &'static str,
    pub loss_pct: f64,
    pub avg_ms: Option<f64>,
    pub mode: String,
}

pub fn state_payload(id: &Identity, s: &Sample) -> String {
    let opt_u = |v: Option<u64>| v.map(|n| n.to_string()).unwrap_or_else(|| "null".into());
    let opt_f = |v: Option<f64>| {
        v.map(|n| format!("{n:.1}"))
            .unwrap_or_else(|| "null".into())
    };
    format!(
        "{{\"host\":{},\"latency_ms\":{},\"reachable\":{},\"result\":\"{}\",\"loss_pct\":{:.1},\"avg_ms\":{},\"mode\":{}}}",
        json_str(&id.target_host),
        opt_u(s.latency_ms),
        s.reachable,
        s.result,
        s.loss_pct,
        opt_f(s.avg_ms),
        json_str(&s.mode),
    )
}

/// Retained discovery messages: (topic, payload) for each entity.
pub fn discovery_messages(id: &Identity, version: &str) -> Vec<(String, String)> {
    let device = format!(
        "\"device\":{{\"identifiers\":[\"pingagent_{}\"],\"name\":{},\"manufacturer\":\"PingAgent\",\"model\":\"PingAgent for Windows\",\"sw_version\":\"{version}\"}},\"origin\":{{\"name\":\"PingAgent\",\"sw\":\"{version}\",\"url\":\"{PROJECT_URL}\"}}",
        id.device_id,
        json_str(&format!("{} ping", id.device_name)),
    );
    let common = format!(
        "\"state_topic\":\"{st}\",\"availability_topic\":\"{av}\",\"json_attributes_topic\":\"{st}\",{device}",
        st = id.state_topic(),
        av = id.availability_topic(),
    );
    let host = &id.target_host;
    let entity = |component: &str, suffix: &str, extra: String| {
        let topic = format!(
            "{DISCOVERY_PREFIX}/{component}/{}/{suffix}/config",
            id.object_base()
        );
        let payload = format!(
            "{{\"unique_id\":\"{}\",{extra},{common}}}",
            id.unique_id(suffix)
        );
        (topic, payload)
    };
    vec![
        entity(
            "sensor",
            "latency",
            format!(
                "\"name\":{},\"value_template\":\"{{{{ value_json.latency_ms }}}}\",\"unit_of_measurement\":\"ms\",\"state_class\":\"measurement\",\"icon\":\"mdi:lan-connect\"",
                json_str(&format!("{host} latency"))
            ),
        ),
        entity(
            "sensor",
            "average",
            format!(
                "\"name\":{},\"value_template\":\"{{{{ value_json.avg_ms }}}}\",\"unit_of_measurement\":\"ms\",\"state_class\":\"measurement\",\"icon\":\"mdi:chart-line\",\"entity_category\":\"diagnostic\"",
                json_str(&format!("{host} average latency"))
            ),
        ),
        entity(
            "sensor",
            "loss",
            format!(
                "\"name\":{},\"value_template\":\"{{{{ value_json.loss_pct }}}}\",\"unit_of_measurement\":\"%\",\"state_class\":\"measurement\",\"icon\":\"mdi:lan-disconnect\"",
                json_str(&format!("{host} packet loss"))
            ),
        ),
        entity(
            "binary_sensor",
            "reachable",
            format!(
                "\"name\":{},\"value_template\":\"{{{{ 'ON' if value_json.reachable else 'OFF' }}}}\",\"device_class\":\"connectivity\"",
                json_str(&format!("{host} reachable"))
            ),
        ),
    ]
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ---------------------------------------------------------------------------
// Background publisher
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    pub enabled: bool,
    pub connected: bool,
    pub last_error: Option<String>,
    pub published: u64,
}

enum Cmd {
    Sample(Sample),
    Reconfigure(MqttConfig, String),
    Quit,
}

/// Handle owned by the UI; the connection lives on its own threads.
pub struct Publisher {
    tx: Sender<Cmd>,
    status: Arc<Mutex<Status>>,
    worker: Option<JoinHandle<()>>,
}

impl Publisher {
    /// `on_status_change` runs on the publisher thread whenever `status()` changes.
    pub fn start(
        cfg: MqttConfig,
        target_host: String,
        on_status_change: impl Fn() + Send + Sync + 'static,
    ) -> Publisher {
        let (tx, rx) = channel();
        let status = Arc::new(Mutex::new(Status {
            enabled: cfg.enabled,
            ..Default::default()
        }));
        let st = status.clone();
        let worker = std::thread::Builder::new()
            .name("mqtt".into())
            .spawn(move || run(cfg, target_host, rx, st, on_status_change))
            .expect("failed to start the MQTT thread");
        Publisher {
            tx,
            status,
            worker: Some(worker),
        }
    }

    pub fn status(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn publish(&self, sample: Sample) {
        let _ = self.tx.send(Cmd::Sample(sample));
    }

    pub fn reconfigure(&self, cfg: MqttConfig, target_host: String) {
        let _ = self.tx.send(Cmd::Reconfigure(cfg, target_host));
    }

    /// Ask the thread to say goodbye to the broker and wait briefly for it.
    pub fn shutdown(&mut self) {
        let _ = self.tx.send(Cmd::Quit);
        if let Some(w) = self.worker.take() {
            let deadline = Instant::now() + Duration::from_millis(1500);
            while !w.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

fn set_status(status: &Mutex<Status>, notify: &dyn Fn(), f: impl FnOnce(&mut Status)) {
    let changed = {
        let mut s = status.lock().unwrap();
        let before = s.clone();
        f(&mut s);
        *s != before
    };
    if changed {
        notify();
    }
}

fn run(
    mut cfg: MqttConfig,
    mut target_host: String,
    rx: Receiver<Cmd>,
    status: Arc<Mutex<Status>>,
    notify: impl Fn() + Send + Sync + 'static,
) {
    let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(notify);
    'sessions: loop {
        if !cfg.enabled {
            set_status(&status, &*notify, |s| {
                *s = Status {
                    enabled: false,
                    ..Default::default()
                };
            });
            // Idle until a reconfigure or quit arrives.
            loop {
                match rx.recv() {
                    Ok(Cmd::Reconfigure(c, h)) => {
                        cfg = c;
                        target_host = h;
                        continue 'sessions;
                    }
                    Ok(Cmd::Quit) | Err(_) => return,
                    Ok(Cmd::Sample(_)) => {}
                }
            }
        }

        let id = Identity::new(&cfg, &target_host);
        let password = match secret::reveal(&cfg.password) {
            Ok(p) => p,
            Err(e) => {
                set_status(&status, &*notify, |s| {
                    s.enabled = true;
                    s.connected = false;
                    s.last_error = Some(e);
                });
                // Wait for new settings.
                loop {
                    match rx.recv() {
                        Ok(Cmd::Reconfigure(c, h)) => {
                            cfg = c;
                            target_host = h;
                            continue 'sessions;
                        }
                        Ok(Cmd::Quit) | Err(_) => return,
                        Ok(Cmd::Sample(_)) => {}
                    }
                }
            }
        };

        let client_id = format!("pingagent-{}-{}", id.device_id, std::process::id());
        let mut options = MqttOptions::new(client_id, cfg.host.trim(), cfg.port);
        options.set_keep_alive(Duration::from_secs(30));
        options.set_clean_session(true);
        options.set_last_will(LastWill::new(
            id.availability_topic(),
            "offline",
            QoS::AtLeastOnce,
            true,
        ));
        if !cfg.username.trim().is_empty() {
            options.set_credentials(cfg.username.trim(), password);
        }
        let (client, mut connection) = Client::new(options, 64);
        set_status(&status, &*notify, |s| {
            s.enabled = true;
            s.connected = false;
        });

        // Event-loop thread: drives the connection, (re)publishes discovery on every connect.
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ev_client = client.clone();
        let ev_status = status.clone();
        let ev_notify = notify.clone();
        let ev_stop = stop.clone();
        let ev_id = id.clone();
        let events = std::thread::Builder::new()
            .name("mqtt-events".into())
            .spawn(move || {
                for event in connection.iter() {
                    if ev_stop.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    match event {
                        Ok(Event::Incoming(Packet::ConnAck(_))) => {
                            let mut err = None;
                            for (topic, payload) in
                                discovery_messages(&ev_id, env!("CARGO_PKG_VERSION"))
                            {
                                if let Err(e) =
                                    ev_client.publish(topic, QoS::AtLeastOnce, true, payload)
                                {
                                    err = Some(e.to_string());
                                }
                            }
                            if let Err(e) = ev_client.publish(
                                ev_id.availability_topic(),
                                QoS::AtLeastOnce,
                                true,
                                "online",
                            ) {
                                err = Some(e.to_string());
                            }
                            set_status(&ev_status, &*ev_notify, |s| {
                                s.connected = true;
                                s.last_error = err;
                            });
                        }
                        Ok(_) => {}
                        Err(e) => {
                            set_status(&ev_status, &*ev_notify, |s| {
                                s.connected = false;
                                s.last_error = Some(friendly_error(&e.to_string()));
                            });
                            // rumqttc reconnects on the next iteration; don't spin.
                            std::thread::sleep(Duration::from_secs(5));
                        }
                    }
                }
            })
            .expect("failed to start the MQTT event thread");

        // Command loop for this session.
        let outcome = loop {
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok(Cmd::Sample(sample)) => {
                    if status.lock().map(|s| s.connected).unwrap_or(false) {
                        let payload = state_payload(&id, &sample);
                        match client.publish(id.state_topic(), QoS::AtMostOnce, false, payload) {
                            Ok(()) => set_status(&status, &*notify, |s| s.published += 1),
                            Err(e) => set_status(&status, &*notify, |s| {
                                s.last_error = Some(e.to_string())
                            }),
                        }
                    }
                }
                Ok(Cmd::Reconfigure(c, h)) => break Some((c, h)),
                Ok(Cmd::Quit) | Err(RecvTimeoutError::Disconnected) => break None,
                Err(RecvTimeoutError::Timeout) => {}
            }
        };

        // Tear the session down: mark offline, disconnect, stop the event thread.
        let _ = client.publish(id.availability_topic(), QoS::AtLeastOnce, true, "offline");
        std::thread::sleep(Duration::from_millis(200));
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = client.disconnect();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !events.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        set_status(&status, &*notify, |s| s.connected = false);

        match outcome {
            Some((c, h)) => {
                cfg = c;
                target_host = h;
            }
            None => return,
        }
    }
}

fn friendly_error(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("notauthorized")
        || lower.contains("badusernameorpassword")
        || lower.contains("not authorized")
    {
        "the broker rejected the username or password".into()
    } else if lower.contains("connection refused") {
        "connection refused: is the broker running on that host and port?".into()
    } else if lower.contains("timed out") || lower.contains("timeout") {
        "the broker did not answer (timed out)".into()
    } else if lower.contains("dns")
        || lower.contains("failed to lookup")
        || lower.contains("no such host")
    {
        "could not resolve the broker's address".into()
    } else {
        raw.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> MqttConfig {
        MqttConfig {
            enabled: true,
            host: "homeassistant.local".into(),
            device_name: "Phil's PC".into(),
            ..Default::default()
        }
    }

    #[test]
    fn sanitizes_identifiers() {
        assert_eq!(sanitize("Phil's PC"), "phil_s_pc");
        assert_eq!(sanitize("192.168.86.1"), "192_168_86_1");
        assert_eq!(sanitize("  --  "), "unknown");
        assert_eq!(sanitize("Router.Home.lan"), "router_home_lan");
    }

    #[test]
    fn topics_and_ids() {
        let id = Identity::new(&cfg(), "192.168.86.1");
        assert_eq!(id.availability_topic(), "pingagent/phil_s_pc/availability");
        assert_eq!(id.state_topic(), "pingagent/phil_s_pc/192_168_86_1/state");
        let msgs = discovery_messages(&id, "0.3.0");
        assert_eq!(msgs.len(), 4);
        assert_eq!(
            msgs[0].0,
            "homeassistant/sensor/pingagent_phil_s_pc_192_168_86_1/latency/config"
        );
        assert_eq!(
            msgs[3].0,
            "homeassistant/binary_sensor/pingagent_phil_s_pc_192_168_86_1/reachable/config"
        );
        for (_, payload) in &msgs {
            let v: serde_json::Value =
                serde_json::from_str(payload).expect("discovery payload is valid JSON");
            assert_eq!(v["device"]["identifiers"][0], "pingagent_phil_s_pc");
            assert_eq!(v["device"]["name"], "Phil's PC ping");
            assert_eq!(v["availability_topic"], "pingagent/phil_s_pc/availability");
            assert_eq!(v["state_topic"], "pingagent/phil_s_pc/192_168_86_1/state");
            assert!(v["unique_id"]
                .as_str()
                .unwrap()
                .starts_with("pingagent_phil_s_pc_192_168_86_1_"));
        }
        let latency: serde_json::Value = serde_json::from_str(&msgs[0].1).unwrap();
        assert_eq!(latency["unit_of_measurement"], "ms");
        assert_eq!(latency["value_template"], "{{ value_json.latency_ms }}");
        let reach: serde_json::Value = serde_json::from_str(&msgs[3].1).unwrap();
        assert_eq!(reach["device_class"], "connectivity");
    }

    #[test]
    fn state_payload_is_json() {
        let id = Identity::new(&cfg(), "192.168.86.1");
        let s = Sample {
            latency_ms: Some(12),
            reachable: true,
            result: "reply",
            loss_pct: 1.666,
            avg_ms: Some(14.26),
            mode: "every 5 s".into(),
        };
        let v: serde_json::Value = serde_json::from_str(&state_payload(&id, &s)).unwrap();
        assert_eq!(v["latency_ms"], 12);
        assert_eq!(v["reachable"], true);
        assert_eq!(v["loss_pct"], 1.7);
        assert_eq!(v["avg_ms"], 14.3);
        assert_eq!(v["host"], "192.168.86.1");
        let t = Sample {
            latency_ms: None,
            reachable: false,
            result: "timeout",
            loss_pct: 100.0,
            avg_ms: None,
            mode: "x".into(),
        };
        let v: serde_json::Value = serde_json::from_str(&state_payload(&id, &t)).unwrap();
        assert!(v["latency_ms"].is_null());
        assert!(v["avg_ms"].is_null());
        assert_eq!(v["result"], "timeout");
    }

    #[test]
    fn config_validation() {
        assert!(MqttConfig::default().validate().is_ok());
        assert!(MqttConfig {
            enabled: true,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(MqttConfig {
            enabled: true,
            host: "ha".into(),
            port: 0,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(cfg().validate().is_ok());
    }

    #[test]
    fn publisher_idles_when_disabled_and_stops_cleanly() {
        let mut p = Publisher::start(MqttConfig::default(), "h".into(), || {});
        p.publish(Sample {
            latency_ms: Some(1),
            reachable: true,
            result: "reply",
            loss_pct: 0.0,
            avg_ms: Some(1.0),
            mode: String::new(),
        });
        std::thread::sleep(Duration::from_millis(50));
        let st = p.status();
        assert!(!st.enabled && !st.connected && st.published == 0);
        p.shutdown();
    }
}
