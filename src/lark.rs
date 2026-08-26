use std::sync::Arc;

use async_trait::async_trait;
use open_lark::Client;

#[async_trait]
pub trait BitableReader: Send + Sync {
    async fn read_all(&self, app_token: &str, table_id: &str) -> Result<serde_json::Value, String>;

    async fn ensure_bitable_subscription(&self, _app_token: &str) -> Result<(), String> {
        Ok(())
    }
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
        serde_json::to_value(records)
            .map(flatten_records)
            .map_err(|error| error.to_string())
    }

    async fn ensure_bitable_subscription(&self, app_token: &str) -> Result<(), String> {
        use open_lark::docs::ccm::drive::v1::file::{GetSubscribeRequest, SubscribeFileRequest};

        let config = self.client.docs.config().clone();
        match GetSubscribeRequest::new(config.clone(), app_token, "bitable")
            .execute()
            .await
        {
            Ok(status) if status.is_subscribe => {}
            Ok(_) => {
                SubscribeFileRequest::new(config, app_token, "bitable")
                    .execute()
                    .await
                    .map_err(|error| error.to_string())?;
            }
            Err(error) if error.to_string().contains("1069605") => {
                SubscribeFileRequest::new(config, app_token, "bitable")
                    .execute()
                    .await
                    .map_err(|error| error.to_string())?;
            }
            Err(error) => return Err(error.to_string()),
        }
        Ok(())
    }
}

fn flatten_records(records: serde_json::Value) -> serde_json::Value {
    let serde_json::Value::Array(records) = records else {
        return records;
    };
    serde_json::Value::Array(
        records
            .into_iter()
            .map(|record| {
                let serde_json::Value::Object(record) = record else {
                    return record;
                };
                let Some(serde_json::Value::Object(fields)) = record.get("fields") else {
                    return serde_json::Value::Object(record);
                };
                serde_json::Value::Object(
                    fields
                        .iter()
                        .map(|(name, value)| (name.clone(), flatten_field_value(value)))
                        .collect(),
                )
            })
            .collect(),
    )
}

fn flatten_field_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(values)
            if values.iter().all(|item| {
                item.as_object()
                    .is_some_and(|object| object.contains_key("text"))
            }) =>
        {
            let values: Vec<_> = values
                .iter()
                .filter_map(|item| item.get("text").cloned())
                .collect();
            if values.len() == 1 {
                values.into_iter().next().unwrap()
            } else {
                serde_json::Value::Array(values)
            }
        }
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.iter().map(flatten_field_value).collect())
        }
        serde_json::Value::Object(object) if object.contains_key("text") => object
            .get("text")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        serde_json::Value::Object(object) => serde_json::Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), flatten_field_value(value)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_fields_and_text_wrappers() {
        let input = serde_json::json!([{"fields": {
            "视频ID": [{"text": "767", "type": "text"}],
            "标签": [{"text": "a", "type": "text"}, {"text": "b", "type": "text"}]
        }}]);
        assert_eq!(
            flatten_records(input),
            serde_json::json!([{"视频ID":"767", "标签":["a", "b"]}])
        );
    }
}
