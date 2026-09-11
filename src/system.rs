use anyhow::Result;
use regex::Regex;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::process::Command;
use walkdir::WalkDir;

// ─────────────────────────────────────────────
// VOLUME
// ─────────────────────────────────────────────

/// Esegue il comando volume e restituisce il messaggio (già formattato)
/// da inviare a TTS/UI, leggendo i template da messages_it.json.
/// Restituisce None se il comando non è riconosciuto.
pub fn set_volume(
    azione: &str,
    delta: u8,
    cmd_setvol: &[String],
    cmd_upvol: &[String],
    cmd_downvol: &[String],
    cmd_silent: &[String],
    msg_volume_set: &str,
    msg_volume_increased: &str,
    msg_volume_decreased: &str,
    msg_volume_muted: &str,
) -> Option<String> {
    let re = Regex::new(r"\d+").unwrap();
    let contiene = |parole: &[String]| parole.iter().any(|p| azione.contains(p.as_str()));

    // Unmute in ogni caso tranne quando si chiede di silenziare
    if !contiene(cmd_silent) {
        let _ = Command::new("pactl")
        .args(["set-sink-mute", "@DEFAULT_SINK@", "0"])
        .status();
    }

    if contiene(cmd_setvol) {
        if let Some(perc) = re.find(azione).map(|m| m.as_str()) {
            let _ = Command::new("pactl")
            .args(["set-sink-volume", "@DEFAULT_SINK@", &format!("{}%", perc)])
            .status();
            return Some(msg_volume_set.replace("{percent}", perc));
        }
        return None;
    }

    if contiene(cmd_upvol) {
        let _ = Command::new("pactl")
        .args(["set-sink-volume", "@DEFAULT_SINK@", &format!("+{}%", delta)])
        .status();
        return Some(msg_volume_increased.replace("{deltavolume}", &delta.to_string()));
    }

    if contiene(cmd_downvol) {
        let _ = Command::new("pactl")
        .args(["set-sink-volume", "@DEFAULT_SINK@", &format!("-{}%", delta)])
        .status();
        return Some(msg_volume_decreased.replace("{deltavolume}", &delta.to_string()));
    }

    if contiene(cmd_silent) {
        let _ = Command::new("pactl")
        .args(["set-sink-mute", "@DEFAULT_SINK@", "toggle"])
        .status();
        return Some(msg_volume_muted.to_string());
    }

    None
}

// ─────────────────────────────────────────────
// PROGRAMMI
// ─────────────────────────────────────────────

/// Cerca il programma nel comando vocale e lo avvia.
/// Restituisce Some(nome) se trovato e avviato, None altrimenti.
/// I messaggi (parlati/UI) sono gestiti dal chiamante leggendo da messages_it.json.
pub fn apri_programma(comando: &str, listaprogrammi: &std::path::Path) -> Option<String> {
    if let Ok(file) = File::open(listaprogrammi) {
        let reader = BufReader::new(file);
        for line in reader.lines().flatten() {
            let line = line.trim().to_string();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((nome, eseguibile)) = line.split_once('=') {
                let nome = nome.trim();
                let eseguibile = eseguibile.trim();
                if comando.to_lowercase().contains(&nome.to_lowercase()) {
                    // Gestisci comandi con argomenti (es. "libreoffice --writer")
                    let parts: Vec<&str> = eseguibile.split_whitespace().collect();
                    if !parts.is_empty() {
                        let mut cmd = Command::new(parts[0]);
                        if parts.len() > 1 {
                            cmd.args(&parts[1..]);
                        }
                        let _ = cmd.spawn();
                        return Some(nome.to_string());
                    }
                }
            }
        }
    }
    None
}

