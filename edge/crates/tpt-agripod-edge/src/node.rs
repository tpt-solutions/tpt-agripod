// SPDX-License-Identifier: MIT OR Apache-2.0

//! The edge node: the control loop, its sensors, and its link to the fleet over MQTT.
//!
//! [`Node`] is pure logic. It turns sensor readings into a control step and returns the messages
//! to publish, and it turns inbound messages into acks. It never touches the network.
//!
//! [`run`] is the transport. It connects to the broker with `minimq` over a tokio TCP stream,
//! subscribes to pushes and commands, publishes what `Node` returns, and reconnects after any
//! failure. The control loop never waits on the network, so a broken link cannot change what the
//! hardware does. The last-will marks the node offline if the process disappears.

use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use embedded_io_adapters::tokio_1::FromTokio;
use minimq::{Buffers, ConfigBuilder, Publication, QoS, Session, TopicFilter, Will};
use tokio::net::TcpStream;
use tokio::time::{MissedTickBehavior, interval, sleep};

use crate::cycle::{Actuators, ControlLoop, Inputs, Journal, Outcome};
use crate::link::{
    AckCache, Envelope, TelemetrySample, Topics, command_ack_rejected, command_msg_id, handle_push,
    status_message, telemetry_message,
};
use crate::profile::ProfileStore;

/// Where the node gets its readings. Hardware implements it; tests use fakes.
pub trait SensorSource {
    /// The readings for this cycle. Each channel is `None` if the sensor reported nothing.
    fn read(&mut self, now_s: u64) -> Inputs;
}

/// Node identity and timing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeConfig {
    /// Site id (slug).
    pub site: String,
    /// Node id (slug). Also the MQTT client id.
    pub node: String,
    /// Broker address, host:port.
    pub broker: String,
    /// MQTT keep-alive, seconds. The broker declares the node dead after about 1.5 times this.
    pub keepalive_s: u16,
    /// Control cycle period.
    pub cycle: Duration,
    /// Status heartbeat period.
    pub heartbeat: Duration,
    /// Wait before reconnecting after a failure.
    pub retry: Duration,
}

impl NodeConfig {
    /// The node config for a validated site: identity and broker from the site file.
    pub fn from_site(site: &crate::site::SiteConfig) -> Result<Self, String> {
        Self::new(&site.site_id, &site.node_id, &site.broker)
    }

    /// A config with the timing the spec implies: a 10 s cycle, a 30 s heartbeat and keep-alive,
    /// and a 2 s reconnect delay.
    pub fn new(site: &str, node: &str, broker: &str) -> Result<Self, String> {
        Topics::new(site, node)?;
        if broker.is_empty() {
            return Err("broker address is empty".into());
        }
        Ok(Self {
            site: site.into(),
            node: node.into(),
            broker: broker.into(),
            keepalive_s: 30,
            cycle: Duration::from_secs(10),
            heartbeat: Duration::from_secs(30),
            retry: Duration::from_secs(2),
        })
    }
}

/// A message the node wants published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outbound {
    /// Topic.
    pub topic: String,
    /// JSON payload.
    pub payload: String,
    /// Whether the broker retains it.
    pub retain: bool,
}

/// The node's state and logic, without the network.
pub struct Node<S: SensorSource, A: Actuators, J: Journal> {
    cfg: NodeConfig,
    topics: Topics,
    control: ControlLoop<A, J>,
    source: S,
    store: ProfileStore,
    acks: AckCache,
    seq: u64,
    started_s: u64,
    last_state: &'static str,
}

impl<S: SensorSource, A: Actuators, J: Journal> Node<S, A, J> {
    /// A node with the given control loop and sensors. No profile is active until one is pushed.
    pub fn new(cfg: NodeConfig, control: ControlLoop<A, J>, source: S) -> Result<Self, String> {
        let topics = Topics::new(&cfg.site, &cfg.node)?;
        Ok(Self {
            cfg,
            topics,
            control,
            source,
            store: ProfileStore::new(),
            acks: AckCache::new(256),
            seq: 0,
            started_s: now_s(),
            last_state: "online",
        })
    }

    /// The configuration.
    pub fn config(&self) -> &NodeConfig {
        &self.cfg
    }

    /// The topics this node uses.
    pub fn topics(&self) -> &Topics {
        &self.topics
    }

    /// The profile store.
    pub fn profiles(&self) -> &ProfileStore {
        &self.store
    }

    /// The control loop.
    pub fn control(&self) -> &ControlLoop<A, J> {
        &self.control
    }

