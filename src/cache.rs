use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use tokio::sync::{Mutex, RwLock, RwLockReadGuard};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CacheKey {
    pub app_token: String,
    pub table_id: String,
}

impl CacheKey {
    pub fn new(app_token: impl Into<String>, table_id: impl Into<String>) -> Self {
        Self {
            app_token: app_token.into(),
            table_id: table_id.into(),
        }
    }
}

#[derive(Default)]
pub struct BitableCache {
    values: RwLock<HashMap<CacheKey, Arc<serde_json::Value>>>,
    key_locks: Mutex<HashMap<CacheKey, Arc<Mutex<()>>>>,
    subscriptions: RwLock<HashMap<String, HashSet<String>>>,
    subscription_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    invalidation_barrier: RwLock<()>,
}

impl BitableCache {
    pub async fn get(&self, key: &CacheKey) -> Option<Arc<serde_json::Value>> {
        self.values.read().await.get(key).cloned()
    }

    pub async fn insert(&self, key: CacheKey, value: serde_json::Value) -> Arc<serde_json::Value> {
        let value = Arc::new(value);
        self.values.write().await.insert(key, value.clone());
        value
    }

    pub async fn key_lock(&self, key: &CacheKey) -> Arc<Mutex<()>> {
        let mut locks = self.key_locks.lock().await;
        locks
            .entry(key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    pub async fn ensure_subscription<F, Fut>(
        &self,
        app_token: &str,
        table_id: &str,
        subscribe: F,
    ) -> Result<(), String>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<(), String>>,
    {
        if self
            .subscriptions
            .read()
            .await
            .get(app_token)
            .is_some_and(|tables| tables.contains(table_id))
        {
            return Ok(());
        }

        let key_lock = {
            let mut locks = self.subscription_locks.lock().await;
            locks
                .entry(app_token.to_owned())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _guard = key_lock.lock().await;
        if self.subscriptions.read().await.contains_key(app_token) {
            self.subscriptions
                .write()
                .await
                .entry(app_token.to_owned())
                .or_default()
                .insert(table_id.to_owned());
            return Ok(());
        }

        subscribe().await?;
        self.subscriptions
            .write()
            .await
            .entry(app_token.to_owned())
            .or_default()
            .insert(table_id.to_owned());
        Ok(())
    }

    #[cfg(test)]
    pub(crate) async fn subscription_tables(&self, app_token: &str) -> HashSet<String> {
        self.subscriptions
            .read()
            .await
            .get(app_token)
            .cloned()
            .unwrap_or_default()
    }

    /// 首次装载与缓存写入期间持有读屏障，保证失效操作不会被旧请求越过。
    pub async fn read_transaction(&self) -> RwLockReadGuard<'_, ()> {
        self.invalidation_barrier.read().await
    }

    pub async fn invalidate_tables(&self, app_token: &str, table_ids: &[String]) -> usize {
        let _barrier = self.invalidation_barrier.write().await;
        let mut values = self.values.write().await;
        let before = values.len();
        values.retain(|key, _| {
            key.app_token != app_token || !table_ids.iter().any(|id| id == &key.table_id)
        });
        before - values.len()
    }

    pub async fn invalidate_bitable(&self, app_token: &str) -> usize {
        let _barrier = self.invalidation_barrier.write().await;
        let mut values = self.values.write().await;
        let before = values.len();
        values.retain(|key, _| key.app_token != app_token);
        before - values.len()
    }

    pub async fn invalidate_all(&self) -> usize {
        let _barrier = self.invalidation_barrier.write().await;
        let mut values = self.values.write().await;
        let count = values.len();
        values.clear();
        count
    }

    #[cfg(test)]
    pub(crate) async fn len(&self) -> usize {
        self.values.read().await.len()
    }

    #[cfg(test)]
    pub(crate) async fn is_empty(&self) -> bool {
        self.values.read().await.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn invalidates_only_the_changed_table() {
        let cache = BitableCache::default();
        cache
            .insert(CacheKey::new("app-a", "table-1"), json!([1]))
            .await;
        cache
            .insert(CacheKey::new("app-a", "table-2"), json!([2]))
            .await;
        cache
            .insert(CacheKey::new("app-b", "table-1"), json!([3]))
            .await;

        let removed = cache
            .invalidate_tables("app-a", &["table-1".to_owned()])
            .await;

        assert_eq!(removed, 1);
        assert_eq!(cache.len().await, 2);
        assert!(
            cache
                .get(&CacheKey::new("app-a", "table-1"))
                .await
                .is_none()
        );
        assert!(
            cache
                .get(&CacheKey::new("app-a", "table-2"))
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn invalidation_waits_for_an_in_flight_cache_commit() {
        let cache = Arc::new(BitableCache::default());
        let transaction = cache.read_transaction().await;
        cache
            .insert(CacheKey::new("app-a", "table-1"), json!(["stale"]))
            .await;

        let invalidator = {
            let cache = cache.clone();
            tokio::spawn(async move { cache.invalidate_all().await })
        };
        tokio::task::yield_now().await;
        assert!(!invalidator.is_finished());

        drop(transaction);
        assert_eq!(invalidator.await.expect("invalidation task"), 1);
        assert!(cache.is_empty().await);
    }

    #[tokio::test]
    async fn subscribes_each_bitable_once_and_tracks_tables() {
        let cache = BitableCache::default();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        for table in ["table-1", "table-2", "table-1"] {
            let calls = calls.clone();
            cache
                .ensure_subscription("base-1", table, move || async move {
                    calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                })
                .await
                .expect("subscription");
        }
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(cache.subscription_tables("base-1").await.len(), 2);
    }
}