/// Cerca il programma nel comando vocale e lo chiude.
/// Restituisce Some(nome) se trovato e chiuso, None altrimenti.
/// I messaggi (parlati/UI) sono gestiti dal chiamante leggendo da messages_it.json.
pub fn chiudi_programma(comando: &str, listaprogrammi: &std::path::Path) -> Option<String> {
    if let Ok(file) = File::open(listaprogrammi) {
        let reader = BufReader::new(file);
        for line in reader.lines().flatten() {
            let line = line.trim().to_string();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((nome, eseguibile)) = line.split_once('=') {
                let nome = nome.trim();
                let eseguibile = eseguibile.trim();
                if comando.to_lowercase().contains(&nome.to_lowercase()) {
                    // Prendi solo il nome del processo (prima parola, senza path)
                    let process_name = eseguibile
                    .split_whitespace()
                    .next()
                    .unwrap_or(eseguibile)
                    .split('/')
                    .last()
                    .unwrap_or(eseguibile);
                    let _ = Command::new("pkill").arg(process_name).status();
                    return Some(nome.to_string());
                }
            }
        }
    }
    None
}

// ─────────────────────────────────────────────
// GESTORE FILE
// ─────────────────────────────────────────────

pub fn apri_gestore_file(path: &str) {
    #[cfg(target_os = "linux")]
    {
        // Prova i file manager più comuni in ordine
        for fm in &["dolphin", "nautilus", "thunar", "pcmanfm", "nemo", "xdg-open"] {
            if Command::new("which").arg(fm).output()
                .map(|o| o.status.success()).unwrap_or(false)
                {
                    let _ = Command::new(fm).arg(path).spawn();
                    return;
                }
        }
    }
    #[cfg(target_os = "macos")]
    { let _ = Command::new("open").arg(path).spawn(); }
    #[cfg(target_os = "windows")]
    { let _ = Command::new("explorer").arg(path).spawn(); }
}

/// Sposta (o rinomina) un file o una directory da `sorgente` a `destinazione`.
/// Se `destinazione` è una directory esistente, il file/cartella viene spostato
/// al suo interno mantenendo il nome originale (comportamento tipo `mv`).
/// Ritorna un messaggio di esito pronto per il TTS.
pub fn sposta_file(sorgente: &str, destinazione: &str) -> String {
    let src = std::path::Path::new(sorgente);
    if !src.exists() {
        return format!("Non trovo {}", sorgente);
    }

    let mut dst = std::path::PathBuf::from(destinazione);
    if dst.is_dir() {
        if let Some(nome) = src.file_name() {
            dst = dst.join(nome);
        }
    }

    if std::fs::rename(src, &dst).is_ok() {
        return format!("Spostato in {}", dst.display());
    }

    // std::fs::rename fallisce se sorgente e destinazione sono su filesystem
    // diversi (EXDEV): ricado sul comando di sistema, che gestisce il caso
    // copiando e cancellando l'originale.
    #[cfg(target_os = "windows")]
    let esito = Command::new("cmd").args(["/C", "move", "/Y"]).arg(src).arg(&dst).status();
    #[cfg(not(target_os = "windows"))]
    let esito = Command::new("mv").arg(src).arg(&dst).status();

    match esito {
        Ok(s) if s.success() => format!("Spostato in {}", dst.display()),
        _ => format!("Errore nello spostamento di {}", sorgente),
    }
}

/// Crea una directory, comprese le eventuali directory intermedie mancanti.
pub fn crea_directory(path: &str) -> String {
    match std::fs::create_dir_all(path) {
        Ok(_) => format!("Cartella {} creata", path),
        Err(e) => format!("Errore nella creazione della cartella: {}", e),
    }
}

/// Cancella un file o una directory (ricorsivamente se è una directory).
/// Nessuna conferma qui: va richiesta a monte, in intent.rs, prima di
/// chiamare questa funzione — l'esecuzione qui è immediata e definitiva.
pub fn cancella_file(path: &str) -> String {
    let p = std::path::Path::new(path);
    if !p.exists() {
        return format!("Non trovo {}", path);
    }
    let risultato = if p.is_dir() {
        std::fs::remove_dir_all(p)
    } else {
        std::fs::remove_file(p)
    };
    match risultato {
        Ok(_) => format!("{} cancellato", path),
        Err(e) => format!("Errore nella cancellazione: {}", e),
    }
}

