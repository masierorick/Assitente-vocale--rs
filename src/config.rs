use std::fs;
use std::path::PathBuf;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use dirs;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub botname: String,
    pub wakeword: String,
    pub sleep_time: u64,
    pub deltavolume: u8,
    pub layout: String,
    pub musicplayer: String,
    pub browser: String,
    #[serde(default)]
    pub whisper_model: Option<String>,
    #[serde(default)]
    pub piper_bin: Option<String>,
    #[serde(default)]
    pub piper_model: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Messages {
    pub welcome_messages: Vec<String>,
    pub goodbye_messages: Vec<String>,
    pub error_messages: serde_json::Value,
    pub other_messages: serde_json::Value,
    pub commands: serde_json::Value,
    pub objects: serde_json::Value,
}

/// Restituisce il percorso del file di configurazione seguendo questa priorità:
/// 1. ~/.config/assistente-rs/<file>
/// 2. /usr/share/assistente/config/<file>
/// 3. ./config/<file> (solo come fallback per sviluppo)
pub fn config_path(file: &str) -> PathBuf {
    // 1. Percorso utente (prioritario)
    if let Some(home) = dirs::home_dir() {
        let user_path = home.join(".config").join("assistente").join(file);
        if user_path.exists() {
            return user_path;
        }
    }

    // 2. Percorso di sistema (predefinito)
    let system_path = PathBuf::from("/usr/share/assistente/config").join(file);
    if system_path.exists() {
        return system_path;
    }

    // 3. Fallback locale (per sviluppo)
    PathBuf::from("config").join(file)
}

// NUOVA FUNZIONE per il file .env (stessa logica)
pub fn env_path() -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        let user_path = home.join(".config").join("assistente-rs").join(".env");
        if user_path.exists() {
            return user_path;
        }
    }
    let system_path = PathBuf::from("/usr/share/assistente/config").join(".env");
    if system_path.exists() {
        return system_path;
    }
    PathBuf::from("config").join(".env")
}


pub fn load_config() -> Result<Config> {
    let path = config_path("config.json");
    let data = fs::read_to_string(path)?;
    let mut cfg: Config = serde_json::from_str(&data)?;

    cfg.whisper_model = cfg.whisper_model.map(|p| expand_tilde(&p));
    cfg.piper_bin = cfg.piper_bin.map(|p| expand_tilde(&p));
    cfg.piper_model = cfg.piper_model.map(|p| expand_tilde(&p));

    Ok(cfg)
}

/// Espande un path che inizia con "~" nella home dell'utente corrente,
/// cosi' config.json puo' restare identico su macchine/utenti diversi
/// invece di avere un path assoluto hardcoded.
fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    }
    path.to_string()
}

pub fn load_messages() -> Result<Messages> {
    let path = config_path("messages_it.json");
    let data = fs::read_to_string(path)?;
    Ok(serde_json::from_str::<Messages>(&data)?)
}
