use std::sync::Arc;

use anyhow::{Context, Result};
use open_lark::{Client, Config as LarkConfig};
use openlark_bitable_service::{
    AppState, cache::BitableCache, config::AppConfig, create_router, events::spawn_event_tasks,
    lark::OpenLarkReader,
};
use salvo::prelude::*;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let config = AppConfig::from_env()?;

    let client = Arc::new(
        Client::builder()
            .app_id(config.app_id.clone())
            .app_secret(config.app_key.clone())
            .base_url(config.openlark_base_url.clone())
            .build()
            .map_err(|error| anyhow::anyhow!(error.to_string()))
            .context("创建 OpenLark 客户端失败")?,
    );
    let ws_config = Arc::new(
        LarkConfig::builder()
            .app_id(config.app_id.clone())
            .app_secret(config.app_key.clone())
            .base_url(config.openlark_base_url.clone())
            .build(),
    );
    let cache = Arc::new(BitableCache::default());
    let reader = Arc::new(OpenLarkReader::new(client));
    let state = Arc::new(AppState::new(
        config.auth_token,
        config.noencrypt_auth_token,
        &config.encrypt_token,
        cache.clone(),
        reader,
    ));

    spawn_event_tasks(ws_config, cache);

    let router = create_router(state);
    let acceptor = TcpListener::new(config.bind_addr.clone()).bind().await;
    tracing::info!(bind_addr = %config.bind_addr, "OpenLark Bitable 服务已启动");
    Server::new(acceptor).serve(router).await;
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("openlark_bitable_service=info,salvo=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