/// Risolve un nome comune di cartella (pronunciato in italiano, es. "home",
/// "scaricati", "documenti") nel percorso reale sul filesystem. "home" è
/// gestita a parte perché è $HOME stesso, non una xdg-user-dir. Le altre
/// passano da `xdg-user-dir`, che restituisce il percorso rispettando la
/// lingua/localizzazione dell'utente (es. "Scaricati" invece di "Downloads"
/// su un sistema in italiano) — non hardcodiamo i nomi delle cartelle.
/// Ritorna None se il nome non è riconosciuto o se `xdg-user-dir` non è
/// disponibile/fallisce: in quel caso il chiamante ricade sul percorso
/// assoluto letto per intero.
/// Restituisce la home dell'utente in modo cross-platform: $HOME su Linux/
/// macOS, %USERPROFILE% su Windows (con fallback a %HOMEDRIVE%+%HOMEPATH%
/// per i rari casi in cui USERPROFILE non sia impostata).
fn home_dir() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if let Ok(p) = std::env::var("USERPROFILE") {
            return Some(std::path::PathBuf::from(p));
        }
        let drive = std::env::var("HOMEDRIVE").ok()?;
        let path = std::env::var("HOMEPATH").ok()?;
        Some(std::path::PathBuf::from(format!("{}{}", drive, path)))
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME").ok().map(std::path::PathBuf::from)
    }
}

/// Risolve un nome comune di cartella (pronunciato in italiano, es. "home",
/// "scaricati", "documenti") nel percorso reale sul filesystem, in modo
/// cross-platform:
/// - Linux: prova prima `xdg-user-dir`, che rispetta lingua/localizzazione
///   (es. "Scaricati" invece di "Downloads" su un sistema in italiano).
/// - macOS/Windows (o Linux senza xdg-user-dir): usa il nome inglese
///   standard della cartella come sottocartella di home ("Movies" invece di
///   "Videos" su macOS; "Public" risolta via %PUBLIC% su Windows, dove non
///   è dentro la home dell'utente).
/// - In ogni caso, se il nome non è tra quelli standard riconosciuti, cerca
///   comunque una sottocartella diretta di home con lo stesso nome
///   (case-insensitive) — copre cartelle personalizzate come un link a un
///   disco esterno montato dentro home (es. "multimedia").
pub fn risolvi_cartella_comune(nome: &str) -> Option<std::path::PathBuf> {
    let home = home_dir()?;
    if nome == "home" {
        return Some(home);
    }

    let nome_standard: Option<&str> = match nome {
        "scaricati" | "download" | "downloads" => Some("Downloads"),
        "documenti" | "document" | "documents" => Some("Documents"),
        "immagini" | "foto" | "pictures" => Some("Pictures"),
        "video" | "film" | "videos" => {
            if cfg!(target_os = "macos") { Some("Movies") } else { Some("Videos") }
        }
        "musica" | "music" => Some("Music"),
        "desktop" | "scrivania" => Some("Desktop"),
        "pubblici" | "condivisi" | "public" => Some("Public"),
        "modelli" | "templates" => Some("Templates"),
        _ => None,
    };

    if let Some(nome_std) = nome_standard {
        #[cfg(target_os = "linux")]
        {
            let chiave_xdg = match nome_std {
                "Downloads" => Some("DOWNLOAD"),
                "Documents" => Some("DOCUMENTS"),
                "Pictures"  => Some("PICTURES"),
                "Videos"    => Some("VIDEOS"),
                "Music"     => Some("MUSIC"),
                "Desktop"   => Some("DESKTOP"),
                "Public"    => Some("PUBLICSHARE"),
                "Templates" => Some("TEMPLATES"),
                _ => None,
            };
            if let Some(chiave) = chiave_xdg {
                if let Ok(output) = Command::new("xdg-user-dir").arg(chiave).output() {
                    if output.status.success() {
                        if let Ok(percorso) = String::from_utf8(output.stdout) {
                            let percorso = percorso.trim();
                            if !percorso.is_empty() {
                                return Some(std::path::PathBuf::from(percorso));
                            }
                        }
                    }
                }
            }
        }

        // Su Windows la cartella "Public" è un profilo a sé (C:\Users\Public),
        // non una sottocartella della home dell'utente.
        #[cfg(target_os = "windows")]
        if nome_std == "Public" {
            if let Ok(p) = std::env::var("PUBLIC") {
                return Some(std::path::PathBuf::from(p));
            }
        }

        // macOS/Windows, o Linux senza xdg-user-dir: sottocartella standard
        // in inglese dentro home, se esiste davvero.
        let candidato = home.join(nome_std);
        if candidato.is_dir() {
            return Some(candidato);
        }
    }

    // Fallback generico per nomi non standard.
    for entry in std::fs::read_dir(&home).ok()?.flatten() {
        if entry.file_name().to_string_lossy().eq_ignore_ascii_case(nome) {
            let path = entry.path();
            if std::fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(false) {
                return Some(path);
            }
        }
    }
    None
}

