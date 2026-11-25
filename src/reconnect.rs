use std::time::{Duration, Instant};

use url::Url;

pub struct Reconnect {
    pub url: Url,
    pub name: String,
    pub due: Option<Instant>,
    pub reason: String,
    pub attempts: u64,
    previous: u64,
    next: u64,
}

impl Reconnect {
    pub fn new(url: Url, name: String) -> Self {
        Self {
            url,
            name,
            due: None,
            reason: String::new(),
            attempts: 0,
            previous: 0,
            next: 1,
        }
    }

    pub fn schedule(&mut self, now: Instant, reason: String) {
        let delay = self.next;
        (self.previous, self.next) = (self.next, (self.previous + self.next).min(60));
        self.attempts = self.attempts.saturating_add(1);
        self.reason = reason;
        self.due = Some(now + Duration::from_secs(delay));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fibonacci_delays_are_capped_and_reset_after_a_new_session() {
        let now = Instant::now();
        let mut retry = Reconnect::new(
            Url::parse("https://example.org/live").unwrap(),
            "Radio".into(),
        );
        for seconds in [1, 1, 2, 3, 5, 8, 13, 21, 34, 55, 60, 60, 60] {
            retry.schedule(now, "Offline".into());
            assert_eq!(retry.due.unwrap().duration_since(now).as_secs(), seconds);
        }
        assert_eq!(retry.attempts, 13);
        let mut fresh = Reconnect::new(retry.url, retry.name);
        fresh.schedule(now, "Offline".into());
        assert_eq!(fresh.due.unwrap().duration_since(now).as_secs(), 1);
    }
}
