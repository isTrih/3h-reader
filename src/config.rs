use anyhow::{Context, Result, bail};

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub auth_token: String,
    pub noencrypt_auth_token: String,
    pub encrypt_token: String,
    pub app_id: String,
    pub app_key: String,
    pub bind_addr: String,
    pub openlark_base_url: String,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();

        let config = Self {
            auth_token: required_env("authToken")?,
            noencrypt_auth_token: required_env("noencrypt_authToken")?,
            encrypt_token: required_env("encryptToken")?,
            app_id: required_env("APP_ID")?,
            app_key: required_env("APP_KEY")?,
            bind_addr: std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_owned()),
            openlark_base_url: std::env::var("OPENLARK_BASE_URL")
                .unwrap_or_else(|_| "https://open.feishu.cn".to_owned()),
        };

        if config.auth_token.len() < 16 {
            bail!("authToken 至少需要 16 个字符");
        }
        if config.noencrypt_auth_token.len() < 16 {
            bail!("noencrypt_authToken 至少需要 16 个字符");
        }
        if config.auth_token == config.noencrypt_auth_token {
            bail!("authToken 与 noencrypt_authToken 不能相同");
        }
        if config.encrypt_token.len() < 8 {
            bail!("encryptToken 至少需要 8 个字符");
        }

        Ok(config)
    }
}

fn required_env(name: &str) -> Result<String> {
    let value = std::env::var(name).with_context(|| format!("缺少环境变量 {name}"))?;
    if value.trim().is_empty() {
        bail!("环境变量 {name} 不能为空");
    }
    Ok(value)
}
