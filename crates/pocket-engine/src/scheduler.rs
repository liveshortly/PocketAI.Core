//! ENG-10: admission ordering and fairness for concurrent generation
//! requests. Deliberately a plain synchronous queue (no tokio/async) - the
//! actual blocking-until-admitted wait is the caller's job (an
//! `mpsc`/`oneshot` wrapped around `submit`/`finish`); this module is only
//! the ordering/fairness policy, which is what the card's "Done when"
//! actually tests.
/// A§4.4's ordering: interactive (UI) > remote device > API > background.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Background,
    Api,
    RemoteDevice,
    Interactive,
}

struct Queued {
    id: String,
    priority: Priority,
    seq: u64,
}

pub struct Scheduler {
    max_concurrent: usize,
    running: usize,
    waiting: Vec<Queued>,
    next_seq: u64,
}

impl Scheduler {
    /// `max_concurrent`: 2 on Mac (if memory allows - the caller decides
    /// that before calling `new`/resizing), 1 on iPhone.
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            max_concurrent: max_concurrent.max(1),
            running: 0,
            waiting: Vec::new(),
            next_seq: 0,
        }
    }

    /// Submits a request; returns its 0-based queue position (0 means
    /// admitted immediately and now counted in `running()`).
    pub fn submit(&mut self, id: impl Into<String>, priority: Priority) -> usize {
        if self.running < self.max_concurrent {
            self.running += 1;
            return 0;
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.waiting.push(Queued {
            id: id.into(),
            priority,
            seq,
        });
        self.sort_waiting();
        self.position_of(&self.waiting.last().unwrap().id).unwrap()
    }

    /// Call when a running request completes or is cancelled; admits the
    /// next-highest-priority (then earliest-arrived) waiting request if
    /// there's room, returning its id.
    pub fn finish(&mut self, _completed_id: &str) -> Option<String> {
        self.running = self.running.saturating_sub(1);
        if self.running >= self.max_concurrent || self.waiting.is_empty() {
            return None;
        }
        let next = self.waiting.remove(0);
        self.running += 1;
        Some(next.id)
    }

    /// 1-based position for an SSE `: queued N` comment, or `None` if the
    /// id isn't currently waiting (already running, or unknown).
    pub fn position_of(&self, id: &str) -> Option<usize> {
        self.waiting.iter().position(|q| q.id == id).map(|i| i + 1)
    }

    pub fn running(&self) -> usize {
        self.running
    }

    pub fn waiting_ids_in_order(&self) -> Vec<String> {
        self.waiting.iter().map(|q| q.id.clone()).collect()
    }

    fn sort_waiting(&mut self) {
        // Higher priority first; FIFO (ascending seq) within the same tier.
        self.waiting
            .sort_by(|a, b| b.priority.cmp(&a.priority).then(a.seq.cmp(&b.seq)));
    }
}

pub fn sse_queued_comment(position: usize) -> String {
    format!(": queued {position}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admits_immediately_up_to_max_concurrent() {
        let mut s = Scheduler::new(2);
        assert_eq!(s.submit("a", Priority::Api), 0);
        assert_eq!(s.submit("b", Priority::Api), 0);
        assert_eq!(s.running(), 2);
    }

    #[test]
    fn queues_once_at_capacity() {
        let mut s = Scheduler::new(1);
        assert_eq!(s.submit("a", Priority::Api), 0);
        assert_eq!(s.submit("b", Priority::Api), 1);
        assert_eq!(s.running(), 1);
        assert_eq!(s.waiting_ids_in_order(), vec!["b"]);
    }

    #[test]
    fn higher_priority_jumps_the_queue_ahead_of_lower_priority_arrivals() {
        let mut s = Scheduler::new(1);
        s.submit("running", Priority::Api); // fills the one slot
        s.submit("background-job", Priority::Background);
        s.submit("api-job", Priority::Api);
        s.submit("interactive-job", Priority::Interactive);
        s.submit("remote-job", Priority::RemoteDevice);

        assert_eq!(
            s.waiting_ids_in_order(),
            vec!["interactive-job", "remote-job", "api-job", "background-job"]
        );
    }

    #[test]
    fn same_priority_requests_stay_fifo_fairness() {
        let mut s = Scheduler::new(1);
        s.submit("running", Priority::Api);
        s.submit("a", Priority::Background);
        s.submit("b", Priority::Background);
        s.submit("c", Priority::Background);
        assert_eq!(s.waiting_ids_in_order(), vec!["a", "b", "c"]);
    }

    #[test]
    fn finish_admits_the_next_highest_priority_waiting_request() {
        let mut s = Scheduler::new(1);
        s.submit("running", Priority::Api);
        s.submit("low", Priority::Background);
        s.submit("high", Priority::Interactive);

        let admitted = s.finish("running");
        assert_eq!(admitted, Some("high".to_string()));
        assert_eq!(s.running(), 1);
        assert_eq!(s.waiting_ids_in_order(), vec!["low"]);
    }

    #[test]
    fn finish_returns_none_when_nothing_is_waiting() {
        let mut s = Scheduler::new(1);
        s.submit("only", Priority::Api);
        assert_eq!(s.finish("only"), None);
        assert_eq!(s.running(), 0);
    }

    #[test]
    fn mac_allows_two_concurrent_iphone_allows_one() {
        let mut mac = Scheduler::new(2);
        assert_eq!(mac.submit("a", Priority::Interactive), 0);
        assert_eq!(mac.submit("b", Priority::Interactive), 0);
        assert_eq!(mac.submit("c", Priority::Interactive), 1);

        let mut iphone = Scheduler::new(1);
        assert_eq!(iphone.submit("a", Priority::Interactive), 0);
        assert_eq!(iphone.submit("b", Priority::Interactive), 1);
    }

    #[test]
    fn position_of_reports_one_based_queue_position() {
        let mut s = Scheduler::new(1);
        s.submit("running", Priority::Api);
        s.submit("a", Priority::Background);
        s.submit("b", Priority::Background);
        assert_eq!(s.position_of("a"), Some(1));
        assert_eq!(s.position_of("b"), Some(2));
        assert_eq!(s.position_of("running"), None);
        assert_eq!(s.position_of("nonexistent"), None);
    }

    #[test]
    fn sse_comment_format() {
        assert_eq!(sse_queued_comment(2), ": queued 2");
    }

    #[test]
    fn priority_ordering_matches_the_spec() {
        assert!(Priority::Interactive > Priority::RemoteDevice);
        assert!(Priority::RemoteDevice > Priority::Api);
        assert!(Priority::Api > Priority::Background);
    }
}
