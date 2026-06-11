use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Default)]
pub(crate) struct NonceCache {
    seen: Arc<Mutex<HashMap<String, u64>>>,
}

impl NonceCache {
    const TTL: Duration = Duration::from_secs(600);

    pub(crate) fn check_and_store(&self, key_id: &str, nonce: &str, now: u64) -> bool {
        let mut seen = self.seen.lock().expect("nonce cache mutex poisoned");
        let cutoff = now.saturating_sub(Self::TTL.as_secs());
        seen.retain(|_, seen_at| *seen_at >= cutoff);

        let cache_key = format!("{key_id}:{nonce}");
        if seen.contains_key(&cache_key) {
            return false;
        }

        seen.insert(cache_key, now);
        true
    }
}