// ─────────────────────────────────────────────
// AGGIORNAMENTO SISTEMA
// ─────────────────────────────────────────────

/// Esegue l'aggiornamento di sistema in background.
/// `msg_completed` e `tx_output` permettono di notificare la UI
/// e il TTS al termine, leggendo il testo da messages_it.json.
pub fn aggiorna_sistema(
    msg_completed: String,
    completo: bool) {
    std::thread::spawn(move || {
        let which = |cmd: &str| Command::new("which").arg(cmd)
            .output().map(|o| o.status.success()).unwrap_or(false);

        if which("apt") {
            if completo {
                    let _ = Command::new("sudo").args(["-n", "/usr/bin/apt", "dist-upgrade", "-y"]).status();
            } else {
                    let _ = Command::new("sudo").args(["-n", "/usr/bin/pkcon", "update", "-y"]).status();
            }

        } else if which("pacman") {
            let _ = Command::new("sudo").args(["-n", "pacman", "-Syu", "--noconfirm"]).status();
        } else if which("dnf") {
            let _ = Command::new("sudo").args(["-n", "dnf", "upgrade", "-y"]).status();
        } else if which("zypper") {
            let _ = Command::new("sudo").args(["-n", "zypper", "update", "-y"]).status();
        }


        let _ = crate::tts::speak(&msg_completed);
    });
}

// ─────────────────────────────────────────────
// BOOKMARKS: multi-browser
// ─────────────────────────────────────────────

pub fn generate_bookmarks_list(output_path: &str) -> Result<()> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut file = File::create(output_path)?;

    // Prova i browser in ordine di priorità
    let browser_paths = [
        format!("{}/.config/vivaldi/Default/Bookmarks", home),
            format!("{}/.config/google-chrome/Default/Bookmarks", home),
                format!("{}/.config/chromium/Default/Bookmarks", home),
                    format!("{}/.config/microsoft-edge/Default/Bookmarks", home),
    ];

    for path in &browser_paths {
        if std::path::Path::new(path).exists() {
            let data = std::fs::read_to_string(path)?;
            let json: serde_json::Value = serde_json::from_str(&data)?;
            if let Some(roots) = json["roots"].as_object() {
                for (_, folder) in roots {
                    extract_bookmarks(folder, &mut file);
                }
            }
            return Ok(());
        }
    }

    // Fallback: Firefox (SQLite)
    let firefox_dir = format!("{}/.mozilla/firefox", home);
    if std::path::Path::new(&firefox_dir).exists() {
        extract_firefox_bookmarks(&firefox_dir, &mut file)?;
    }

    Ok(())
}

fn extract_bookmarks(node: &serde_json::Value, file: &mut File) {
    if let Some(obj) = node.as_object() {
        if obj.get("type").and_then(|t| t.as_str()) == Some("url") {
            let name = obj.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let url = obj.get("url").and_then(|u| u.as_str()).unwrap_or("");
            if !name.is_empty() && !url.is_empty() {
                let _ = writeln!(file, "{}={}", name, url);
            }
        }
        if let Some(children) = obj.get("children").and_then(|c| c.as_array()) {
            for child in children {
                extract_bookmarks(child, file);
            }
        }
    }
}

