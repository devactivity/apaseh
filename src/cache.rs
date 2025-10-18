use crate::http::HttpResponse;
use lru::LruCache;
use parking_lot::RwLock;
use std::{
    num::NonZeroUsize,
    sync::Arc,
    time::{Duration, Instant},
};

pub struct ResponseCache {
    cache: Arc<RwLock<LruCache<String, CacheEntry>>>,
    ttl: Duration,
}

#[derive(Clone)]
struct CacheEntry {
    response: HttpResponse,
    created_at: Instant,
}

impl ResponseCache {
    pub fn new(size_bytes: usize, ttl_seconds: u64) -> Self {
        // estimate cache capacity
        // based on avg response size
        let capacity =
            NonZeroUsize::new(size_bytes / 4096).unwrap_or(NonZeroUsize::new(1000).unwrap());

        Self {
            cache: Arc::new(RwLock::new(LruCache::new(capacity))),
            ttl: Duration::from_secs(ttl_seconds),
        }
    }

    pub fn get(&self, key: &str) -> Option<HttpResponse> {
        let mut cache = self.cache.write();

        if let Some(entry) = cache.get(key) {
            if entry.created_at.elapsed() < self.ttl {
                return Some(entry.response.clone());
            } else {
                cache.pop(key);
            }
        }

        None
    }

    pub fn put(&self, key: String, response: HttpResponse) {
        let entry = CacheEntry {
            response,
            created_at: Instant::now(),
        };

        let mut cache = self.cache.write();
        cache.put(key, entry);
    }
}
