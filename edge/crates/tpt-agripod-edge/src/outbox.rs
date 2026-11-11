// SPDX-License-Identifier: MIT OR Apache-2.0

//! Durable telemetry outbox: telemetry the node has measured but not yet delivered.
//!
//! The control loop never waits on the broker. Each telemetry message is appended here before it
//! is sent, and removed only once the publish call has succeeded. If the broker is down, the node
//! keeps measuring, and the outbox grows up to a byte cap. Past the cap the oldest messages are
//! dropped, and the drop is visible downstream as a gap in `seq`.
//!
//! On disk: one record per line, `seq<TAB>payload`, where the payload is one line of compact JSON.
//! Every change is written to a temp file, fsynced, and renamed over the old file, so a crash
//! leaves the old state or the new one, never a torn file. A torn final line from an older write
//! is skipped on load.

use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// One undelivered telemetry message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The telemetry sequence number. Gaps mean messages were dropped at the cap.
    pub seq: u64,
    /// The compact JSON payload, one line.
    pub payload: String,
}

/// The outbox. Owns its file.
#[derive(Debug)]
pub struct Outbox {
    path: PathBuf,
    cap_bytes: u64,
    records: VecDeque<Record>,
    bytes: u64,
    dropped: u64,
    high_water: u64,
}

impl Outbox {
    /// Opens the outbox at `path`, loading any records a previous run left behind.
    pub fn open(path: &Path, cap_bytes: u64) -> io::Result<Self> {
        if cap_bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "outbox cap must be positive",
            ));
        }
        let mut outbox = Self {
            path: path.to_path_buf(),
            cap_bytes,
            records: VecDeque::new(),
            bytes: 0,
            dropped: 0,
            high_water: 0,
        };
        match fs::read_to_string(path) {
            Ok(text) => {
                // A final line without a newline is a torn write. Skip it.
                let complete = match text.rfind('\n') {
                    Some(i) => &text[..=i],
                    None => "",
                };
                for line in complete.lines() {
                    if let Some(hw) = line.strip_prefix("#hw\t") {
                        outbox.high_water = outbox.high_water.max(hw.parse().unwrap_or(0));
                        continue;
                    }
                    if let Some(record) = parse_line(line) {
                        outbox.bytes += encoded_len(&record);
                        outbox.high_water = outbox.high_water.max(record.seq);
                        outbox.records.push_back(record);
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        Ok(outbox)
    }

    /// Undelivered records, oldest first.
    pub fn pending(&self) -> &VecDeque<Record> {
        &self.records
    }

    /// The highest sequence number this outbox has ever held. A node resumes its sequence from here
    /// after a restart, so a restart never reuses a number.
    pub fn high_water(&self) -> u64 {
        self.high_water
    }

    /// Records dropped at the byte cap since this outbox was opened.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Appends a message, durably. Drops the oldest messages first if the cap would be exceeded.
    pub fn push(&mut self, seq: u64, payload: &str) -> io::Result<()> {
        if payload.contains('\n') || payload.contains('\r') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "payload must be one line",
            ));
        }
        let record = Record {
            seq,
            payload: payload.to_string(),
        };
        let size = encoded_len(&record);
        if size > self.cap_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "one message is larger than the outbox cap",
            ));
        }
        self.high_water = self.high_water.max(seq);
        self.records.push_back(record);
        self.bytes += size;
        while self.bytes > self.cap_bytes {
            if let Some(old) = self.records.pop_front() {
                self.bytes -= encoded_len(&old);
                self.dropped += 1;
            }
        }
        self.persist()
    }

    /// Removes a delivered message. Unknown sequence numbers are ignored.
    pub fn ack(&mut self, seq: u64) -> io::Result<()> {
        let Some(i) = self.records.iter().position(|r| r.seq == seq) else {
            return Ok(());
        };
        if let Some(r) = self.records.remove(i) {
            self.bytes -= encoded_len(&r);
        }
        self.persist()
    }

    fn persist(&self) -> io::Result<()> {
        let mut body = format!("#hw\t{}\n", self.high_water);
        for r in &self.records {
            body.push_str(&format!("{}\t{}\n", r.seq, r.payload));
        }
        write_atomic(&self.path, body.as_bytes())
    }
}

fn encoded_len(r: &Record) -> u64 {
    // seq digits, the tab, the payload, and the newline.
    (r.seq.to_string().len() + 1 + r.payload.len() + 1) as u64
}

fn parse_line(line: &str) -> Option<Record> {
    let (seq, payload) = line.split_once('\t')?;
    let seq: u64 = seq.parse().ok()?;
    if payload.is_empty() {
        return None;
    }
    Some(Record {
        seq,
        payload: payload.to_string(),
    })
}

