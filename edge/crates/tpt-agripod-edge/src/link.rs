// SPDX-License-Identifier: MIT OR Apache-2.0

//! The node's side of the fleet protocol (docs/mqtt-topics.md): topic names, the common envelope,
//! message builders, and profile-push handling with an idempotent ack cache.
//!
//! Nothing here touches the network. The node runner (`node.rs`) owns the connection and calls
//! these functions, so the protocol rules can be tested without a broker.

use std::collections::VecDeque;

use crate::json::{self, Value};
use crate::profile::{ApplyOutcome, Problem, ProfileStore};

/// Largest push payload accepted, bytes. Larger payloads are refused without parsing.
pub const MAX_PUSH_BYTES: usize = 128 * 1024;

/// Topics for one node, below `agripod/{site}/{node}/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Topics {
    /// `profile/push`, subscribed.
    pub profile_push: String,
    /// `profile/ack`, published.
    pub profile_ack: String,
    /// `telemetry`, published.
    pub telemetry: String,
    /// `command`, subscribed.
    pub command: String,
    /// `command/ack`, published.
    pub command_ack: String,
    /// `status`, published (retained), and the last-will.
    pub status: String,
}

impl Topics {
    /// Builds the topics for `site` and `node`. Both must be slugs.
    pub fn new(site: &str, node: &str) -> Result<Self, String> {
        if !is_slug(site) {
            return Err(format!("site id {site:?} is not a slug"));
        }
        if !is_slug(node) {
            return Err(format!("node id {node:?} is not a slug"));
        }
        let base = format!("agripod/{site}/{node}");
        Ok(Self {
            profile_push: format!("{base}/profile/push"),
            profile_ack: format!("{base}/profile/ack"),
            telemetry: format!("{base}/telemetry"),
            command: format!("{base}/command"),
            command_ack: format!("{base}/command/ack"),
            status: format!("{base}/status"),
        })
    }
}

/// True for a lower-case slug of 2 to 63 characters, as the topic map defines.
pub fn is_slug(s: &str) -> bool {
    let b = s.as_bytes();
    (2..=63).contains(&b.len())
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// True for a canonical UUID string (lower-case hex, 8-4-4-4-12).
pub fn is_uuid(s: &str) -> bool {
    let groups: Vec<&str> = s.split('-').collect();
    groups.len() == 5
        && [8, 4, 4, 4, 12].iter().zip(&groups).all(|(len, g)| {
            g.len() == *len
                && g.bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        })
}

/// A random version 4 UUID, from the operating system's random source.
pub fn uuid_v4() -> Result<String, String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|e| format!("random source failed: {e}"))?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0],
        b[1],
        b[2],
        b[3],
        b[4],
        b[5],
        b[6],
        b[7],
        b[8],
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15]
    ))
}

/// RFC 3339 UTC with a `Z` suffix, for a Unix time in seconds.
pub fn rfc3339_utc(unix_s: u64) -> String {
    let days = (unix_s / 86_400) as i64;
    let secs = unix_s % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// The common header on every message the node publishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    /// Fresh UUID for this message.
    pub msg_id: String,
    /// RFC 3339 UTC.
    pub ts: String,
    /// Site id.
    pub site_id: String,
    /// Node id.
    pub node_id: String,
}

impl Envelope {
    /// A new envelope stamped at `now_unix` with a fresh msg_id.
    pub fn new(site: &str, node: &str, now_unix: u64) -> Result<Self, String> {
        Ok(Self {
            msg_id: uuid_v4()?,
            ts: rfc3339_utc(now_unix),
            site_id: site.to_string(),
            node_id: node.to_string(),
        })
    }

    fn members(&self) -> String {
        format!(
            "\"msg_id\":{},\"ts\":{},\"site_id\":{},\"node_id\":{}",
            json::quote(&self.msg_id),
            json::quote(&self.ts),
            json::quote(&self.site_id),
            json::quote(&self.node_id)
        )
    }
}

/// A `status` message. Retained, so a late subscriber sees the last state.
pub fn status_message(
    env: &Envelope,
    state: &str,
    profile: Option<(&str, &str)>,
    uptime_s: u64,
) -> String {
    let mut s = format!("{{{},\"state\":{}", env.members(), json::quote(state));
    if let Some((id, version)) = profile {
        s.push_str(&format!(
            ",\"profile_id\":{},\"profile_version\":{}",
            json::quote(id),
            json::quote(version)
        ));
    }
    s.push_str(&format!(",\"uptime_s\":{uptime_s}}}"));
    s
}

