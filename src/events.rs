use std::{collections::HashSet, sync::Arc, time::Duration};

use open_lark::Config;
use open_lark::ws_client::{EventDispatcherHandler, EventHandler, LarkWsClient, WsClientError};
use serde::Deserialize;
use serde_json::Value;
use tokio::{sync::mpsc, time::timeout};

use crate::cache::BitableCache;

pub const BITABLE_RECORD_CHANGED: &str = "drive.file.bitable_record_changed_v1";
const QUIET_PERIOD: Duration = Duration::from_secs(120);

#[derive(Clone)]
struct ChangedEventSender {
    sender: mpsc::UnboundedSender<Vec<u8>>,
}

impl EventHandler for ChangedEventSender {
    fn handle(&self, payload: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.sender.send(payload.to_vec())?;
        Ok(())
    }
}

#[derive(Debug, Default)]
struct InvalidationBatch {
    clear_all: bool,
    bitables: HashSet<String>,
    tables: HashSet<(String, String)>,
}

impl InvalidationBatch {
    fn add_payload(&mut self, payload: &[u8]) {
        let Ok(envelope) = serde_json::from_slice::<EventEnvelope>(payload) else {
            self.clear_all = true;
            return;
        };

        let mut app_tokens = HashSet::new();
        let mut table_ids = HashSet::new();
        collect_identifiers(&envelope.event, &mut app_tokens, &mut table_ids);

        if app_tokens.is_empty() {
            self.clear_all = true;
        } else if table_ids.is_empty() {
            self.bitables.extend(app_tokens);
        } else {
            for app_token in app_tokens {
                for table_id in &table_ids {
                    self.tables.insert((app_token.clone(), table_id.clone()));
                }
            }
        }
    }

    async fn flush(self, cache: &BitableCache) {
        let removed = if self.clear_all {
            cache.invalidate_all().await
        } else {
            let mut removed = 0;
            for app_token in self.bitables {
                removed += cache.invalidate_bitable(&app_token).await;
            }
            for (app_token, table_id) in self.tables {
                removed += cache.invalidate_tables(&app_token, &[table_id]).await;
            }
            removed
        };
        tracing::info!(removed, "变更事件静默 120 秒，已清除相关缓存");
    }
}

#[derive(Deserialize)]
struct EventEnvelope {
    event: Value,
}

fn collect_identifiers(
    value: &Value,
    app_tokens: &mut HashSet<String>,
    table_ids: &mut HashSet<String>,
) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if let Some(value) = value.as_str() {
                    match key.as_str() {
                        "app_token" | "file_token" => {
                            app_tokens.insert(value.to_owned());
                        }
                        "table_id" => {
                            table_ids.insert(value.to_owned());
                        }
                        _ => {}
                    }
                }
                collect_identifiers(value, app_tokens, table_ids);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_identifiers(item, app_tokens, table_ids);
            }
        }
        _ => {}
    }
}

pub fn spawn_event_tasks(config: Arc<Config>, cache: Arc<BitableCache>) {
    let (sender, receiver) = mpsc::unbounded_channel();
    tokio::spawn(run_debounced_invalidation(receiver, cache));
    tokio::spawn(run_websocket(config, sender));
}

async fn run_websocket(config: Arc<Config>, sender: mpsc::UnboundedSender<Vec<u8>>) {
    let mut retry_delay = Duration::from_secs(2);
    loop {
        let dispatcher = match EventDispatcherHandler::builder().register_raw(
            BITABLE_RECORD_CHANGED,
            ChangedEventSender {
                sender: sender.clone(),
            },
        ) {
            Ok(dispatcher) => dispatcher.build(),
            Err(error) => {
                tracing::error!(%error, "注册飞书多维表格变更事件失败");
                return;
            }
        };

        tracing::info!(event = BITABLE_RECORD_CHANGED, "正在连接飞书 WebSocket");
        match LarkWsClient::open(config.clone(), dispatcher).await {
            Ok(()) => tracing::warn!("飞书 WebSocket 会话已结束"),
            Err(WsClientError::ConnectionClosed { reason }) => {
                tracing::warn!(?reason, "飞书 WebSocket 已断开");
            }
            Err(error) => tracing::error!(%error, "飞书 WebSocket 发生错误"),
        }

        tokio::time::sleep(retry_delay).await;
        retry_delay = (retry_delay * 2).min(Duration::from_secs(60));
    }
}

async fn run_debounced_invalidation(
    mut receiver: mpsc::UnboundedReceiver<Vec<u8>>,
    cache: Arc<BitableCache>,
) {
    while let Some(payload) = receiver.recv().await {
        let mut batch = InvalidationBatch::default();
        batch.add_payload(&payload);

        loop {
            match timeout(QUIET_PERIOD, receiver.recv()).await {
                Ok(Some(payload)) => batch.add_payload(&payload),
                Ok(None) => {
                    batch.flush(&cache).await;
                    return;
                }
                Err(_) => {
                    batch.flush(&cache).await;
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::cache::CacheKey;

    #[tokio::test]
    async fn extracts_nested_file_and_table_ids() {
        let cache = BitableCache::default();
        cache
            .insert(CacheKey::new("base-1", "tbl-1"), json!([1]))
            .await;
        cache
            .insert(CacheKey::new("base-1", "tbl-2"), json!([2]))
            .await;

        let payload = serde_json::to_vec(&json!({
            "event": {
                "file_token": "base-1",
                "action_list": [{"table_id": "tbl-1", "record_id": "rec-1"}]
            }
        }))
        .expect("serialize");
        let mut batch = InvalidationBatch::default();
        batch.add_payload(&payload);
        batch.flush(&cache).await;

        assert!(cache.get(&CacheKey::new("base-1", "tbl-1")).await.is_none());
        assert!(cache.get(&CacheKey::new("base-1", "tbl-2")).await.is_some());
    }

    #[tokio::test]
    async fn malformed_event_falls_back_to_clearing_all() {
        let cache = BitableCache::default();
        cache
            .insert(CacheKey::new("base-1", "tbl-1"), json!([1]))
            .await;
        let mut batch = InvalidationBatch::default();
        batch.add_payload(b"not json");
        batch.flush(&cache).await;
        assert!(cache.is_empty().await);
    }
}
