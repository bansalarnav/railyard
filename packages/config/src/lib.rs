#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub app_name: &'static str,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self { app_name: "aethon" }
    }
}