/// One reading in a telemetry batch. Only good readings are sent.
#[derive(Debug, Clone, PartialEq)]
pub struct TelemetrySample {
    /// Channel name, lower_snake_case.
    pub channel: &'static str,
    /// Unit symbol, as in docs/units.md.
    pub unit: &'static str,
    /// Reading time, Unix seconds.
    pub at_s: u64,
    /// Value. Must be finite.
    pub value: f64,
}

/// A `telemetry` batch.
pub fn telemetry_message(env: &Envelope, seq: u64, samples: &[TelemetrySample]) -> String {
    let items: Vec<String> = samples
        .iter()
        .map(|s| {
            format!(
                "{{\"channel\":{},\"ts\":{},\"value\":{},\"unit\":{},\"quality\":\"good\"}}",
                json::quote(s.channel),
                json::quote(&rfc3339_utc(s.at_s)),
                s.value,
                json::quote(s.unit)
            )
        })
        .collect();
    format!(
        "{{{},\"seq\":{seq},\"samples\":[{}]}}",
        env.members(),
        items.join(",")
    )
}

/// A `profile/ack`.
pub fn profile_ack_message(
    env: &Envelope,
    ack_of: &str,
    profile_id: &str,
    profile_version: &str,
    applied: bool,
    problems: &[Problem],
) -> String {
    let status = if applied { "applied" } else { "rejected" };
    let errors: Vec<String> = problems
        .iter()
        .map(|p| {
            format!(
                "{{\"path\":{},\"message\":{}}}",
                json::quote(&p.path),
                json::quote(&p.message)
            )
        })
        .collect();
    let mut s = format!(
        "{{{},\"ack_of\":{},\"profile_id\":{},\"profile_version\":{},\"status\":\"{status}\"",
        env.members(),
        json::quote(ack_of),
        json::quote(profile_id),
        json::quote(profile_version)
    );
    if !problems.is_empty() {
        s.push_str(&format!(",\"errors\":[{}]", errors.join(",")));
    }
    s.push('}');
    s
}

/// A `command/ack` that refuses the command.
pub fn command_ack_rejected(env: &Envelope, ack_of: &str, reason: &str) -> String {
    format!(
        "{{{},\"ack_of\":{},\"status\":\"rejected\",\"reason\":{}}}",
        env.members(),
        json::quote(ack_of),
        json::quote(reason)
    )
}

/// Recently sent profile acks, by the msg_id of the push they answer. A push that arrives again
/// (the orchestrator retried it) gets the same ack back, and the profile is not applied twice.
#[derive(Debug)]
pub struct AckCache {
    cap: usize,
    order: VecDeque<(String, String)>,
}

impl AckCache {
    /// A cache that remembers the last `cap` acks.
    pub fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            order: VecDeque::new(),
        }
    }

    fn get(&self, msg_id: &str) -> Option<&str> {
        self.order
            .iter()
            .find(|(id, _)| id == msg_id)
            .map(|(_, ack)| ack.as_str())
    }

    fn put(&mut self, msg_id: String, ack: String) {
        if self.order.len() == self.cap {
            self.order.pop_front();
        }
        self.order.push_back((msg_id, ack));
    }
}

/// What a profile push produced.
#[derive(Debug, Clone, PartialEq)]
pub struct PushOutcome {
    /// The ack to publish on `profile/ack`.
    pub ack: String,
    /// Set when this push made a new profile active. The caller applies it to the control loop.
    pub newly_applied: Option<crate::profile::ActiveProfile>,
    /// True when the ack came from the cache, because the push was a repeat.
    pub repeat: bool,
}

