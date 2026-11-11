// SPDX-License-Identifier: MIT OR Apache-2.0

//! The edge node against a real MQTT broker. The orchestrator side is played by a second minimq
//! session in the test, so the protocol is exercised from both ends.
//!
//! Needs a broker: `go run ./fleet/testbroker -addr 127.0.0.1:1883`, then
//! `AGRIPOD_TEST_BROKER=127.0.0.1:1883 cargo test -p tpt-agripod-edge --test node_link`.
//! Without the variable the tests report that they were skipped.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use embedded_io_adapters::tokio_1::FromTokio;
use minimq::{Buffers, ConfigBuilder, Connection, Publication, QoS, Session, TopicFilter};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::time::timeout;

use tpt_agripod_edge::json::{self, Value};
use tpt_agripod_edge::link::{Topics, uuid_v4};
mod common;

use tpt_agripod_edge::{
    Actuators, Command, ControlLoop, Event, Inputs, Journal, Node, NodeConfig, Sample, SensorSource,
};

const SITE: &str = "wgtn-bay-01";
const EXAMPLE: &str = include_str!("../../../../profiles/examples/example-plant-v0.json");
const WAIT: Duration = Duration::from_secs(10);

type Peer<'s, 'b> = Connection<'s, 'b, FromTokio<TcpStream>>;

/// A node id no earlier run has used. Retained status from an earlier run would otherwise look
/// like readiness before this run's node has subscribed.
fn unique(prefix: &str) -> String {
    let id = uuid_v4().expect("random id");
    format!("{prefix}-{}", &id[..8])
}

fn broker() -> Option<String> {
    let addr = std::env::var("AGRIPOD_TEST_BROKER").ok();
    if addr.is_none() {
        eprintln!("skipped: set AGRIPOD_TEST_BROKER to run the broker tests");
    }
    addr
}

/// Sensors whose readings can be switched off, to test the safe-state path.
#[derive(Clone)]
struct Sensors {
    healthy: Arc<Mutex<bool>>,
}

