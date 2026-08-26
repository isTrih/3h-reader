pub mod cache;
pub mod config;
pub mod crypto;
pub mod error;
pub mod events;
pub mod lark;

use std::sync::Arc;

use salvo::{catch_panic::CatchPanic, cors::Cors, http::Method, prelude::*};
use serde::Serialize;
use serde_json::json;
use subtle::ConstantTimeEq;

use cache::{BitableCache, CacheKey};
use crypto::Encryptor;
use error::ApiError;
use lark::BitableReader;

pub struct AppState {
    pub auth_token: String,
    pub encryptor: Encryptor,
    pub cache: Arc<BitableCache>,
    pub reader: Arc<dyn BitableReader>,
}

impl AppState {
    pub fn new(
        auth_token: String,
        encrypt_token: &str,
        cache: Arc<BitableCache>,
        reader: Arc<dyn BitableReader>,
    ) -> Self {
        Self {
            auth_token,
            encryptor: Encryptor::new(encrypt_token),
            cache,
            reader,
        }
    }
}

#[derive(Serialize)]
struct EncryptedResponse {
    data: String,
}

#[handler]
async fn require_auth(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
    ctrl: &mut FlowCtrl,
) {
    // 浏览器的 CORS 预检不携带业务认证信息。
    if req.method() == Method::OPTIONS {
        return;
    }

    let Ok(state) = depot.get_typed::<Arc<AppState>>() else {
        render_error(res, StatusCode::INTERNAL_SERVER_ERROR, "服务状态不可用");
        ctrl.skip_rest();
        return;
    };

    let supplied = req
        .header::<String>("authorization")
        .and_then(|value| {
            value
                .strip_prefix("Bearer ")
                .or_else(|| value.strip_prefix("bearer "))
                .map(str::to_owned)
                .or(Some(value))
        })
        .or_else(|| req.header::<String>("auth"));

    let authorized = supplied
        .as_deref()
        .map(|value| constant_time_equal(value.as_bytes(), state.auth_token.as_bytes()))
        .unwrap_or(false);

    if !authorized {
        render_error(res, StatusCode::UNAUTHORIZED, "Unauthorized");
        ctrl.skip_rest();
    }
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len() && bool::from(left.ct_eq(right))
}

fn render_error(res: &mut Response, status: StatusCode, message: &str) {
    res.status_code(status);
    res.render(Json(json!({"error": message, "code": status.as_u16()})));
}

#[handler]
async fn health() -> Json<serde_json::Value> {
    Json(json!({"status": "ok"}))
}

#[handler]
async fn preflight() -> StatusCode {
    StatusCode::OK
}

#[handler]
async fn read_bitable(
    req: &mut Request,
    depot: &mut Depot,
) -> Result<Json<EncryptedResponse>, ApiError> {
    let app_token = req
        .param::<String>("app_token")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ApiError::BadRequest("缺少 bitable token".to_owned()))?;
    let table_id = req
        .query::<String>("table")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ApiError::BadRequest("缺少查询参数 table".to_owned()))?;
    let state = depot
        .get_typed::<Arc<AppState>>()
        .map_err(|_| ApiError::internal("服务状态不可用"))?;
    let key = CacheKey::new(app_token, table_id);

    let records = if let Some(value) = state.cache.get(&key).await {
        value
    } else {
        let key_lock = state.cache.key_lock(&key).await;
        let _guard = key_lock.lock().await;
        let _transaction = state.cache.read_transaction().await;
        if let Some(value) = state.cache.get(&key).await {
            value
        } else {
            let value = state
                .reader
                .read_all(&key.app_token, &key.table_id)
                .await
                .map_err(ApiError::Upstream)?;
            state.cache.insert(key, value).await
        }
    };

    let data = state.encryptor.encrypt_json(records.as_ref())?;
    Ok(Json(EncryptedResponse { data }))
}

pub fn create_router(state: Arc<AppState>) -> Router {
    // 任意域名、方法和请求头均可跨域；认证仍由受保护路由的中间件执行。
    let cors = Cors::permissive().into_handler();

    Router::new()
        .hoop(CatchPanic::new())
        .hoop(cors)
        .hoop(affix_state::inject(state))
        .push(Router::with_path("health").get(health))
        .push(
            Router::with_path("bitable/{app_token}")
                .hoop(require_auth)
                .options(preflight)
                .get(read_bitable),
        )
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use salvo::test::{ResponseExt, TestClient};

    use super::*;

    struct FakeReader {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl BitableReader for FakeReader {
        async fn read_all(
            &self,
            app_token: &str,
            table_id: &str,
        ) -> Result<serde_json::Value, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(json!([{"app_token": app_token, "table_id": table_id}]))
        }
    }

    fn test_service() -> Service {
        let reader = Arc::new(FakeReader {
            calls: AtomicUsize::new(0),
        });
        let state = Arc::new(AppState::new(
            "0123456789abcdef".to_owned(),
            "encrypt-secret",
            Arc::new(BitableCache::default()),
            reader,
        ));
        Service::new(create_router(state))
    }

    #[tokio::test]
    async fn rejects_missing_auth_header() {
        let response = TestClient::get("http://localhost/bitable/base-1?table=tbl-1")
            .send(&test_service())
            .await;
        assert_eq!(response.status_code, Some(StatusCode::UNAUTHORIZED));
    }

    #[tokio::test]
    async fn accepts_bearer_token_and_returns_encrypted_data() {
        let mut response = TestClient::get("http://localhost/bitable/base-1?table=tbl-1")
            .bearer_auth("0123456789abcdef")
            .send(&test_service())
            .await;
        assert_eq!(response.status_code, Some(StatusCode::OK));
        let body = response
            .take_json::<serde_json::Value>()
            .await
            .expect("json response");
        assert!(body["data"].as_str().is_some_and(|value| !value.is_empty()));
    }

    #[tokio::test]
    async fn requires_table_query_parameter() {
        let response = TestClient::get("http://localhost/bitable/base-1")
            .add_header("auth", "0123456789abcdef", true)
            .send(&test_service())
            .await;
        assert_eq!(response.status_code, Some(StatusCode::BAD_REQUEST));
    }

    #[tokio::test]
    async fn allows_cross_origin_preflight() {
        let response = TestClient::options("http://localhost/bitable/base-1?table=tbl-1")
            .add_header("origin", "https://any.example", true)
            .add_header("access-control-request-method", "GET", true)
            .send(&test_service())
            .await;
        assert_eq!(response.status_code, Some(StatusCode::NO_CONTENT));
        assert_eq!(
            response
                .headers()
                .get("access-control-allow-origin")
                .and_then(|v| v.to_str().ok()),
            Some("*")
        );
    }
}