/// Handles one `profile/push` payload addressed to this node.
///
/// Refused without an ack: payloads that are too large, not JSON, not addressed to this site and
/// node, or without a usable msg_id. There is nothing to answer. Every other push gets an ack,
/// applied or rejected, and the rejected profile never changes the store.
pub fn handle_push(
    site: &str,
    node: &str,
    payload: &str,
    store: &mut ProfileStore,
    acks: &mut AckCache,
    now_unix: u64,
) -> Result<PushOutcome, String> {
    if payload.len() > MAX_PUSH_BYTES {
        return Err("profile push is larger than 128 KiB".into());
    }
    let root = json::parse(payload)
        .map_err(|e| format!("push is not JSON at byte {}: {}", e.offset, e.message))?;
    let msg_id = root
        .get("msg_id")
        .and_then(Value::as_str)
        .filter(|id| is_uuid(id))
        .ok_or("push has no valid msg_id")?
        .to_string();
    if let Some(ack) = acks.get(&msg_id) {
        return Ok(PushOutcome {
            ack: ack.to_string(),
            newly_applied: None,
            repeat: true,
        });
    }
    let addressed = root.get("site_id").and_then(Value::as_str) == Some(site)
        && root.get("node_id").and_then(Value::as_str) == Some(node);
    if !addressed {
        return Err("push is not addressed to this site and node".into());
    }
    let profile = root.get("profile").ok_or("push has no profile")?;
    let raw_id = profile
        .get("profile_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let raw_version = profile
        .get("profile_version")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let profile_text = json::to_string(profile);

    let env = Envelope::new(site, node, now_unix)?;
    let (ack, newly_applied) = match store.apply(&profile_text) {
        ApplyOutcome::Applied(active) => {
            let ack = profile_ack_message(
                &env,
                &msg_id,
                &active.profile_id,
                &active.profile_version,
                true,
                &[],
            );
            (ack, Some(active))
        }
        ApplyOutcome::Rejected(problems) => {
            let ack = profile_ack_message(&env, &msg_id, &raw_id, &raw_version, false, &problems);
            (ack, None)
        }
    };
    acks.put(msg_id, ack.clone());
    Ok(PushOutcome {
        ack,
        newly_applied,
        repeat: false,
    })
}

/// Parses the msg_id of an inbound command, for the ack that refuses it.
pub fn command_msg_id(payload: &str) -> Option<String> {
    let root = json::parse(payload).ok()?;
    root.get("msg_id")
        .and_then(Value::as_str)
        .filter(|id| is_uuid(id))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SITE: &str = "wgtn-bay-01";
    const NODE: &str = "nuc-01";
    const EXAMPLE: &str = include_str!("../../../../profiles/examples/example-plant-v0.json");
    const PUSH_ID: &str = "0190f3a2-7c1e-7d4b-9a3e-2f5c8e1b6d10";

    fn push(profile: &str, msg_id: &str) -> String {
        format!(
            "{{\"msg_id\":\"{msg_id}\",\"ts\":\"2026-11-11T02:15:00Z\",\"site_id\":\"{SITE}\",\"node_id\":\"{NODE}\",\"profile\":{profile}}}"
        )
    }

    #[test]
    fn rfc3339_matches_known_instants() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(1_767_225_600), "2026-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(1_709_208_000), "2024-02-29T12:00:00Z");
        assert_eq!(rfc3339_utc(4_102_444_799), "2099-12-31T23:59:59Z");
    }

    #[test]
    fn topics_follow_the_map_and_reject_bad_slugs() {
        let t = Topics::new(SITE, NODE).unwrap();
        assert_eq!(t.profile_push, "agripod/wgtn-bay-01/nuc-01/profile/push");
        assert_eq!(t.status, "agripod/wgtn-bay-01/nuc-01/status");
        assert!(Topics::new("Wgtn", NODE).is_err());
        assert!(Topics::new(SITE, "a/b").is_err());
    }

    #[test]
    fn uuid_v4_is_well_formed_and_random() {
        let a = uuid_v4().unwrap();
        let b = uuid_v4().unwrap();
        assert!(is_uuid(&a), "{a}");
        assert_eq!(a.as_bytes()[14], b'4', "version nibble");
        assert_ne!(a, b);
    }

    #[test]
    fn messages_are_valid_json_with_the_envelope() {
        let env = Envelope {
            msg_id: PUSH_ID.into(),
            ts: rfc3339_utc(1_767_225_600),
            site_id: SITE.into(),
            node_id: NODE.into(),
        };
        for text in [
            status_message(&env, "online", Some(("example-plant", "0.1.0")), 42),
            status_message(&env, "safe_state", None, 0),
            telemetry_message(
                &env,
                7,
                &[TelemetrySample {
                    channel: "moisture_frac",
                    unit: "frac",
                    at_s: 1_767_225_600,
                    value: 0.31,
                }],
            ),
            profile_ack_message(
                &env,
                PUSH_ID,
                "x",
                "0.0.1",
                false,
                &[Problem {
                    path: "/controls/pump_duty/max".into(),
                    message: "say \"no\"".into(),
                }],
            ),
            command_ack_rejected(&env, PUSH_ID, "not implemented"),
        ] {
            let v = json::parse(&text).unwrap_or_else(|e| panic!("{text}: {e:?}"));
            assert_eq!(v.get("node_id").and_then(Value::as_str), Some(NODE));
            assert_eq!(
                v.get("ts").and_then(Value::as_str),
                Some("2026-01-01T00:00:00Z")
            );
        }
    }

    #[test]
    fn telemetry_carries_seq_and_good_quality() {
        let env = Envelope {
            msg_id: PUSH_ID.into(),
            ts: "2026-01-01T00:00:00Z".into(),
            site_id: SITE.into(),
            node_id: NODE.into(),
        };
        let text = telemetry_message(
            &env,
            9,
            &[TelemetrySample {
                channel: "reservoir_level_l",
                unit: "l",
                at_s: 0,
                value: 80.5,
            }],
        );
        let v = json::parse(&text).unwrap();
        assert_eq!(v.get("seq").and_then(Value::as_f64), Some(9.0));
        let s = &v.get("samples").and_then(Value::as_array).unwrap()[0];
        assert_eq!(s.get("quality").and_then(Value::as_str), Some("good"));
        assert_eq!(s.get("value").and_then(Value::as_f64), Some(80.5));
    }

    #[test]
    fn a_valid_push_is_applied_and_acked() {
        let mut store = ProfileStore::new();
        let mut acks = AckCache::new(16);
        let out = handle_push(
            SITE,
            NODE,
            &push(EXAMPLE, PUSH_ID),
            &mut store,
            &mut acks,
            1_767_225_600,
        )
        .unwrap();
        assert!(!out.repeat);
        let active = out.newly_applied.expect("valid profile must apply");
        assert_eq!(active.profile_id, "example-plant");
        let ack = json::parse(&out.ack).unwrap();
        assert_eq!(ack.get("status").and_then(Value::as_str), Some("applied"));
        assert_eq!(ack.get("ack_of").and_then(Value::as_str), Some(PUSH_ID));
        assert_eq!(
            store.active().map(|p| p.profile_version.as_str()),
            Some("0.1.0")
        );
    }

    #[test]
    fn an_unsafe_push_is_rejected_with_reasons_and_changes_nothing() {
        let mut store = ProfileStore::new();
        let mut acks = AckCache::new(16);
        handle_push(
            SITE,
            NODE,
            &push(EXAMPLE, PUSH_ID),
            &mut store,
            &mut acks,
            0,
        )
        .unwrap();
        let bad = EXAMPLE.replacen(r#""max": 0.15,"#, r#""max": 0.9,"#, 1);
        let out = handle_push(
            SITE,
            NODE,
            &push(&bad, "0190f3a2-7c1e-7d4b-9a3e-2f5c8e1b6d11"),
            &mut store,
            &mut acks,
            0,
        )
        .unwrap();
        assert!(out.newly_applied.is_none());
        let ack = json::parse(&out.ack).unwrap();
        assert_eq!(ack.get("status").and_then(Value::as_str), Some("rejected"));
        let errors = ack.get("errors").and_then(Value::as_array).unwrap();
        assert!(!errors.is_empty());
        assert_eq!(
            store.active().map(|p| p.profile_version.as_str()),
            Some("0.1.0"),
            "previous profile kept"
        );
    }

    #[test]
    fn a_repeated_push_gets_the_same_ack_and_is_not_applied_twice() {
        let mut store = ProfileStore::new();
        let mut acks = AckCache::new(16);
        let first = handle_push(
            SITE,
            NODE,
            &push(EXAMPLE, PUSH_ID),
            &mut store,
            &mut acks,
            0,
        )
        .unwrap();
        let again = handle_push(
            SITE,
            NODE,
            &push(EXAMPLE, PUSH_ID),
            &mut store,
            &mut acks,
            5,
        )
        .unwrap();
        assert!(again.repeat);
        assert!(again.newly_applied.is_none());
        assert_eq!(first.ack, again.ack);
    }

    #[test]
    fn pushes_not_for_this_node_or_without_a_msg_id_get_no_ack() {
        let mut store = ProfileStore::new();
        let mut acks = AckCache::new(16);
        let other = push(EXAMPLE, PUSH_ID).replace(NODE, "nuc-02");
        assert!(handle_push(SITE, NODE, &other, &mut store, &mut acks, 0).is_err());
        assert!(
            handle_push(
                SITE,
                NODE,
                &push(EXAMPLE, "not-a-uuid"),
                &mut store,
                &mut acks,
                0
            )
            .is_err()
        );
        assert!(handle_push(SITE, NODE, "{", &mut store, &mut acks, 0).is_err());
        assert!(store.active().is_none());
    }

    #[test]
    fn ack_cache_is_bounded() {
        let mut acks = AckCache::new(2);
        acks.put("a".into(), "1".into());
        acks.put("b".into(), "2".into());
        acks.put("c".into(), "3".into());
        assert!(acks.get("a").is_none());
        assert_eq!(acks.get("c"), Some("3"));
    }
}