    /// The control loop, mutably. For operator actions such as a latch reset.
    pub fn control_mut(&mut self) -> &mut ControlLoop<A, J> {
        &mut self.control
    }

    /// The status the node is in now: `safe_state` while latched or after a safe-state cycle.
    pub fn state(&self) -> &'static str {
        if self.control.is_latched() {
            "safe_state"
        } else {
            self.last_state
        }
    }

    fn envelope(&self, now: u64) -> Result<Envelope, String> {
        Envelope::new(&self.cfg.site, &self.cfg.node, now)
    }

    /// The last-will: an offline status, retained, published by the broker if the node vanishes.
    pub fn will(&self) -> Result<Outbound, String> {
        let env = self.envelope(now_s())?;
        Ok(Outbound {
            topic: self.topics.status.clone(),
            payload: status_message(&env, "offline", None, 0),
            retain: true,
        })
    }

    /// A status heartbeat, retained so a late subscriber sees the current state.
    pub fn heartbeat(&self, now: u64) -> Result<Outbound, String> {
        let env = self.envelope(now)?;
        let profile = self
            .store
            .active()
            .map(|p| (p.profile_id.as_str(), p.profile_version.as_str()));
        Ok(Outbound {
            topic: self.topics.status.clone(),
            payload: status_message(
                &env,
                self.state(),
                profile,
                now.saturating_sub(self.started_s),
            ),
            retain: true,
        })
    }

    /// Runs one control cycle and returns what to publish: good readings as telemetry, and a
    /// status change if the cycle went to safe state.
    pub fn tick(&mut self, now: u64) -> Result<Vec<Outbound>, String> {
        let inputs = self.source.read(now);
        let outcome = self.control.step(&inputs);
        self.last_state = match outcome {
            Outcome::Applied(_) => "online",
            Outcome::Safe { .. } | Outcome::Latched { .. } => "safe_state",
        };

        let mut out = Vec::new();
        let mut samples = Vec::new();
        if let Some(s) = inputs.moisture {
            samples.push(TelemetrySample {
                channel: "moisture_frac",
                unit: "frac",
                at_s: s.at_s,
                value: s.value,
            });
        }
        if let Some(s) = inputs.reservoir_l {
            samples.push(TelemetrySample {
                channel: "reservoir_level_l",
                unit: "l",
                at_s: s.at_s,
                value: s.value,
            });
        }
        if let Some(s) = inputs.headroom_w {
            samples.push(TelemetrySample {
                channel: "grid_power_headroom_w",
                unit: "w",
                at_s: s.at_s,
                value: s.value,
            });
        }
        if !samples.is_empty() {
            self.seq += 1;
            let env = self.envelope(now)?;
            out.push(Outbound {
                topic: self.topics.telemetry.clone(),
                payload: telemetry_message(&env, self.seq, &samples),
                retain: false,
            });
        }
        if self.last_state == "safe_state" {
            out.push(self.heartbeat(now)?);
        }
        Ok(out)
    }

    /// Handles one inbound message. Pushes are validated, applied or rejected, and acked. Commands
    /// are refused with an ack, because command execution is not implemented on the node yet.
    /// Anything else is ignored.
    pub fn on_message(&mut self, topic: &str, payload: &str, now: u64) -> Vec<Outbound> {
        if topic == self.topics.profile_push {
            return match handle_push(
                &self.cfg.site,
                &self.cfg.node,
                payload,
                &mut self.store,
                &mut self.acks,
                now,
            ) {
                Ok(outcome) => {
                    if let Some(active) = &outcome.newly_applied {
                        self.control.apply_profile(active);
                    }
                    vec![Outbound {
                        topic: self.topics.profile_ack.clone(),
                        payload: outcome.ack,
                        retain: false,
                    }]
                }
                Err(_) => Vec::new(), // nothing to answer: see handle_push
            };
        }
        if topic == self.topics.command {
            let Some(ack_of) = command_msg_id(payload) else {
                return Vec::new();
            };
            let Ok(env) = self.envelope(now) else {
                return Vec::new();
            };
            return vec![Outbound {
                topic: self.topics.command_ack.clone(),
                payload: command_ack_rejected(
                    &env,
                    &ack_of,
                    "command execution is not implemented on this node",
                ),
                retain: false,
            }];
        }
        Vec::new()
    }
}

/// Unix time in whole seconds.
pub fn now_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Why one connection ended.
#[derive(Debug, PartialEq, Eq)]
enum Ended {
    /// Shutdown was requested. The caller stops.
    Shutdown,
    /// The broker dropped the connection. The caller reconnects.
    Disconnected,
}

