use std::env;
#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub origin: String,
    pub bind: String,
    pub secure: bool,
    pub setup_key: String,
    pub media_ice_servers: serde_json::Value,
    pub media_public_ip: String,
    pub media_udp_min: u16,
    pub media_udp_max: u16,
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
            media_ice_servers: serde_json::from_str(&get("MEDIA_ICE_SERVERS", "[]"))?,
            media_public_ip: get("MEDIA_PUBLIC_IP", ""),
            media_udp_min: get("MEDIA_UDP_MIN", "50000").parse()?,
            media_udp_max: get("MEDIA_UDP_MAX", "51999").parse()?,
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
        anyhow::ensure!(
            c.media_udp_min > 0 && c.media_udp_max >= c.media_udp_min,
            "Faixa UDP inválida"
        );
        let _: Vec<webrtc::ice_transport::ice_server::RTCIceServer> =
            serde_json::from_value(c.media_ice_servers.clone())?;
        if !c.media_public_ip.is_empty() {
            let _: std::net::IpAddr = c.media_public_ip.parse()?;
        }
        if secure {
            anyhow::ensure!(
                c.origin.starts_with("https://"),
                "Produção exige HTTPS e WSS"
            );
            anyhow::ensure!(
                !c.setup_key.starts_with("development"),
                "Configure segredos próprios de produção"
            );
        }
        Ok(c)
    }
}
