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

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use embedded_io_adapters::tokio_1::FromTokio;
use minimq::{
    Buffers, ConfigBuilder, Connection, PubError, Publication, QoS, Session, TopicFilter, Will,
};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::time::{MissedTickBehavior, interval, sleep, timeout};

use crate::cycle::{Actuators, ControlLoop, Inputs, Journal, Outcome};
use crate::link::{
    AckCache, Envelope, TelemetrySample, Topics, command_ack_rejected, command_msg_id, handle_push,
    status_message, telemetry_message,
};
use crate::outbox::{Outbox, Record};
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
    outbox: Outbox,
    live: VecDeque<Outbound>,
}

impl<S: SensorSource, A: Actuators, J: Journal> Node<S, A, J> {
    /// A node with the given control loop and sensors. No profile is active until one is pushed.
    pub fn new(
        cfg: NodeConfig,
        control: ControlLoop<A, J>,
        source: S,
        outbox: Outbox,
    ) -> Result<Self, String> {
        let topics = Topics::new(&cfg.site, &cfg.node)?;
        Ok(Self {
            cfg,
            topics,
            control,
            source,
            store: ProfileStore::new(),
            acks: AckCache::new(256),
            seq: outbox.high_water(),
            started_s: now_s(),
            last_state: "online",
            outbox,
            live: VecDeque::new(),
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

    /// The telemetry outbox, read-only.
    pub fn outbox(&self) -> &Outbox {
        &self.outbox
    }

    /// Up to `max` undelivered telemetry messages, oldest first.
    pub fn pending_telemetry(&self, max: usize) -> Vec<Record> {
        self.outbox.pending().iter().take(max).cloned().collect()
    }

    /// Marks a telemetry message as delivered, removing it from the durable outbox.
    pub fn acknowledge(&mut self, seq: u64) -> Result<(), String> {
        self.outbox.ack(seq).map_err(|e| format!("outbox: {e}"))
    }

    /// Runs one control cycle and returns what to publish: good readings as telemetry, and a
    /// status change if the cycle went to safe state.
    pub fn tick(&mut self, now: u64) -> Result<(), String> {
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
            let payload = telemetry_message(&env, self.seq, &samples);
            // Durable before it is sent: the outbox keeps it until the broker has taken it.
            self.outbox
                .push(self.seq, &payload)
                .map_err(|e| format!("outbox: {e}"))?;
        }
        if self.last_state == "safe_state" {
            out.push(self.heartbeat(now)?);
        }
        self.live.extend(out);
        Ok(())
    }

    /// Handles one inbound message and queues the replies for the link to send.
    pub fn handle_inbound(&mut self, topic: &str, payload: &str, now: u64) -> Result<(), String> {
        let replies = self.on_message(topic, payload, now);
        self.live.extend(replies);
        Ok(())
    }

    /// Takes the queued live messages (status, acks) for the link to send. Live messages are not
    /// durable: they describe the present, so a message lost while disconnected is not replayed.
    pub fn take_live(&mut self) -> Vec<Outbound> {
        self.live.drain(..).collect()
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
    /// The connection dropped. The link reconnects.
    Disconnected,
}

/// How often the link flushes queued messages and checks for new ones.
const FLUSH_EVERY: Duration = Duration::from_secs(1);
/// Telemetry sent per flush. Enough to catch up after an outage without starving other work.
const DRAIN_BATCH: usize = 200;
/// Longest a TCP connect may take before the link gives up and retries.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest one wait for session progress while the send window is full.
const PROGRESS_WAIT: Duration = Duration::from_millis(100);
/// How many times a publish is retried while the send window stays full. At 100 ms each, 600
/// attempts is one minute: a broker that never acknowledges is an error, not a stall.
const SEND_ATTEMPTS: u32 = 600;

/// Runs the node until `shutdown` completes, and returns it.
///
/// Two tasks run for the life of the node, and neither waits on the other:
///
/// - The control task runs a cycle on schedule whether or not the broker is reachable. The
///   hardware never waits on the network, and telemetry is kept in the durable outbox.
/// - The link task keeps a connection to the broker, reconnecting after failures. When connected,
///   it sends live messages and the outbox, and it handles pushes and commands.
pub async fn run<S, A, J, F>(node: Node<S, A, J>, shutdown: F) -> Result<Node<S, A, J>, String>
where
    S: SensorSource,
    A: Actuators,
    J: Journal,
    F: Future<Output = ()>,
{
    let node = Mutex::new(node);
    let cycle = node.lock().await.cfg.cycle;
    let mut shutdown = std::pin::pin!(shutdown);
    tokio::select! {
        r = control_task(&node, cycle) => { r?; }
        r = link_task(&node, shutdown.as_mut()) => { r?; }
    }
    Ok(node.into_inner())
}

async fn control_task<S, A, J>(node: &Mutex<Node<S, A, J>>, cycle: Duration) -> Result<(), String>
where
    S: SensorSource,
    A: Actuators,
    J: Journal,
{
    let mut tick = interval(cycle);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        node.lock().await.tick(now_s())?;
    }
}

async fn link_task<S, A, J>(
    node: &Mutex<Node<S, A, J>>,
    mut shutdown: Pin<&mut impl Future<Output = ()>>,
) -> Result<(), String>
where
    S: SensorSource,
    A: Actuators,
    J: Journal,
{
    loop {
        let retry = node.lock().await.cfg.retry;
        match session(node, shutdown.as_mut()).await {
            Ok(Ended::Shutdown) => return Ok(()),
            Ok(Ended::Disconnected) => eprintln!("mqtt: disconnected; reconnecting"),
            Err(e) => eprintln!("mqtt: {e}; retrying"),
        }
        tokio::select! {
            _ = shutdown.as_mut() => return Ok(()),
            _ = sleep(retry) => {}
        }
    }
}

async fn session<S, A, J>(
    node: &Mutex<Node<S, A, J>>,
    mut shutdown: Pin<&mut impl Future<Output = ()>>,
) -> Result<Ended, String>
where
    S: SensorSource,
    A: Actuators,
    J: Journal,
{
    // Copy what the session borrows, so the session does not hold a borrow of the node.
    let (
        client_id,
        keepalive,
        broker,
        heartbeat_every,
        status_topic,
        push_topic,
        command_topic,
        will_out,
        online,
    ) = {
        let n = node.lock().await;
        (
            n.cfg.node.clone(),
            n.cfg.keepalive_s,
            n.cfg.broker.clone(),
            n.cfg.heartbeat,
            n.topics.status.clone(),
            n.topics.profile_push.clone(),
            n.topics.command.clone(),
            n.will()?,
            n.heartbeat(now_s())?,
        )
    };

    let tcp = timeout(CONNECT_TIMEOUT, TcpStream::connect(&broker))
        .await
        .map_err(|_| format!("connect {broker}: timed out"))?
        .map_err(|e| format!("connect {broker}: {e}"))?;
    let (mut rx, mut tx) = (vec![0u8; 4096], vec![0u8; 8192]);
    let will = Will::new(&status_topic, will_out.payload.as_bytes(), &[])
        .map_err(|e| format!("will: {e:?}"))?
        .retained()
        .qos(QoS::AtLeastOnce);
    let config = ConfigBuilder::new(Buffers::new(&mut rx, &mut tx))
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
    publish_all(node, &mut conn, vec![online]).await?;
    flush(node, &mut conn).await?;

    let mut flush_tick = interval(FLUSH_EVERY);
    flush_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut beat = interval(heartbeat_every);
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
                        node.lock().await.handle_inbound(&topic, &payload, now_s())?;
                    }
                    None => return Ok(Ended::Disconnected),
                }
            }
            _ = flush_tick.tick() => {
                flush(node, &mut conn).await?;
            }
            _ = beat.tick() => {
                let hb = node.lock().await.heartbeat(now_s())?;
                publish_all(node, &mut conn, vec![hb]).await?;
            }
        }
    }
}

