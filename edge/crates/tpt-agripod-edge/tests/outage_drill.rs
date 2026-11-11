// SPDX-License-Identifier: MIT OR Apache-2.0

//! Outage drill: the broker is stopped while the node runs, and started again. Telemetry measured
//! during the outage must arrive after the restart, in order, with no gap in `seq`.
//!
//! Needs the test broker built from `fleet/testbroker`:
//!
//! `AGRIPOD_TESTBROKER_BIN=path/to/testbroker cargo test -p tpt-agripod-edge --test outage_drill`
//!
//! Without the variable the drill reports that it was skipped.

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command as Process};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use embedded_io_adapters::tokio_1::FromTokio;
use minimq::{Buffers, ConfigBuilder, Connection, Session, TopicFilter};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::time::timeout;

use tpt_agripod_edge::json::{self, Value};
use tpt_agripod_edge::link::Topics;
use tpt_agripod_edge::outbox::Outbox;
use tpt_agripod_edge::{
    Actuators, Command, ControlLoop, Event, Inputs, Journal, Node, NodeConfig, Sample,
    SensorSource, SiteConfig,
};

const SITE: &str = "wgtn-bay-01";
const SITE_JSON: &str = include_str!("fixtures/site.test.json");

#[derive(Clone)]
struct Sensors;

