use anyhow::Result;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::process::Command;
use std::path::Path;
use crate::audio_filter;

pub fn play_radio_stream(url: &str) -> Result<()> {
    // Ferma uno stream radio già in riproduzione, se presente, cosi'
    // non restano piu' stream sovrapposti e radio_reference punta
    // sempre a un solo processo attivo.
    stop_radio();

    let child = Command::new("ffplay")
        .args(["-nodisp", "-loglevel", "panic", url])
        .spawn()?;

    audio_filter::set_radio_reference(Some(child.id().to_string()));
    Ok(())
}

pub fn search_and_play(comando: &str, stations_csv: &Path) {
    if let Ok(file) = File::open(stations_csv) {
        let reader = BufReader::new(file);
        for line in reader.lines().flatten() {
            let line = line.trim().to_string();
            // Salta righe vuote e commenti (prima riga del CSV è un commento con #)
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // Formato CSV: Nome,url,  (tre campi, terzo vuoto)
            // split_once prende solo il primo separatore → nome e "url,"
            // Usiamo splitn per gestire correttamente
            let parts: Vec<&str> = line.splitn(3, ',').collect();
            if parts.len() >= 2 {
                let nome = parts[0].trim();
                let url = parts[1].trim();
                if !nome.is_empty() && !url.is_empty()
                    && comando.to_lowercase().contains(&nome.to_lowercase())
                {
                    let _ = play_radio_stream(url);
                    if let Ok(messages) = crate::config::load_messages() {
                        let msg = messages.other_messages["radio_station_opened"]
                            .as_str()
                            .unwrap_or("[missing: other_messages.radio_station_opened]")
                            .replace("{stazione}", nome);
                        let _ = crate::tts::speak(&msg);
                    }
                    return;
                }
            }
        }
    }
    if let Ok(messages) = crate::config::load_messages() {
        let msg = messages.error_messages["radio_not_found"]
            .as_str()
            .unwrap_or("[missing: error_messages.radio_not_found]");
        let _ = crate::tts::speak(msg);
    }
}

pub fn stop_radio() {
    if let Some(pid_str) = audio_filter::get_radio_reference() {
        if let Ok(pid) = pid_str.parse::<u32>() {
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
        audio_filter::set_radio_reference(None);
        return;
    }
    // fallback se non c'è un riferimento salvato (es. radio avviata prima di questa modifica)
    let _ = Command::new("pkill").arg("ffplay").status();
}

/// Restituisce la lista delle stazioni come testo formattato
pub fn lista_stazioni(stations_csv: &Path) -> String {
    let intestazione = crate::config::load_messages()
        .ok()
        .and_then(|m| m.other_messages["radio_list_header"].as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "[missing: other_messages.radio_list_header]".to_string());
    let mut testo = format!("{}\n", intestazione);
    if let Ok(file) = File::open(stations_csv) {
        let reader = BufReader::new(file);
        for line in reader.lines().flatten() {
            let line = line.trim().to_string();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(nome) = line.splitn(3, ',').next() {
                testo.push_str(nome.trim());
                testo.push('\n');
            }
        }
    }
    testo
}