/// Sends queued live messages, then the outbox in order. Each outbox message is acknowledged only
/// after its publish call has succeeded, so a failure leaves it for the next connection.
async fn flush<S, A, J, IO>(
    node: &Mutex<Node<S, A, J>>,
    conn: &mut Connection<'_, '_, IO>,
) -> Result<(), String>
where
    S: SensorSource,
    A: Actuators,
    J: Journal,
    IO: minimq::Io,
{
    let live = node.lock().await.take_live();
    publish_all(node, conn, live).await?;
    let (topic, batch) = {
        let n = node.lock().await;
        (n.topics.telemetry.clone(), n.pending_telemetry(DRAIN_BATCH))
    };
    for Record { seq, payload } in batch {
        send(node, conn, &topic, &payload, false).await?;
        node.lock().await.acknowledge(seq)?;
    }
    Ok(())
}

/// Publishes every message in order at QoS 1.
async fn publish_all<S, A, J, IO>(
    node: &Mutex<Node<S, A, J>>,
    conn: &mut Connection<'_, '_, IO>,
    messages: Vec<Outbound>,
) -> Result<(), String>
where
    S: SensorSource,
    A: Actuators,
    J: Journal,
    IO: minimq::Io,
{
    for m in messages {
        send(node, conn, &m.topic, &m.payload, m.retain).await?;
    }
    Ok(())
}

