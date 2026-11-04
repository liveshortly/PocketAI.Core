//! MAC-23: the last 200 API requests plus today's totals (local day), for the
//! Mac's Server page. In memory only - a restart starts from zero.
use std::collections::VecDeque;
use std::sync::Mutex;

pub const REQUEST_LOG_CAPACITY: usize = 200;

#[derive(Debug, Clone, PartialEq)]
pub struct RequestRecord {
    pub ts_ms: i64,
    /// Paired device name, "API key", or "local".
    pub client: String,
    pub endpoint: String,
    pub model: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub duration_ms: u32,
    /// Time spent decoding `completion_tokens`; 0 = unknown (left out of tok/s).
    pub decode_ms: u32,
    pub status: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RequestStats {
    pub requests_today: u64,
    pub tokens_out_today: u64,
    pub avg_tokens_per_sec: f64,
}

#[derive(Default)]
struct Inner {
    log: VecDeque<RequestRecord>,
    day: i64,
    requests: u64,
    tokens_out: u64,
    decoded_tokens: u64,
    decode_ms: u64,
}

#[derive(Default)]
pub struct RequestLog(Mutex<Inner>);

impl RequestLog {
    pub fn record(&self, r: RequestRecord) {
        self.record_on(local_day(r.ts_ms), r);
    }

    pub fn stats(&self) -> RequestStats {
        self.stats_on(local_day(now_ms()))
    }

    /// Newest first.
    pub fn recent(&self, limit: usize) -> Vec<RequestRecord> {
        let g = self.0.lock().unwrap();
        g.log.iter().rev().take(limit).cloned().collect()
    }

    fn record_on(&self, day: i64, r: RequestRecord) {
        let mut g = self.0.lock().unwrap();
        if g.day != day {
            let log = std::mem::take(&mut g.log);
            *g = Inner { log, day, ..Default::default() };
        }
        g.requests += 1;
        g.tokens_out += r.completion_tokens as u64;
        if r.decode_ms > 0 {
            g.decoded_tokens += r.completion_tokens as u64;
            g.decode_ms += r.decode_ms as u64;
        }
        if g.log.len() == REQUEST_LOG_CAPACITY {
            g.log.pop_front();
        }
        g.log.push_back(r);
    }

    fn stats_on(&self, day: i64) -> RequestStats {
        let g = self.0.lock().unwrap();
        if g.day != day {
            return RequestStats::default();
        }
        RequestStats {
            requests_today: g.requests,
            tokens_out_today: g.tokens_out,
            avg_tokens_per_sec: if g.decode_ms == 0 {
                0.0
            } else {
                g.decoded_tokens as f64 * 1000.0 / g.decode_ms as f64
            },
        }
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// A key that changes at local midnight (year * 1000 + day of year).
fn local_day(ts_ms: i64) -> i64 {
    let t = (ts_ms / 1000) as libc::time_t;
    // SAFETY: `tm` is plain old data; localtime_r only writes into it.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    tm.tm_year as i64 * 1000 + tm.tm_yday as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(i: u32, completion: u32, decode_ms: u32) -> RequestRecord {
        RequestRecord {
            ts_ms: i as i64,
            client: "local".into(),
            endpoint: "/v1/chat/completions".into(),
            model: "m".into(),
            prompt_tokens: 1,
            completion_tokens: completion,
            duration_ms: 10,
            decode_ms,
            status: 200,
        }
    }

    #[test]
    fn ring_buffer_keeps_last_200_newest_first() {
        let log = RequestLog::default();
        for i in 0..250 {
            log.record_on(1, rec(i, 0, 0));
        }
        let all = log.recent(usize::MAX);
        assert_eq!(all.len(), REQUEST_LOG_CAPACITY);
        assert_eq!(all[0].ts_ms, 249);
        assert_eq!(all[199].ts_ms, 50);
        assert_eq!(log.recent(3).iter().map(|r| r.ts_ms).collect::<Vec<_>>(), [249, 248, 247]);
        // Totals count every request, not just the ones still in the buffer.
        assert_eq!(log.stats_on(1).requests_today, 250);
    }

    #[test]
    fn stats_average_decode_rate_and_reset_on_new_day() {
        let log = RequestLog::default();
        assert_eq!(log.stats_on(1), RequestStats::default());
        log.record_on(1, rec(0, 100, 1000)); // 100 tok in 1 s
        log.record_on(1, rec(1, 300, 2000)); // 300 tok in 2 s
        log.record_on(1, rec(2, 50, 0)); // decode time unknown: counted, not averaged
        let s = log.stats_on(1);
        assert_eq!(s.requests_today, 3);
        assert_eq!(s.tokens_out_today, 450);
        assert!((s.avg_tokens_per_sec - 400.0 / 3.0).abs() < 1e-9);

        // Next day: yesterday's totals read as zero, then start over; the log stays.
        assert_eq!(log.stats_on(2), RequestStats::default());
        log.record_on(2, rec(3, 10, 100));
        let s = log.stats_on(2);
        assert_eq!((s.requests_today, s.tokens_out_today), (1, 10));
        assert!((s.avg_tokens_per_sec - 100.0).abs() < 1e-9);
        assert_eq!(log.recent(usize::MAX).len(), 4);
    }

    #[test]
    fn local_day_is_stable_within_a_day() {
        let now = now_ms();
        assert_eq!(local_day(now), local_day(now));
        assert_ne!(local_day(now), local_day(now + 86_400_000));
    }
}