fn extract_firefox_bookmarks(profiles_dir: &str, file: &mut File) -> Result<()> {
    use rusqlite::Connection;

    for entry in std::fs::read_dir(profiles_dir)?.flatten() {
        let places = entry.path().join("places.sqlite");
        if !places.exists() {
            continue;
        }
        // Copia il DB per evitare lock di Firefox
        let tmp = std::env::temp_dir().join("places_tmp_marco.sqlite");
        std::fs::copy(&places, &tmp)?;
        if let Ok(conn) = Connection::open(&tmp) {
            let query = "SELECT b.title, p.url FROM moz_bookmarks b \
JOIN moz_places p ON b.fk = p.id \
WHERE b.title IS NOT NULL AND p.url IS NOT NULL";
            if let Ok(mut stmt) = conn.prepare(query) {
                let _ = stmt.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map(|rows| {
                    for row in rows.flatten() {
                        let _ = writeln!(file, "{}={}", row.0, row.1);
                    }
                });
            }
        }
        let _ = std::fs::remove_file(&tmp);
        break;
    }
    Ok(())
}

// ─────────────────────────────────────────────
// LISTA PROGRAMMI
// ─────────────────────────────────────────────

pub fn generate_programs_list(output_path: &str) -> Result<()> {
    let mut entries = Vec::new();
    let home = std::env::var("HOME").unwrap_or_default();
    let search_dirs = [
        "/usr/share/applications".to_string(),
        format!("{}/.local/share/applications", home),
    ];
    for dir in &search_dirs {
        if !std::path::Path::new(dir).exists() {
            continue;
        }
        for entry in WalkDir::new(dir).max_depth(2).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("desktop") {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(path) {
                let mut name = None;
                let mut exec = None;
                for line in content.lines() {
                    if line.starts_with("Name=") && name.is_none() {
                        name = line.strip_prefix("Name=").map(|s| s.to_string());
                    } else if line.starts_with("Exec=") && exec.is_none() {
                        // Rimuovi placeholder %F %u %U ecc.
                        let raw = line.strip_prefix("Exec=").unwrap_or("");
                        let clean = raw.split('%').next().unwrap_or(raw).trim();
                        if !clean.is_empty() {
                            exec = Some(clean.to_string());
                        }
                    }
                    if name.is_some() && exec.is_some() {
                        break;
                    }
                }
                if let (Some(n), Some(e)) = (name, exec) {
                    entries.push(format!("{}={}", n, e));
                }
            }
        }
    }
    entries.sort();
    entries.dedup();
    let mut file = File::create(output_path)?;
    for entry in entries {
        writeln!(file, "{}", entry)?;
    }
    Ok(())
}

// ─────────────────────────────────────────────
// SISTEMA
// ─────────────────────────────────────────────

pub fn shutdown() {
    #[cfg(target_os = "linux")]
    { let _ = Command::new("shutdown").args(["-h", "now"]).status(); }
    #[cfg(target_os = "macos")]
    { let _ = Command::new("sudo").args(["shutdown", "-h", "now"]).status(); }
    #[cfg(target_os = "windows")]
    { let _ = Command::new("shutdown").args(["/s", "/f", "/t", "0"]).status(); }
}

pub fn reboot() {
    #[cfg(target_os = "linux")]
    { let _ = Command::new("reboot").status(); }
    #[cfg(target_os = "macos")]
    { let _ = Command::new("sudo").args(["shutdown", "-r", "now"]).status(); }
    #[cfg(target_os = "windows")]
    { let _ = Command::new("shutdown").args(["/r", "/f", "/t", "0"]).status(); }
}

#[allow(dead_code)]
pub fn kill_process(process_name: &str) {
    let _ = Command::new("pkill").arg(process_name).status();
}