/// Publishes one message at QoS 1. When the session's send window is full, publish refuses with
/// `NotReady`. The session then has to make progress, which processes acknowledgements, so this
/// polls the session and tries again. Any inbound message seen meanwhile goes to the node.
async fn send<S, A, J, IO>(
    node: &Mutex<Node<S, A, J>>,
    conn: &mut Connection<'_, '_, IO>,
    topic: &str,
    payload: &str,
    retain: bool,
) -> Result<(), String>
where
    S: SensorSource,
    A: Actuators,
    J: Journal,
    IO: minimq::Io,
{
    for _ in 0..SEND_ATTEMPTS {
        let publication = Publication::text(topic, payload).qos(QoS::AtLeastOnce);
        let publication = if retain {
            publication.retain()
        } else {
            publication
        };
        match conn.publish(publication).await {
            Ok(_) => return Ok(()),
            Err(PubError::Session(
                minimq::Error::NotReady
                | minimq::Error::Resource(minimq::ResourceError::InflightExhausted),
            )) => {
                if let Some((t, p)) = progress(conn).await? {
                    node.lock().await.handle_inbound(&t, &p, now_s())?;
                }
            }
            Err(e) => return Err(format!("publish {topic}: {e:?}")),
        }
    }
    Err(format!(
        "publish {topic}: send window stayed full for a minute; the broker is not acknowledging"
    ))
}

