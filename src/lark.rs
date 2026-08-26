use std::sync::Arc;

use async_trait::async_trait;
use open_lark::Client;

#[async_trait]
pub trait BitableReader: Send + Sync {
    async fn read_all(&self, app_token: &str, table_id: &str) -> Result<serde_json::Value, String>;
}

pub struct OpenLarkReader {
    client: Arc<Client>,
}

impl OpenLarkReader {
    pub fn new(client: Arc<Client>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl BitableReader for OpenLarkReader {
    async fn read_all(&self, app_token: &str, table_id: &str) -> Result<serde_json::Value, String> {
        let records = self
            .client
            .docs
            .search_bitable_records_all(app_token, table_id)
            .await
            .map_err(|error| error.to_string())?;
        serde_json::to_value(records).map_err(|error| error.to_string())
    }
}
