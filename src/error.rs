use async_trait::async_trait;
use salvo::prelude::*;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("飞书 API 请求失败")]
    Upstream(String),
    #[error("{0}")]
    Internal(String),
}

impl ApiError {
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }
}

#[async_trait]
impl Writer for ApiError {
    async fn write(self, _req: &mut Request, _depot: &mut Depot, res: &mut Response) {
        let (status, public_message) = match &self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message.as_str()),
            Self::Upstream(detail) => {
                tracing::error!(error = %detail, "飞书 API 请求失败");
                (StatusCode::BAD_GATEWAY, "飞书 API 请求失败")
            }
            Self::Internal(detail) => {
                tracing::error!(error = %detail, "内部服务错误");
                (StatusCode::INTERNAL_SERVER_ERROR, "内部服务错误")
            }
        };
        res.status_code(status);
        res.render(Json(json!({
            "error": public_message,
            "code": status.as_u16()
        })));
    }
}