impl SensorSource for Sensors {
    fn read(&mut self, now_s: u64) -> Inputs {
        if !*self.healthy.lock().unwrap() {
            return Inputs {
                now_s,
                moisture: None,
                reservoir_l: None,
                headroom_w: None,
            };
        }
        Inputs {
            now_s,
            moisture: Some(Sample {
                value: 0.2,
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

#[derive(Default)]
struct Hw;

impl Actuators for Hw {
    fn apply(&mut self, _cmd: &Command) -> Result<(), String> {
        Ok(())
    }

    fn isolate(&mut self) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Default)]
struct Log;

impl Journal for Log {
    fn record(&mut self, _e: Event) {}
}

type NodeT = Node<Sensors, Hw, Log>;

fn new_node(addr: &str, node: &str, healthy: Arc<Mutex<bool>>) -> NodeT {
    let mut cfg = NodeConfig::new(SITE, node, addr).unwrap();
    cfg.cycle = Duration::from_millis(100);
    cfg.heartbeat = Duration::from_secs(30);
    let control = ControlLoop::new(&common::test_site(), Hw, Log).unwrap();
    let path = std::env::temp_dir().join(format!("agripod-it-{node}-{}", uuid_v4().unwrap()));
    std::fs::create_dir_all(&path).unwrap();
    let outbox =
        tpt_agripod_edge::outbox::Outbox::open(&path.join("outbox.log"), 1_000_000).unwrap();
    Node::new(cfg, control, Sensors { healthy }, outbox).unwrap()
}

/// A running node and the handle to stop it.
struct Running {
    handle: tokio::task::JoinHandle<Result<(), String>>,
    stop: Option<oneshot::Sender<()>>,
}

fn spawn_node(node: NodeT) -> Running {
    let (tx, rx) = oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        tpt_agripod_edge::node::run(node, async {
            let _ = rx.await;
        })
        .await
        .map(|_| ())
    });
    Running {
        handle,
        stop: Some(tx),
    }
}

impl Running {
    async fn stop(mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        let result = timeout(WAIT, self.handle)
            .await
            .expect("node did not stop")
            .unwrap();
        assert!(result.is_ok(), "{result:?}");
    }
}

/// A session for the test's own client. The caller owns the buffers and the session.
fn new_session<'a>(rx: &'a mut [u8], tx: &'a mut [u8], client_id: &str) -> Session<'a> {
    Session::new(
        ConfigBuilder::new(Buffers::new(rx, tx))
            .client_id(client_id)
            .unwrap()
            .keepalive_interval(30),
    )
}

/// Connects the test client and subscribes it to `filters`.
async fn connect_to<'s, 'b>(
    session: &'s mut Session<'b>,
    addr: &str,
    filters: &[&str],
) -> Peer<'s, 'b> {
    let tcp = timeout(WAIT, TcpStream::connect(addr))
        .await
        .expect("tcp connect timed out")
        .expect("tcp connect");
    let mut conn = session
        .connect(FromTokio::new(tcp))
        .await
        .expect("peer connect");
    let topics: Vec<TopicFilter> = filters.iter().map(|f| TopicFilter::new(f)).collect();
    timeout(WAIT, conn.subscribe(&topics, &[]))
        .await
        .expect("peer subscribe timed out")
        .expect("peer subscribe");
    conn
}

/// Next message on `topic`, with its payload as text.
async fn next_on(peer: &mut Peer<'_, '_>, topic: &str) -> String {
    loop {
        let got = timeout(WAIT, peer.recv())
            .await
            .expect("timed out waiting for a message")
            .expect("receive");
        if got.topic() == topic {
            return String::from_utf8_lossy(got.payload()).into_owned();
        }
    }
}

/// Waits for a status on `topic` whose `state` field is `state`.
async fn next_state(peer: &mut Peer<'_, '_>, topic: &str, state: &str) -> Value {
    loop {
        let text = next_on(peer, topic).await;
        let v = json::parse(&text).expect("status is JSON");
        if v.get("state").and_then(Value::as_str) == Some(state) {
            return v;
        }
    }
}

async fn publish(peer: &mut Peer<'_, '_>, topic: &str, payload: &str) {
    timeout(
        WAIT,
        peer.publish(Publication::text(topic, payload).qos(QoS::AtLeastOnce)),
    )
    .await
    .expect("publish timed out")
    .expect("publish");
}

fn push_payload(msg_id: &str, node: &str, profile: &str) -> String {
    format!(
        "{{\"msg_id\":\"{msg_id}\",\"ts\":\"2026-11-11T02:15:00Z\",\"site_id\":\"{SITE}\",\"node_id\":\"{node}\",\"profile\":{profile}}}"
    )
}

fn ack_of(text: &str) -> (String, String) {
    let v = json::parse(text).expect("ack is JSON");
    (
        v.get("ack_of")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        v.get("status")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    )
}

#[tokio::test]
async fn a_pushed_profile_is_applied_acked_and_not_applied_twice() {
    let Some(addr) = broker() else { return };
    let node_id = unique("nuc-push-a");
    let node_id = node_id.as_str();
    let topics = Topics::new(SITE, node_id).unwrap();
    let running = spawn_node(new_node(&addr, node_id, Arc::new(Mutex::new(true))));

    let (mut rx, mut tx) = (vec![0u8; 4096], vec![0u8; 8192]);
    let mut session = new_session(&mut rx, &mut tx, "orch-push-a");
    let mut peer = connect_to(&mut session, &addr, &[&topics.status, &topics.profile_ack]).await;
    // The node publishes a retained online status once it has subscribed to its inputs.
    next_state(&mut peer, &topics.status, "online").await;

    let msg_id = uuid_v4().unwrap();
    let payload = push_payload(&msg_id, node_id, EXAMPLE);
    publish(&mut peer, &topics.profile_push, &payload).await;
    let first = next_on(&mut peer, &topics.profile_ack).await;
    assert_eq!(ack_of(&first), (msg_id.clone(), "applied".into()));

    // The orchestrator retries the same push. The node must answer identically, not re-apply.
    publish(&mut peer, &topics.profile_push, &payload).await;
    let second = next_on(&mut peer, &topics.profile_ack).await;
    assert_eq!(first, second);

    running.stop().await;
}

#[tokio::test]
async fn an_unsafe_profile_is_rejected_with_reasons() {
    let Some(addr) = broker() else { return };
    let node_id = unique("nuc-push-b");
    let node_id = node_id.as_str();
    let topics = Topics::new(SITE, node_id).unwrap();
    let running = spawn_node(new_node(&addr, node_id, Arc::new(Mutex::new(true))));

    let (mut rx, mut tx) = (vec![0u8; 4096], vec![0u8; 8192]);
    let mut session = new_session(&mut rx, &mut tx, "orch-push-b");
    let mut peer = connect_to(&mut session, &addr, &[&topics.status, &topics.profile_ack]).await;
    next_state(&mut peer, &topics.status, "online").await;

    let bad = EXAMPLE.replacen(r#""max": 0.15,"#, r#""max": 0.9,"#, 1);
    let msg_id = uuid_v4().unwrap();
    publish(
        &mut peer,
        &topics.profile_push,
        &push_payload(&msg_id, node_id, &bad),
    )
    .await;
    let ack = next_on(&mut peer, &topics.profile_ack).await;
    assert_eq!(ack_of(&ack), (msg_id, "rejected".into()));
    let v = json::parse(&ack).unwrap();
    let errors = v
        .get("errors")
        .and_then(Value::as_array)
        .expect("rejection carries errors");
    assert!(
        errors
            .iter()
            .any(|e| e.get("path").and_then(Value::as_str) == Some("/controls/pump_duty/max")),
        "{ack}"
    );

    running.stop().await;
}

#[tokio::test]
async fn telemetry_is_published_each_cycle_with_increasing_seq() {
    let Some(addr) = broker() else { return };
    let node_id = unique("nuc-telem");
    let node_id = node_id.as_str();
    let topics = Topics::new(SITE, node_id).unwrap();
    let running = spawn_node(new_node(&addr, node_id, Arc::new(Mutex::new(true))));

    let (mut rx, mut tx) = (vec![0u8; 4096], vec![0u8; 8192]);
    let mut session = new_session(&mut rx, &mut tx, "orch-telem");
    let mut peer = connect_to(&mut session, &addr, &[&topics.telemetry]).await;
    let mut seqs = Vec::new();
    while seqs.len() < 3 {
        let text = next_on(&mut peer, &topics.telemetry).await;
        let v = json::parse(&text).unwrap();
        seqs.push(v.get("seq").and_then(Value::as_f64).unwrap() as u64);
        let samples = v.get("samples").and_then(Value::as_array).unwrap();
        assert_eq!(samples.len(), 3, "moisture, reservoir and headroom");
    }
    assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1), "{seqs:?}");

    running.stop().await;
}

#[tokio::test]
async fn a_lost_sensor_puts_the_node_in_safe_state_and_says_so() {
    let Some(addr) = broker() else { return };
    let node_id = unique("nuc-safe");
    let node_id = node_id.as_str();
    let topics = Topics::new(SITE, node_id).unwrap();
    let healthy = Arc::new(Mutex::new(true));
    let running = spawn_node(new_node(&addr, node_id, healthy.clone()));

    let (mut rx, mut tx) = (vec![0u8; 4096], vec![0u8; 8192]);
    let mut session = new_session(&mut rx, &mut tx, "orch-safe");
    let mut peer = connect_to(&mut session, &addr, &[&topics.status]).await;
    next_state(&mut peer, &topics.status, "online").await;

    *healthy.lock().unwrap() = false;
    let v = next_state(&mut peer, &topics.status, "safe_state").await;
    assert_eq!(v.get("node_id").and_then(Value::as_str), Some(node_id));

    running.stop().await;
}

#[tokio::test]
async fn a_node_that_vanishes_is_reported_offline_by_its_last_will() {
    let Some(addr) = broker() else { return };
    let node_id = unique("nuc-lwt");
    let node_id = node_id.as_str();
    let topics = Topics::new(SITE, node_id).unwrap();
    let running = spawn_node(new_node(&addr, node_id, Arc::new(Mutex::new(true))));

    let (mut rx, mut tx) = (vec![0u8; 4096], vec![0u8; 8192]);
    let mut session = new_session(&mut rx, &mut tx, "orch-lwt");
    let mut peer = connect_to(&mut session, &addr, &[&topics.status]).await;
    next_state(&mut peer, &topics.status, "online").await;

    // Abort the task. Its socket closes without a DISCONNECT, so the broker publishes the will.
    running.handle.abort();
    next_state(&mut peer, &topics.status, "offline").await;
}