/// Lets the session make progress. Returns an inbound publish if that progress produced one.
async fn progress<IO>(conn: &mut Connection<'_, '_, IO>) -> Result<Option<(String, String)>, String>
where
    IO: minimq::Io,
{
    // Bounded: a session with no progress must still yield, so timers and the control loop run.
    let waited = timeout(PROGRESS_WAIT, conn.poll()).await;
    let Ok(result) = waited else {
        return Ok(None);
    };
    match result {
        Ok(Some(m)) => Ok(Some((
            m.topic().to_string(),
            String::from_utf8_lossy(m.payload()).into_owned(),
        ))),
        Ok(None) => Ok(None),
        Err(minimq::Error::Disconnected) => {
            Err("session disconnected while waiting for acknowledgements".into())
        }
        Err(e) => Err(format!("session progress: {e:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cycle::{Actuators, Command, ControlLoop, Event, Journal};
    use crate::estimate::Sample;
    use crate::json::Value;

    struct Fixed;

    impl SensorSource for Fixed {
        fn read(&mut self, now_s: u64) -> Inputs {
            Inputs {
                now_s,
                moisture: Some(Sample {
                    value: 0.6,
                    at_s: now_s,
                }),
                reservoir_l: Some(Sample {
                    value: 80.0,
                    at_s: now_s,
                }),
                headroom_w: Some(Sample {
                    value: 2500.0,
                    at_s: now_s,
                }),
            }
        }
    }

    struct Hw;

    impl Actuators for Hw {
        fn apply(&mut self, _c: &Command) -> Result<(), String> {
            Ok(())
        }

        fn isolate(&mut self) -> Result<(), String> {
            Ok(())
        }
    }

    struct Quiet;

    impl Journal for Quiet {
        fn record(&mut self, _e: Event) {}
    }

    fn site() -> crate::site::SiteConfig {
        crate::site::SiteConfig::parse(include_str!("../tests/fixtures/site.test.json")).unwrap()
    }

    fn outbox_path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("agripod-node-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("outbox.log");
        let _ = std::fs::remove_file(&path);
        path
    }

    fn node(path: &std::path::Path) -> Node<Fixed, Hw, Quiet> {
        let cfg = NodeConfig::new("wgtn-bay-01", "nuc-01", "127.0.0.1:1").unwrap();
        let control = ControlLoop::new(&site(), Hw, Quiet).unwrap();
        Node::new(cfg, control, Fixed, Outbox::open(path, 1_000_000).unwrap()).unwrap()
    }

    #[test]
    fn telemetry_waits_in_the_outbox_and_survives_a_restart_until_acknowledged() {
        let path = outbox_path("durable");
        {
            // No broker is involved: this is the state of a node that cannot reach one.
            let mut n = node(&path);
            for t in 0..3 {
                n.tick(1_767_225_600 + t * 10).unwrap();
            }
            let seqs: Vec<u64> = n.pending_telemetry(10).iter().map(|r| r.seq).collect();
            assert_eq!(seqs, vec![1, 2, 3]);
            n.acknowledge(2).unwrap();
        }
        // Restart: the unacknowledged messages are still there, in order.
        let mut n = node(&path);
        let pending = n.pending_telemetry(10);
        assert_eq!(
            pending.iter().map(|r| r.seq).collect::<Vec<_>>(),
            vec![1, 3]
        );
        let v = crate::json::parse(&pending[0].payload).unwrap();
        assert_eq!(v.get("node_id").and_then(Value::as_str), Some("nuc-01"));
        assert_eq!(
            v.get("samples").and_then(Value::as_array).map(|s| s.len()),
            Some(3)
        );
        // The sequence continues after a restart: the outbox does not reset it, so no false gap is
        // created by a fresh process.
        n.tick(1_767_225_700).unwrap();
        assert_eq!(n.pending_telemetry(10).last().map(|r| r.seq), Some(4));
    }

    #[test]
    fn a_full_outbox_drops_the_oldest_and_the_gap_is_visible_in_seq() {
        let path = outbox_path("full");
        let cfg = NodeConfig::new("wgtn-bay-01", "nuc-01", "127.0.0.1:1").unwrap();
        let control = ControlLoop::new(&site(), Hw, Quiet).unwrap();
        // Room for about two telemetry messages (each is a few hundred bytes).
        let one = {
            let mut n = node(&path);
            n.tick(1_767_225_600).unwrap();
            n.pending_telemetry(1)[0].payload.len() as u64
        };
        let _ = std::fs::remove_file(&path);
        let mut n = Node::new(
            cfg,
            control,
            Fixed,
            Outbox::open(&path, one * 2 + 8).unwrap(),
        )
        .unwrap();
        for t in 0..5 {
            n.tick(1_767_225_600 + t * 10).unwrap();
        }
        let seqs: Vec<u64> = n.pending_telemetry(10).iter().map(|r| r.seq).collect();
        assert_eq!(seqs, vec![4, 5]);
        assert_eq!(n.outbox().dropped(), 3);
    }

    #[tokio::test]
    async fn measurement_continues_while_the_broker_is_unreachable() {
        // Nothing listens on port 1, so every connect fails. The control loop must not care.
        let path = outbox_path("offline");
        let mut cfg = NodeConfig::new("wgtn-bay-01", "nuc-01", "127.0.0.1:1").unwrap();
        cfg.cycle = Duration::from_millis(50);
        cfg.retry = Duration::from_millis(50);
        let control = ControlLoop::new(&site(), Hw, Quiet).unwrap();
        let outbox = Outbox::open(&path, 1_000_000).unwrap();
        let n = Node::new(cfg, control, Fixed, outbox).unwrap();

        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let handle = tokio::spawn(run(n, async {
            let _ = stopped.await;
        }));
        tokio::time::sleep(Duration::from_millis(600)).await;
        let _ = stop.send(());
        let node = handle.await.unwrap().expect("the node stops cleanly");

        let seqs: Vec<u64> = node
            .pending_telemetry(10_000)
            .iter()
            .map(|r| r.seq)
            .collect();
        assert!(
            seqs.len() >= 4,
            "only {} telemetry messages while offline",
            seqs.len()
        );
        assert!(
            seqs.windows(2).all(|w| w[1] == w[0] + 1),
            "no gaps while offline: {seqs:?}"
        );
    }
}