/// Runs the node against the broker until `shutdown` completes. Reconnects after failures.
pub async fn run<S, A, J, F>(node: &mut Node<S, A, J>, shutdown: F) -> Result<(), String>
where
    S: SensorSource,
    A: Actuators,
    J: Journal,
    F: Future<Output = ()>,
{
    let mut shutdown = std::pin::pin!(shutdown);
    let mut rx = vec![0u8; 4096];
    let mut tx = vec![0u8; 8192];
    loop {
        match session(node, &mut rx, &mut tx, shutdown.as_mut()).await {
            Ok(Ended::Shutdown) => return Ok(()),
            Ok(Ended::Disconnected) => eprintln!("mqtt: disconnected; reconnecting"),
            Err(e) => eprintln!("mqtt: {e}; retrying"),
        }
        tokio::select! {
            _ = shutdown.as_mut() => return Ok(()),
            _ = sleep(node.cfg.retry) => {}
        }
    }
}

async fn session<S, A, J>(
    node: &mut Node<S, A, J>,
    rx: &mut [u8],
    tx: &mut [u8],
    mut shutdown: Pin<&mut impl Future<Output = ()>>,
) -> Result<Ended, String>
where
    S: SensorSource,
    A: Actuators,
    J: Journal,
{
    // Copy what the session borrows, so the session does not hold a borrow of the node.
    let client_id = node.cfg.node.clone();
    let keepalive = node.cfg.keepalive_s;
    let status_topic = node.topics.status.clone();
    let push_topic = node.topics.profile_push.clone();
    let command_topic = node.topics.command.clone();
    let will_out = node.will()?;
    let online = node.heartbeat(now_s())?;

    let tcp = TcpStream::connect(&node.cfg.broker)
        .await
        .map_err(|e| format!("connect {}: {e}", node.cfg.broker))?;
    let will = Will::new(&status_topic, will_out.payload.as_bytes(), &[])
        .map_err(|e| format!("will: {e:?}"))?
        .retained()
        .qos(QoS::AtLeastOnce);
    let config = ConfigBuilder::new(Buffers::new(rx, tx))
        .client_id(&client_id)
        .map_err(|e| format!("client id: {e:?}"))?
        .keepalive_interval(keepalive)
        .will(will)
        .map_err(|e| format!("will: {e:?}"))?;
    let mut session = Session::new(config);
    let mut conn = session
        .connect(FromTokio::new(tcp))
        .await
        .map_err(|e| format!("mqtt connect: {e:?}"))?;

    conn.subscribe(
        &[
            TopicFilter::new(&push_topic),
            TopicFilter::new(&command_topic),
        ],
        &[],
    )
    .await
    .map_err(|e| format!("subscribe: {e:?}"))?;
    publish_all(&mut conn, vec![online]).await?;

    let mut cycle = interval(node.cfg.cycle);
    cycle.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut beat = interval(node.cfg.heartbeat);
    beat.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = shutdown.as_mut() => {
                let _ = conn.disconnect().await;
                return Ok(Ended::Shutdown);
            }
            received = async {
                match conn.recv().await {
                    Ok(m) => Ok(Some((m.topic().to_string(), String::from_utf8_lossy(m.payload()).into_owned()))),
                    Err(minimq::Error::Disconnected) => Ok(None),
                    Err(e) => Err(format!("receive: {e:?}")),
                }
            } => {
                match received? {
                    Some((topic, payload)) => {
                        let out = node.on_message(&topic, &payload, now_s());
                        publish_all(&mut conn, out).await?;
                    }
                    None => return Ok(Ended::Disconnected),
                }
            }
            _ = cycle.tick() => {
                let out = node.tick(now_s())?;
                publish_all(&mut conn, out).await?;
            }
            _ = beat.tick() => {
                let out = vec![node.heartbeat(now_s())?];
                publish_all(&mut conn, out).await?;
            }
        }
    }
}

async fn publish_all<IO>(
    conn: &mut minimq::Connection<'_, '_, IO>,
    messages: Vec<Outbound>,
) -> Result<(), String>
where
    IO: minimq::Io,
{
    for m in messages {
        let publication = Publication::text(&m.topic, &m.payload).qos(QoS::AtLeastOnce);
        let publication = if m.retain {
            publication.retain()
        } else {
            publication
        };
        conn.publish(publication)
            .await
            .map_err(|e| format!("publish {}: {e:?}", m.topic))?;
    }
    Ok(())
}
