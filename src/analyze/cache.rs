use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::Value;

#[derive(Debug, Clone)]
pub struct CachedAnalyzeResult {
    pub payload: Value,
}

#[derive(Debug, Default)]
pub struct AnalyzeCache {
    entries: Mutex<HashMap<String, CachedAnalyzeResult>>,
}

impl AnalyzeCache {
    pub fn get(&self, key: &str) -> Option<CachedAnalyzeResult> {
        self.entries
            .lock()
            .expect("analyze cache mutex was poisoned; a panic occurred while the cache lock was held")
            .get(key)
            .cloned()
    }

    pub fn put(&self, key: String, payload: Value) {
        self.entries
            .lock()
            .expect("analyze cache mutex was poisoned; a panic occurred while the cache lock was held")
            .insert(key, CachedAnalyzeResult { payload });
    }

    pub fn key(repo: &str, branch: &str, commit: &str) -> String {
        format!("{repo}/{branch}/{commit}")
    }
}
