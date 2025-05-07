use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct TemplateSettings {
    pub dir: String,
    pub debug: bool,
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub debug: bool,
    pub host: String,
    pub port: u16,
    pub ws_port: u16,
    pub template: TemplateSettings,
    pub other: HashMap<String, String>, // Manteniamo eventuali future impostazioni
}