impl SensorSource for Sensors {
    fn read(&mut self, now_s: u64) -> Inputs {
        Inputs {
            now_s,
            moisture: Some(Sample {
                value: 0.5,
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

fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    l.local_addr().expect("addr").port()
}

fn start_broker(bin: &PathBuf, port: u16) -> Child {
    let child = Process::new(bin)
        .arg("-addr")
        .arg(format!("127.0.0.1:{port}"))
        .spawn()
        .expect("start the test broker");
    // Wait until it accepts connections.
    let deadline = Instant::now() + Duration::from_secs(10);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(
            Instant::now() < deadline,
            "broker did not start on port {port}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    child
}

fn outbox_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("agripod-drill-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let _ = std::fs::remove_file(dir.join("outbox.log"));
    dir
}

/// The outbox as it is on disk: the records waiting, and the sequence high-water mark.
fn read_outbox_file(path: &PathBuf) -> (Vec<u64>, u64) {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut seqs = Vec::new();
    let mut high_water = 0;
    for line in text.lines() {
        if let Some(hw) = line.strip_prefix("#hw\t") {
            high_water = hw.parse().unwrap_or(0);
        } else if let Some((seq, _)) = line.split_once('\t') {
            if let Ok(seq) = seq.parse() {
                seqs.push(seq);
            }
        }
    }
    (seqs, high_water)
}

#[tokio::test]
async fn telemetry_measured_during_an_outage_is_held_then_delivered_after_the_restart() {
    let Ok(bin) = std::env::var("AGRIPOD_TESTBROKER_BIN") else {
        eprintln!("skipped: set AGRIPOD_TESTBROKER_BIN to the built fleet/testbroker binary");
        return;
    };
    let bin = PathBuf::from(bin);
    let port = free_port();
    let addr = format!("127.0.0.1:{port}");
    let node_id = format!("nuc-drill-{port}");
    let topics = Topics::new(SITE, &node_id).unwrap();

    let mut broker = start_broker(&bin, port);

    // The node measures every 100 ms. Its outbox is a real file, which the drill reads directly.
    let mut cfg = NodeConfig::new(SITE, &node_id, &addr).unwrap();
    cfg.cycle = Duration::from_millis(100);
    cfg.retry = Duration::from_millis(300);
    let site = SiteConfig::parse(SITE_JSON).unwrap();
    let control = ControlLoop::new(&site, Hw, Quiet).unwrap();
    let dir = outbox_dir(&node_id);
    let outbox_file = dir.join("outbox.log");
    let outbox = Outbox::open(&outbox_file, 1_000_000).unwrap();
    let node = Node::new(cfg, control, Sensors, outbox).unwrap();
    let (stop, stopped) = oneshot::channel::<()>();
    let handle = tokio::spawn(tpt_agripod_edge::node::run(node, async {
        let _ = stopped.await;
    }));

    // Before the outage: telemetry reaches a subscriber.
    let before = collect_seqs(&addr, &topics.telemetry, 3, Duration::from_secs(10)).await;
    let last_before = *before.last().unwrap();

    // The outage: the broker is killed. The node keeps measuring into the outbox.
    broker.kill().expect("kill broker");
    let _ = broker.wait();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let (held, _) = read_outbox_file(&outbox_file);
    assert!(
        held.iter().filter(|s| **s > last_before).count() >= 5,
        "telemetry measured during the outage must be held: last before {last_before}, held {held:?}"
    );

    // The restart: the node reconnects and drains the outbox. A message leaves the outbox only
    // after its publish succeeded, so an empty outbox means everything was delivered.
    broker = start_broker(&bin, port);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let (pending, _) = read_outbox_file(&outbox_file);
        if pending.is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "outbox did not drain after the restart: {pending:?}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // The link is live again: a new subscriber receives current telemetry.
    let after = collect_seqs(&addr, &topics.telemetry, 1, Duration::from_secs(20)).await;
    assert!(
        after[0] > last_before,
        "post-restart telemetry must follow the outage: {after:?}"
    );

    let _ = stop.send(());
    let node = timeout(Duration::from_secs(10), handle)
        .await
        .expect("node stops")
        .unwrap()
        .expect("node result");
    let _ = broker.kill();
    let _ = broker.wait();

    let (pending, high_water) = read_outbox_file(&outbox_file);
    assert!(pending.is_empty(), "nothing left undelivered: {pending:?}");
    assert!(
        high_water > last_before + 5,
        "measurement continued through the outage: high water {high_water}"
    );
    assert!(node.pending_telemetry(10).is_empty());
}

/// Subscribes, then collects `count` telemetry sequence numbers, in arrival order.
async fn collect_seqs(addr: &str, topic: &str, count: usize, limit: Duration) -> Vec<u64> {
    let (mut rx, mut tx) = (vec![0u8; 4096], vec![0u8; 8192]);
    let deadline = Instant::now() + limit;
    loop {
        if let Some(seqs) = try_collect(addr, topic, count, &mut rx, &mut tx, deadline).await {
            return seqs;
        }
        assert!(
            Instant::now() < deadline,
            "no telemetry from {addr} in time"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn try_collect(
    addr: &str,
    topic: &str,
    count: usize,
    rx: &mut [u8],
    tx: &mut [u8],
    deadline: Instant,
) -> Option<Vec<u64>> {
    let tcp = timeout(Duration::from_secs(2), TcpStream::connect(addr))
        .await
        .ok()?
        .ok()?;
    let mut session: Session<'_> = Session::new(
        ConfigBuilder::new(Buffers::new(rx, tx))
            .client_id(&format!("drill-{}", uuid_suffix()))
            .ok()?
            .keepalive_interval(30),
    );
    let mut conn: Connection<'_, '_, FromTokio<TcpStream>> =
        timeout(Duration::from_secs(2), session.connect(FromTokio::new(tcp)))
            .await
            .ok()?
            .ok()?;
    timeout(
        Duration::from_secs(2),
        conn.subscribe(&[TopicFilter::new(topic)], &[]),
    )
    .await
    .ok()?
    .ok()?;
    let got = Arc::new(Mutex::new(Vec::new()));
    while got.lock().unwrap().len() < count && Instant::now() < deadline {
        let next = timeout(Duration::from_secs(2), conn.recv()).await;
        match next {
            Ok(Ok(m)) => {
                let text = String::from_utf8_lossy(m.payload()).into_owned();
                if let Some(seq) = json::parse(&text)
                    .ok()
                    .and_then(|v| v.get("seq").and_then(Value::as_f64))
                {
                    got.lock().unwrap().push(seq as u64);
                }
            }
            _ => return None,
        }
    }
    let seqs = got.lock().unwrap().clone();
    (seqs.len() >= count).then_some(seqs)
}

fn uuid_suffix() -> String {
    tpt_agripod_edge::link::uuid_v4().unwrap()[..8].to_string()
}
