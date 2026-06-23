use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::Value;
use sha2::{Digest, Sha256};

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
            .expect(
                "analyze cache mutex was poisoned; a panic occurred while the cache lock was held",
            )
            .get(key)
            .cloned()
    }

    pub fn put(&self, key: String, payload: Value) {
        self.entries
            .lock()
            .expect(
                "analyze cache mutex was poisoned; a panic occurred while the cache lock was held",
            )
            .insert(key, CachedAnalyzeResult { payload });
    }

    pub fn key(repo: &str, branch: &str, commit: &str, analyze_version: u8) -> String {
        fn update_field(hasher: &mut Sha256, field: &str) {
            let bytes = field.as_bytes();
            hasher.update((bytes.len() as u64).to_be_bytes());
            hasher.update(bytes);
        }

        let mut hasher = Sha256::new();
        update_field(&mut hasher, repo);
        update_field(&mut hasher, branch);
        update_field(&mut hasher, commit);
        hasher.update([analyze_version]);
        format!("{:x}", hasher.finalize())
    }
}