/// Writes `bytes` to `path` atomically: temp file, fsync, rename.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = path.with_extension("tmp");
    {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all(); // best effort; not supported on every platform
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("agripod-outbox-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir.join("outbox.log")
    }

    #[test]
    fn push_then_ack_keeps_the_rest() {
        let path = temp_path("ack");
        let _ = fs::remove_file(&path);
        let mut o = Outbox::open(&path, 10_000).unwrap();
        o.push(1, r#"{"a":1}"#).unwrap();
        o.push(2, r#"{"a":2}"#).unwrap();
        o.push(3, r#"{"a":3}"#).unwrap();
        o.ack(2).unwrap();
        let seqs: Vec<u64> = o.pending().iter().map(|r| r.seq).collect();
        assert_eq!(seqs, vec![1, 3]);
    }

    #[test]
    fn records_survive_a_restart() {
        let path = temp_path("restart");
        let _ = fs::remove_file(&path);
        {
            let mut o = Outbox::open(&path, 10_000).unwrap();
            o.push(7, r#"{"v":0.31}"#).unwrap();
            o.push(8, r#"{"v":0.32}"#).unwrap();
        }
        let reopened = Outbox::open(&path, 10_000).unwrap();
        let got: Vec<(u64, &str)> = reopened
            .pending()
            .iter()
            .map(|r| (r.seq, r.payload.as_str()))
            .collect();
        assert_eq!(got, vec![(7, r#"{"v":0.31}"#), (8, r#"{"v":0.32}"#)]);
    }

    #[test]
    fn the_cap_drops_the_oldest_and_counts_the_drops() {
        let path = temp_path("cap");
        let _ = fs::remove_file(&path);
        // Each record is "N\t{...}\n": about 16 bytes. A 40-byte cap holds two.
        let mut o = Outbox::open(&path, 25).unwrap();
        for seq in 1..=5u64 {
            o.push(seq, r#"{"x":1}"#).unwrap();
        }
        let seqs: Vec<u64> = o.pending().iter().map(|r| r.seq).collect();
        assert_eq!(seqs, vec![4, 5], "oldest dropped first");
        assert_eq!(o.dropped(), 3);
        assert!(
            fs::metadata(&path).unwrap().len() <= 40,
            "file holds the records plus the header"
        );
    }

    #[test]
    fn a_torn_final_line_is_skipped_on_load() {
        let path = temp_path("torn");
        fs::write(&path, "1\t{\"ok\":true}\n2\t{\"torn\":").unwrap();
        let o = Outbox::open(&path, 10_000).unwrap();
        let seqs: Vec<u64> = o.pending().iter().map(|r| r.seq).collect();
        assert_eq!(seqs, vec![1]);
    }

    #[test]
    fn unparseable_lines_are_skipped_not_fatal() {
        let path = temp_path("junk");
        fs::write(&path, "garbage\n3\t{\"ok\":1}\n\t\n").unwrap();
        let o = Outbox::open(&path, 10_000).unwrap();
        let seqs: Vec<u64> = o.pending().iter().map(|r| r.seq).collect();
        assert_eq!(seqs, vec![3]);
    }

    #[test]
    fn multi_line_payloads_and_oversize_messages_are_refused() {
        let path = temp_path("refuse");
        let _ = fs::remove_file(&path);
        let mut o = Outbox::open(&path, 20).unwrap();
        assert!(o.push(1, "a\nb").is_err());
        assert!(o.push(2, &"x".repeat(50)).is_err());
        assert!(o.pending().is_empty());
    }

    #[test]
    fn a_zero_cap_is_refused() {
        assert!(Outbox::open(&temp_path("zero"), 0).is_err());
    }
}

#[cfg(test)]
mod high_water_tests {
    use super::*;

    #[test]
    fn the_sequence_high_water_survives_full_acknowledgement() {
        let dir = std::env::temp_dir().join(format!("agripod-hw-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("outbox.log");
        let _ = fs::remove_file(&path);
        {
            let mut o = Outbox::open(&path, 10_000).unwrap();
            o.push(1, r#"{"a":1}"#).unwrap();
            o.push(2, r#"{"a":2}"#).unwrap();
            o.push(3, r#"{"a":3}"#).unwrap();
            o.ack(1).unwrap();
            o.ack(2).unwrap();
            o.ack(3).unwrap();
        }
        let reopened = Outbox::open(&path, 10_000).unwrap();
        assert!(reopened.pending().is_empty());
        assert_eq!(
            reopened.high_water(),
            3,
            "a restart must not reuse sequence numbers"
        );
    }
}
