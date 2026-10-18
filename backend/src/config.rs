use std::env;
#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub origin: String,
    pub bind: String,
    pub secure: bool,
    pub setup_key: String,
    pub livekit_url: String,
    pub livekit_internal: String,
    pub livekit_key: String,
    pub livekit_secret: String,
    pub static_dir: String,
}
impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let get = |key: &str, default: &str| env::var(key).unwrap_or_else(|_| default.into());
        let secure = get("APP_ENV", "production") != "development";
        let c = Self {
            database_url: get("DATABASE_URL", "sqlite://data/subdisc.db"),
            origin: get("APP_ORIGIN", "http://localhost:8080"),
            bind: get("BIND", "0.0.0.0:3000"),
            secure,
            setup_key: get("SETUP_KEY", ""),
            livekit_url: get("LIVEKIT_URL", "ws://localhost:7880"),
            livekit_internal: get("LIVEKIT_INTERNAL_URL", "http://127.0.0.1:7880"),
            livekit_key: get("LIVEKIT_API_KEY", "devkey"),
            livekit_secret: get("LIVEKIT_API_SECRET", "secret"),
            static_dir: get("STATIC_DIR", "frontend/dist"),
        };
        anyhow::ensure!(
            c.setup_key.len() >= 16,
            "SETUP_KEY precisa ter pelo menos 16 caracteres"
        );
        anyhow::ensure!(
            !c.origin.ends_with('/'),
            "APP_ORIGIN não deve terminar com barra"
        );
        if secure {
            anyhow::ensure!(
                c.origin.starts_with("https://") && c.livekit_url.starts_with("wss://"),
                "Produção exige HTTPS e WSS"
            );
            anyhow::ensure!(
                c.livekit_secret.len() >= 32
                    && c.livekit_key != "devkey"
                    && !c.setup_key.starts_with("development"),
                "Configure segredos próprios de produção"
            );
        }
        Ok(c)
    }
}
