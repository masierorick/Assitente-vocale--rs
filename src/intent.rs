use regex::Regex;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use strsim::jaro_winkler;
use crate::{tts, system, radio};

/// Vero se `comando` nomina il bookmark `nome`: o perché contiene l'intero
/// nome (bookmark con titolo breve, es. "Youtube"), oppure perché contiene
/// almeno una parola chiave significativa estratta dal nome (utile per
/// titoli descrittivi lunghi)
fn bookmark_corrisponde(comando: &str, nome: &str, stopword: &[String]) -> bool {
    let comando_lower = comando.to_lowercase();
    let nome_lower = nome.to_lowercase();

    if comando_lower.contains(&nome_lower) {
        return true;
    }

    // Parole intere del comando (non sottostringhe): evita che una parola
    // chiave come "mail" matchi dentro "gmail".
    let parole_comando: Vec<&str> = comando_lower
    .split(|c: char| !c.is_alphanumeric())
    .filter(|w| !w.is_empty())
    .collect();

    nome_lower
    .split(|c: char| c.is_whitespace() || c == ':' || c == '-' || c == '\u{2014}')
    .map(|t| t.trim())
    .filter(|t| t.len() >= 3 && !stopword.iter().any(|s| s == t))
    .any(|token| parole_comando.contains(&token))
}

/// Apre `url` in una finestra dedicata del browser (non una nuova tab in
/// una finestra esistente), passando il flag giusto in base al browser
/// configurato. Nessuna modifica permanente al browser: il flag vale solo
/// per questo lancio. Se il lancio diretto fallisce, ricade su webbrowser::open
/// (tab in finestra esistente).
fn apri_finestra_dedicata(browser: &str, url: &str) {
    let flag = if browser.to_lowercase().contains("firefox") {
        "-new-window"
    } else {
        "--new-window"
    };
    let lanciato = std::process::Command::new(browser)
    .arg(flag).arg(url)
    .spawn()
    .is_ok();
    if !lanciato {
        let _ = webbrowser::open(url);
    }
}

/// Come `apri_finestra_dedicata`, ma su Linux prova anche a tracciare l'ID
/// esatto della nuova finestra (via kdotool), confrontando l'elenco delle
/// finestre prima e dopo l'apertura. Ritorna Some(id) se ci riesce, altrimenti
/// None (la chiusura ricadr\u00e0 sul matching per titolo, meno preciso).
fn apri_finestra_dedicata_tracciata(browser: &str, url: &str) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let prima = elenco_finestre_kdotool();
        apri_finestra_dedicata(browser, url);
        for _ in 0..15 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let dopo = elenco_finestre_kdotool();
            if let Some(nuova) = dopo.iter().find(|id| !prima.contains(id)) {
                return Some(nuova.clone());
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        apri_finestra_dedicata(browser, url);
        None
    }
}

/// Elenca gli ID di tutte le finestre visibili conosciute da kdotool.
#[cfg(target_os = "linux")]
fn elenco_finestre_kdotool() -> Vec<String> {
    std::process::Command::new("kdotool")
    .arg("search").arg("--name").arg(".")
    .output()
    .ok()
    .map(|o| String::from_utf8_lossy(&o.stdout)
    .lines()
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
    .collect())
    .unwrap_or_default()
}

/// Chiude una finestra per ID esatto (nessuna ambiguit\u00e0 di titolo).
fn chiudi_finestra_id(id: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("kdotool")
        .arg("windowclose").arg(id)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = id;
        false
    }
}

/// Termina (cross-platform) tutti i processi il cui nome o riga di comando
/// contiene `nome` (case-insensitive). Sostituisce `pkill`, che esiste solo
/// su Unix e non funziona su Windows.
fn termina_processo(nome: &str) {
    use sysinfo::{ProcessesToUpdate, System};
    let nome_lower = nome.to_lowercase();
    let mut sys = System::new_all();
    sys.refresh_processes(ProcessesToUpdate::All);
    for process in sys.processes().values() {
        let pname = process.name().to_string_lossy().to_lowercase();
        let in_cmd = process.cmd().iter()
        .any(|a| a.to_string_lossy().to_lowercase().contains(&nome_lower));
        if pname.contains(&nome_lower) || in_cmd {
            process.kill();
        }
    }
}

/// Chiude, senza toccare nessuna configurazione del browser, tutte le finestre
/// il cui titolo contiene `titolo` (case-insensitive). Funziona con qualsiasi
/// browser perché agisce a livello di finestra del sistema operativo, non di
/// singola tab. Ritorna true se almeno un tentativo di chiusura è stato fatto
/// (non garantisce che sia effettivamente andato a buon fine).
fn chiudi_finestra_titolo(titolo: &str) -> bool {
    let titolo_lower = titolo.to_lowercase();

    #[cfg(target_os = "windows")]
    {
        chiudi_finestra_windows(&titolo_lower)
    }

    #[cfg(target_os = "macos")]
    {
        chiudi_finestra_macos(&titolo_lower)
    }

    #[cfg(target_os = "linux")]
    {
        chiudi_finestra_linux(&titolo_lower)
    }
}

/// Windows: enumera le finestre di primo livello e invia WM_CLOSE a quelle
/// il cui titolo contiene `titolo`. Nessun tool esterno richiesto.
#[cfg(target_os = "windows")]
fn chiudi_finestra_windows(titolo: &str) -> bool {
    use windows::Win32::Foundation::{BOOL, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextLengthW, GetWindowTextW, IsWindowVisible,
        PostMessageW, WM_CLOSE,
    };

    struct Ctx<'a> { titolo: &'a str, trovata: bool }

    unsafe extern "system" fn callback(hwnd: windows::Win32::Foundation::HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam.0 as *mut Ctx);
        if IsWindowVisible(hwnd).as_bool() {
            let len = GetWindowTextLengthW(hwnd);
            if len > 0 {
                let mut buf = vec![0u16; (len + 1) as usize];
                GetWindowTextW(hwnd, &mut buf);
                let titolo_finestra = String::from_utf16_lossy(&buf[..len as usize]).to_lowercase();
                if titolo_finestra.contains(ctx.titolo) {
                    let _ = PostMessageW(hwnd, WM_CLOSE, None, None);
                    ctx.trovata = true;
                }
            }
        }
        BOOL::from(true)
    }

    let mut ctx = Ctx { titolo, trovata: false };
    unsafe {
        let _ = EnumWindows(Some(callback), LPARAM(&mut ctx as *mut Ctx as isize));
    }
    ctx.trovata
}

/// macOS: usa "System Events" via osascript per trovare le finestre di
/// qualsiasi applicazione il cui titolo contiene `titolo` e le chiude
/// premendo il pulsante di chiusura. Richiede il permesso di Accessibilità
/// concesso una tantum (non è una config del browser).
#[cfg(target_os = "macos")]
fn chiudi_finestra_macos(titolo: &str) -> bool {
    let script = format!(
        r#"tell application "System Events"
        set trovata to false
        repeat with proc in (every application process whose background only is false)
    try
    repeat with w in (windows of proc)
    if (name of w as string) contains "{}" then
        try
        click (first button of w whose subrole is "AXCloseButton")
    set trovata to true
    end try
    end if
    end repeat
    end try
    end repeat
    return trovata
    end tell"#,
    titolo.replace('"', "")
    );
    std::process::Command::new("osascript")
    .arg("-e").arg(&script)
    .output()
    .map(|o| o.status.success())
    .unwrap_or(false)
}

/// Linux: prova prima `wmctrl` (finestre X11/XWayland), poi `kdotool`
/// (finestre native Wayland su KDE/KWin) se disponibile. Entrambi sono tool
/// di sistema da installare a parte (`wmctrl`, `kdotool`) — non è possibile
/// enumerare/chiudere finestre di altre app su Wayland nativo senza un
/// helper esterno, per il modello di sicurezza del protocollo.
#[cfg(target_os = "linux")]
fn chiudi_finestra_linux(titolo: &str) -> bool {
    let via_wmctrl = std::process::Command::new("wmctrl")
    .arg("-c").arg(titolo)
    .output()
    .map(|o| o.status.success())
    .unwrap_or(false);
    if via_wmctrl { return true; }

    std::process::Command::new("kdotool")
    .arg("search").arg("--name").arg(titolo)
    .output()
    .ok()
    .and_then(|o| {
        let ids = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if ids.is_empty() { return None; }
        let mut chiuse = false;
        for id in ids.lines() {
            let ok = std::process::Command::new("kdotool")
            .arg("windowclose").arg(id)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
            chiuse = chiuse || ok;
        }
        Some(chiuse)
    })
    .unwrap_or(false)
}

/// Soglia di similarità fonetica per il fuzzy match sui comandi.
/// 0.85 è un punto di partenza: se generi troppi falsi positivi
/// (comandi sbagliati riconosciuti come corretti), alzala verso 0.90-0.92.
/// Se invece continuano a sfuggire varianti valide, abbassala verso 0.80.
const FUZZY_THRESHOLD: f64 = 0.85;

// ─── Helpers JSON ────────────────────────────────────────────────────────────

/// Sceglie un messaggio casuale da un array JSON
fn messaggio_casuale(arr: &Value) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let lista = arr.as_array().map(|v| v.as_slice()).unwrap_or(&[]);
    if lista.is_empty() { return String::new(); }
    let idx = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.subsec_nanos() as usize)
    .unwrap_or(0)
    % lista.len();
    lista[idx].as_str().unwrap_or("").to_string()
}

/// Restituisce le parole di una chiave commands/objects come Vec<String>
fn kw(messages: &Value, section: &str, key: &str) -> Vec<String> {
    messages[section][key]
    .as_array()
    .unwrap_or(&vec![])
    .iter()
    .filter_map(|v| v.as_str().map(|s| s.to_lowercase()))
    .collect()
}

/// Controlla se il comando contiene almeno una delle parole chiave (match
/// substring esatto). Il fuzzy match fonetico è stato rimosso: generava più
/// falsi positivi (dirottamenti su comandi sbagliati) di quanti errori di
/// trascrizione risolvesse.
fn contiene(cmd: &str, parole: &[String]) -> bool {
    parole.iter().any(|p| cmd.contains(p.as_str()))
}

/// Rimuove la wakeword dal comando: prima prova la sostituzione esatta,
/// poi cerca la parola più simile foneticamente e la rimuove (stessa
/// tolleranza di contiene()), per gestire varianti
fn rimuovi_wakeword(cmd: &str, wakeword: &str) -> String {
    if cmd.contains(wakeword) {
        return cmd.replace(wakeword, "").trim().to_string();
    }
    let parole: Vec<&str> = cmd.split_whitespace().collect();
    if let Some(pos) = parole.iter().position(|w| jaro_winkler(w, wakeword) > FUZZY_THRESHOLD) {
        return parole.iter().enumerate()
        .filter(|(i, _)| *i != pos)
        .map(|(_, w)| *w)
        .collect::<Vec<&str>>()
        .join(" ")
        .trim()
        .to_string();
    }
    cmd.trim().to_string()
}

// ─── Correzioni fonetiche ────────────────────────────────────────────────────

fn adatta_lingua(cmd: &str) -> String {
    let correzioni = [
        (r"\bmito\b",     "mitology"),
        (r"\bmitolo\b",   "mitology"),
        (r"\bcrita\b",    "krita"),
        (r"\bcreta\b",    "krita"),
        (r"\bconsole\b",  "konsole"),
        (r"\bcaffeine\b", "kaffeine"),
        (r"\bcate\b",     "kate"),
        (r"\bspegne\b",   "spegni"),
        (r"\bspenge\b",   "spegni"),
        (r"\bspinge\b",   "spegni"),
        (r"\bspingi\b",   "spegni"),
        (r"\bdolfin\b",   "dolphin")
    ];
    let mut result = cmd.to_string();
    for (pattern, replacement) in &correzioni {
        if let Ok(re) = Regex::new(pattern) {
            result = re.replace_all(&result, *replacement).into_owned();
        }
    }
    result
}

/// Estrae il primo URL da una stringa
#[allow(dead_code)]
fn estrai_url(testo: &str) -> Option<String> {
    Regex::new(r"https?://[^\s]+").unwrap()
    .find(testo)
    .map(|m| m.as_str().to_string())
}

/// Converte un percorso "letto ad alta voce" nel testo reale: lo STT non
/// produce mai il carattere `/`, lo trascrive come parola ("slash"/"barra"),
/// e allo stesso modo "punto", "trattino", "trattino basso"/"underscore".
/// Sostituisce queste parole con il simbolo corrispondente e incolla tutto
/// senza spazi (i componenti di un percorso non ne contengono).
/// Se il percorso arriva già con i simboli veri (es. digitato da tastiera),
/// la funzione non lo altera: le parole non riconosciute passano invariate.
fn normalizza_percorso_vocale(testo: &str) -> String {
    let testo = testo.replace("trattino basso", "underscore");
    testo.split_whitespace()
    .map(|parola| match parola {
        "slash" | "barra" => "/",
        "punto" => ".",
        "trattino" => "-",
        "underscore" => "_",
        altro => altro,
    })
    .collect::<String>()
}

/// Restituisce il testo che segue l'occorrenza più a destra tra le parole
/// chiave date (es. "cartella"/"crea"): usato per isolare il percorso in
/// comandi come "crea cartella /home/riccardo/nuova". Se più parole chiave
/// compaiono nel comando, si prende quella che finisce più avanti nel testo,
/// così il percorso non include per errore altre parole chiave.
fn estrai_dopo(cmd: &str, parole: &[String]) -> Option<String> {
    let mut fine_migliore: Option<usize> = None;
    for p in parole {
        if p.is_empty() { continue; }
        if let Some(pos) = cmd.rfind(p.as_str()) {
            let fine = pos + p.len();
            if fine_migliore.map_or(true, |m| fine > m) {
                fine_migliore = Some(fine);
            }
        }
    }
    fine_migliore
    .map(|pos| cmd[pos..].trim().to_string())
    .filter(|s| !s.is_empty())
}

/// Se il testo è nella forma "<nome> in <luogo>" (o "dentro <luogo>"),
/// restituisce (nome, luogo) — usato per comandi come "cartella nuovo in
/// scaricati", dove "luogo" viene poi risolto da
/// system::risolvi_cartella_comune. Se non trova questi separatori
/// restituisce None e il chiamante ricade sul percorso assoluto letto per
/// intero.
fn estrai_nome_e_luogo(testo: &str) -> Option<(String, String)> {
    for sep in [" in ", " dentro "] {
        if let Some(pos) = testo.find(sep) {
            let nome = testo[..pos].trim().to_string();
            let luogo = testo[pos + sep.len()..].trim().to_lowercase();
            if !nome.is_empty() && !luogo.is_empty() {
                return Some((nome, luogo));
            }
        }
    }
    None
}

/// Estrae sorgente e destinazione da un comando "sposta <src> in/dentro/verso
/// <dst>". Rimuove prima la parola comando (sposta/muovi), poi divide sul
/// primo separatore trovato.
fn estrai_sorgente_destinazione(cmd: &str, cmd_move: &[String]) -> Option<(String, String)> {
    let mut testo = cmd.to_string();
    for p in cmd_move {
        if let Some(pos) = testo.find(p.as_str()) {
            testo.replace_range(pos..pos + p.len(), "");
            break;
        }
    }
    let testo = testo.trim();

    for sep in [" in ", " dentro ", " verso "] {
        if let Some(pos) = testo.find(sep) {
            let sorgente = testo[..pos].trim().to_string();
            let destinazione = testo[pos + sep.len()..].trim().to_string();
            if !sorgente.is_empty() && !destinazione.is_empty() {
                return Some((sorgente, destinazione));
            }
        }
    }
    None
}

/// Legge `listaprogrammi` (stesso file usato da `system::apri_programma`,
/// righe "Nome=eseguibile") e produce un elenco "Nome (eseguibile)" per il
/// system prompt dell'AI agent. Include solo eseguibili "nudi" (senza path
/// assoluto né argomenti): sono gli unici che `open_program` in
/// `esegui_azione_ai` accetta — un path assoluto o una stringa con spazi
/// verrebbe comunque scartata a valle, quindi non ha senso suggerirla.
/// Deduplicato sull'eseguibile, troncato a `max` voci per non gonfiare il prompt.
fn lista_programmi_per_ai(path: &std::path::Path, max: usize) -> String {
    use std::fs::File;
    use std::io::{BufRead, BufReader};
    use std::collections::HashSet;

    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return String::new(),
    };

    let mut visti = HashSet::new();
    let mut voci = Vec::new();

    for line in BufReader::new(file).lines().flatten() {
        let line = line.trim().to_string();
        if line.is_empty() || line.starts_with('#') { continue; }
        let Some((nome, eseguibile)) = line.split_once('=') else { continue; };
        let nome = nome.trim();
        let eseguibile = eseguibile.trim();
        let bin = eseguibile.split_whitespace().next().unwrap_or("");
        if bin.is_empty() || bin.contains('/') { continue; }
        if !visti.insert(bin.to_string()) { continue; }
        voci.push(format!("{} ({})", nome, bin));
        if voci.len() >= max { break; }
    }

    voci.join(", ")
}

// ─── AI Action Agent ─────────────────────────────────────────────────────────

fn esegui_azione_ai(action: &Value) -> Result<bool, anyhow::Error> {
    let tipo = action["action"].as_str().unwrap_or("");

    match tipo {
        "open_program" => {
            let programma = action["program"].as_str().unwrap_or("").trim();
            if programma.is_empty() || programma.contains('/') || programma.contains(' ') {
                return Ok(false);
            }
            std::process::Command::new(programma).spawn()?;
            Ok(true)
        }
        "open_url" => {
            let url = action["url"].as_str().unwrap_or("").trim();
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Ok(false);
            }
            webbrowser::open(url).map_err(|e| anyhow::anyhow!(e.to_string()))?;
            Ok(true)
        }
        "type_text" => {
            let testo = action["text"].as_str().unwrap_or("");
            let status = std::process::Command::new("wtype").arg(testo).status()?;
            Ok(status.success())
        }
        "key" => {
            let key = action["key"].as_str().unwrap_or("").trim();
            if key.is_empty() { return Ok(false); }
            let status = std::process::Command::new("wtype").args(["-k", key]).status()?;
            Ok(status.success())
        }
        "hotkey" => {
            let keys = match action["keys"].as_array() {
                Some(k) if !k.is_empty() => k,
                _ => return Ok(false),
            };
            let mut cmd = std::process::Command::new("wtype");
            let mut modifiers = Vec::new();
            let mut normal_key: Option<String> = None;

            for value in keys {
                let key = value.as_str().unwrap_or("").trim();
                match key.to_uppercase().as_str() {
                    "CTRL" | "CONTROL" => modifiers.push("ctrl"),
                    "ALT" => modifiers.push("alt"),
                    "SHIFT" => modifiers.push("shift"),
                    "SUPER" | "META" | "WIN" | "LOGO" => modifiers.push("logo"),
                    _ => {
                        if normal_key.is_some() { return Ok(false); }
                        normal_key = Some(key.to_string());
                    }
                }
            }

            let key = match normal_key {
                Some(k) if !k.is_empty() => k,
                _ => return Ok(false),
            };
            for modifier in &modifiers { cmd.args(["-M", modifier]); }
            cmd.args(["-k", &key]);
            for modifier in modifiers.iter().rev() { cmd.args(["-m", modifier]); }
            Ok(cmd.status()?.success())
        }
        "wait" => {
            let ms = action["ms"].as_u64().unwrap_or(0).min(10_000);
            std::thread::sleep(std::time::Duration::from_millis(ms));
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn esegui_piano_ai(piano: &Value) -> Result<usize, anyhow::Error> {
    let actions = match piano["actions"].as_array() {
        Some(actions) if !actions.is_empty() => actions,
        _ => return Ok(0),
    };
    let mut eseguite = 0usize;
    for action in actions {
        if !esegui_azione_ai(action)? {
            return Err(anyhow::anyhow!("azione AI non valida o non eseguibile"));
        }
        eseguite += 1;
    }
    Ok(eseguite)
}


// ─── IntentHandler ───────────────────────────────────────────────────────────

pub struct IntentHandler {
    pub active: bool,
    pub wakeword: String,
    pub botname: String,
    pub messages: Value,
    pub listaprogrammi: PathBuf,
    pub listabookmarks: PathBuf,
    pub stations_csv: PathBuf,
    pub youtube_open: bool,
    pub awaiting_shutdown: bool,
    pub awaiting_reboot: bool,
    /// Percorso in attesa di conferma vocale per la cancellazione
    /// (comando "cancella file/cartella ..."), None se nessuna cancellazione
    /// è in sospeso.
    pub awaiting_delete: Option<String>,
    pub tx_output: Sender<String>,
    /// ID finestra (per OS che lo supportano, es. Linux/kdotool) tracciato
    /// per ogni bookmark aperto in finestra dedicata, cos\u00ec la chiusura pu\u00f2
    /// mirare esattamente a quella finestra invece di indovinare dal titolo.
    finestre_bookmark: std::collections::HashMap<String, String>,
}

impl IntentHandler {
    pub fn new(
        wakeword: String,
        botname: String,
        messages: Value,
        listaprogrammi: PathBuf,
        listabookmarks: PathBuf,
        stations_csv: PathBuf,
        tx_output: Sender<String>,
    ) -> Self {
        Self {
            active: false,
            wakeword,
            botname,
            messages,
            listaprogrammi,
            listabookmarks,
            stations_csv,
            youtube_open: false,
            awaiting_shutdown: false,
            awaiting_reboot: false,
            awaiting_delete: None,
            tx_output,
            finestre_bookmark: std::collections::HashMap::new(),
        }
    }

    pub fn comrecon(&mut self, comando: &str, api_key: &str, yt_key: &str) -> Result<bool, anyhow::Error> {

        // Se comando proveniente dalla GUI
        if comando.starts_with("__GUI__") {
            let cmd = comando.trim_start_matches("__GUI__");
            self.esegui(cmd, api_key, yt_key)?;
            return Ok(self.active);
        }

        let cmd_lower = comando.to_lowercase();

        // Funzione locale per pulire il testo dopo la wakeword.
        // Whisper può restituire: "marco", "marco!", "marco...", "marco…"
        let pulisci_comando = |testo: String| -> String {
            testo
            .trim()
            .trim_matches(|c: char| c.is_ascii_punctuation())
            .trim()
            .to_string()
        };

        if !self.active {
            if contiene(&cmd_lower, &[self.wakeword.clone()]) {
                self.active = true;

                // Aggiorna SUBITO la UI prima di qualsiasi parlato
                let _ = self.tx_output.send(format!("🤖 {} attivo", self.botname));

                let pulito = pulisci_comando(rimuovi_wakeword(&cmd_lower, &self.wakeword));

                if pulito.is_empty() {
                    let msg = messaggio_casuale(&self.messages["welcome_messages"]);
                    tts::speak(&msg)?;
                } else {
                    let log = self.messages["other_messages"]["log_attivato_comando"]
                    .as_str().unwrap_or("[missing: other_messages.log_attivato_comando]")
                    .replace("{botname}", &self.botname)
                    .replace("{comando}", &pulito);
                    println!("🤖 {}", log);
                    self.esegui(&pulito, api_key, yt_key)?;
                }
            }
            return Ok(self.active);
        }

        let pulito = pulisci_comando(rimuovi_wakeword(&cmd_lower, &self.wakeword));
        if pulito.is_empty() {
            let msg = messaggio_casuale(&self.messages["welcome_messages"]);
            tts::speak(&msg)?;
        } else {
            self.esegui(&pulito, api_key, yt_key)?;
        }
        Ok(self.active)
    }

    pub fn esegui(&mut self, comando: &str, api_key: &str, yt_key: &str) -> Result<(), anyhow::Error> {
        let c = adatta_lingua(&comando.to_lowercase());
        let log_comando = self.messages["other_messages"]["log_comando"]
        .as_str().unwrap_or("[missing: other_messages.log_comando]")
        .replace("{comando}", &c);
        println!("📝 {}", log_comando);
        let _ = self.tx_output.send(format!("📝 {}", log_comando));  // ← aggiunta che consente alla ui di vedere il comando

        // Legge tutte le keyword dal JSON una volta sola
        let cmd_open     = kw(&self.messages, "commands", "open");
        let cmd_close    = kw(&self.messages, "commands", "close");
        let cmd_turnoff  = kw(&self.messages, "commands", "turnoff");
        let cmd_restart  = kw(&self.messages, "commands", "restart");
        let cmd_exit     = kw(&self.messages, "commands", "exit");
        let cmd_search   = kw(&self.messages, "commands", "search");
        #[allow(unused_variables)]
        let cmd_get_ai   = kw(&self.messages, "commands", "getAI");
        let cmd_reply    = kw(&self.messages, "commands", "reply");
        let cmd_change   = kw(&self.messages, "commands", "change");
        let cmd_update   = kw(&self.messages, "commands", "update");
        let cmd_upvol    = kw(&self.messages, "commands", "upvol");
        let cmd_downvol  = kw(&self.messages, "commands", "downvol");
        let cmd_setvol   = kw(&self.messages, "commands", "setvol");
        let cmd_silent   = kw(&self.messages, "commands", "silent");
        let cmd_mute     = kw(&self.messages, "commands", "mute");
        let cmd_move     = kw(&self.messages, "commands", "move");
        let cmd_create   = kw(&self.messages, "commands", "create");
        let cmd_delete   = kw(&self.messages, "commands", "delete");

        let obj_pc       = kw(&self.messages, "objects", "pc");
        let obj_program  = kw(&self.messages, "objects", "program");
        let obj_list     = kw(&self.messages, "objects", "list");
        let obj_internet = kw(&self.messages, "objects", "internet");
        let obj_music    = kw(&self.messages, "objects", "music");
        let _obj_window   = kw(&self.messages, "objects", "window");
        let obj_filemanager = kw(&self.messages, "objects", "filemanager");
        let obj_folder   = kw(&self.messages, "objects", "folder");
        let obj_update_full = kw(&self.messages, "objects", "update_full");

        // Messaggi dal JSON
        let msg_conferma    = self.messages["other_messages"]["command_confirmation"]
        .as_str().unwrap_or("[missing: other_messages.command_confirmation]").to_string();
        let msg_radio_off   = self.messages["other_messages"]["radio_closed"]
        .as_str().unwrap_or("").to_string();
        let msg_radio_list  = self.messages["other_messages"]["radio_list"]
        .as_str().unwrap_or("").to_string();
        let msg_update      = self.messages["other_messages"]["update_in_progress"]
        .as_str().unwrap_or("").to_string();
        let msg_browser_on  = self.messages["other_messages"]["browser_opened"]
        .as_str().unwrap_or("").to_string();
        let msg_browser_off = self.messages["other_messages"]["browser_closed"]
        .as_str().unwrap_or("").to_string();
        let msg_youtube_on  = self.messages["other_messages"]["youtube_opened"]
        .as_str().unwrap_or("").to_string();
        let msg_music_off   = self.messages["other_messages"]["music_player_closed"]
        .as_str().unwrap_or("").to_string();
        let msg_cancelled   = self.messages["other_messages"]["shutdown_cancelled"]
        .as_str().unwrap_or("").to_string();
        let msg_reboot_ok   = self.messages["other_messages"]["reboot_executed"]
        .as_str().unwrap_or("").to_string();
        let shutdown_msgs   = &self.messages["other_messages"]["shutdown_executed"];
        let msg_program_opened   = self.messages["other_messages"]["program_opened"]
        .as_str().unwrap_or("[missing: other_messages.program_opened]").to_string();
        let msg_program_closed   = self.messages["other_messages"]["program_closed"]
        .as_str().unwrap_or("[missing: other_messages.program_closed]").to_string();
        let msg_program_not_found = self.messages["error_messages"]["program_not_found"]
        .as_str().unwrap_or("[missing: error_messages.program_not_found]").to_string();
        let msg_not_recognized   = self.messages["error_messages"]["command_not_recognized"]
        .as_str().unwrap_or("[missing: error_messages.command_not_recognized]").to_string();
        let msg_volume_set       = self.messages["other_messages"]["volume_set"]
        .as_str().unwrap_or("[missing: other_messages.volume_set]").to_string();
        let msg_volume_increased = self.messages["other_messages"]["volume_increased"]
        .as_str().unwrap_or("[missing: other_messages.volume_increased]").to_string();
        let msg_volume_decreased = self.messages["other_messages"]["volume_decreased"]
        .as_str().unwrap_or("[missing: other_messages.volume_decreased]").to_string();
        let msg_volume_muted     = self.messages["other_messages"]["volume_muted"]
        .as_str().unwrap_or("[missing: other_messages.volume_muted]").to_string();
        let msg_muted            = self.messages["other_messages"]["muted_confirmation"]
        .as_str().unwrap_or("[missing: other_messages.muted_confirmation]").to_string();

        // ── CONFERME ─────────────────────────────────────────────────────────
        if self.awaiting_shutdown || self.awaiting_reboot || self.awaiting_delete.is_some() {
            if c.contains("no") {
                self.awaiting_shutdown = false;
                self.awaiting_reboot   = false;
                self.awaiting_delete   = None;
                crate::vocalrecon::set_awaiting_confirmation(false);
                tts::speak(&msg_cancelled)?;
                return Ok(());
            } else if contiene(&c, &cmd_reply) {
                crate::vocalrecon::set_awaiting_confirmation(false);
                if self.awaiting_shutdown {
                    self.awaiting_shutdown = false;
                    let msg = messaggio_casuale(shutdown_msgs);
                    tts::speak(&msg)?;
                    let saluto = messaggio_casuale(&self.messages["goodbye_messages"]);
                    tts::speak(&saluto)?;
                    system::shutdown(); //bypassare se occorre fare verifiche
                } else if self.awaiting_reboot {
                    self.awaiting_reboot = false;
                    tts::speak(&msg_reboot_ok)?;
                    system::reboot(); //bypassare se occorre fare verifiche
                } else if let Some(percorso) = self.awaiting_delete.take() {
                    let msg = system::cancella_file(&percorso);
                    tts::speak(&msg)?;
                }
                return Ok(());
            }
            // Comando non correlato alla conferma: annulla implicitamente la richiesta
            // in sospeso e lascia che il comando prosegua nel resto della funzione,
            // invece di scartarlo silenziosamente.
            self.awaiting_shutdown = false;
            self.awaiting_reboot   = false;
            self.awaiting_delete   = None;
            crate::vocalrecon::set_awaiting_confirmation(false);
        }

        // ── STAI ZITTO / SILENZIO ────────────────────────────────────────────
        // Torna in stand-by esattamente come al timeout di inattività: disattiva
        // active e le conferme in sospeso, senza spegnere né chiudere nulla.
        if contiene(&c, &cmd_mute) {
            tts::speak(&msg_muted)?;
            self.active = false;
            self.awaiting_shutdown = false;
            self.awaiting_reboot   = false;
            self.awaiting_delete   = None;
            let waiting_msg = self.messages["other_messages"]["waiting_wakeword"]
            .as_str().unwrap_or("[missing: other_messages.waiting_wakeword]")
            .replace("{botname}", &self.botname);
            let _ = self.tx_output.send(waiting_msg);
            return Ok(());
        }

        // ── GESTIONE FILE (sposta / crea cartella / cancella) ───────────────────
        // Il percorso va detto/scritto per intero (es. "sposta /home/riccardo/
        // foto.jpg in /home/riccardo/Immagini"). La cancellazione richiede
        // sempre conferma vocale, sposta e crea cartella eseguono subito.
        if contiene(&c, &cmd_move) {
            match estrai_sorgente_destinazione(&c, &cmd_move) {
                Some((sorgente, destinazione)) => {
                    let sorgente = normalizza_percorso_vocale(&sorgente);
                    let destinazione = match system::risolvi_cartella_comune(&destinazione.trim().to_lowercase()) {
                        Some(base) => base.to_string_lossy().to_string(),
                        None => normalizza_percorso_vocale(&destinazione),
                    };
                    let msg = system::sposta_file(&sorgente, &destinazione);
                    tts::speak(&msg)?;
                }
                None => { tts::speak(&msg_not_recognized)?; }
            }
            return Ok(());
        }

        if contiene(&c, &cmd_create) && contiene(&c, &obj_folder) {
            let mut parole_chiave = obj_folder.clone();
            parole_chiave.extend(cmd_create.iter().cloned());
            match estrai_dopo(&c, &parole_chiave) {
                Some(testo) => {
                    let luogo_risolto = estrai_nome_e_luogo(&testo)
                    .and_then(|(nome, luogo)| {
                        system::risolvi_cartella_comune(&luogo).map(|base| base.join(nome))
                    });
                    let msg = match luogo_risolto {
                        Some(percorso) => system::crea_directory(&percorso.to_string_lossy()),
                        None => system::crea_directory(&normalizza_percorso_vocale(&testo)),
                    };
                    tts::speak(&msg)?;
                }
                None => { tts::speak(&msg_not_recognized)?; }
            }
            return Ok(());
        }

        if contiene(&c, &cmd_delete) {
            let mut parole_chiave = obj_folder.clone();
            parole_chiave.push("file".to_string());
            parole_chiave.extend(cmd_delete.iter().cloned());
            match estrai_dopo(&c, &parole_chiave) {
                Some(testo) => {
                    let luogo_risolto = estrai_nome_e_luogo(&testo)
                    .and_then(|(nome, luogo)| {
                        system::risolvi_cartella_comune(&luogo).map(|base| base.join(nome))
                    });
                    let percorso = match luogo_risolto {
                        Some(p) => p.to_string_lossy().to_string(),
                        None => normalizza_percorso_vocale(&testo),
                    };
                    self.awaiting_delete = Some(percorso);
                    tts::speak(&msg_conferma)?;
                    crate::vocalrecon::set_awaiting_confirmation(true);
                }
                None => { tts::speak(&msg_not_recognized)?; }
            }
            return Ok(());
        }

        // ── RADIO ─────────────────────────────────────────────────────────────
        if c.contains("radio") {
            if contiene(&c, &cmd_turnoff) || contiene(&c, &cmd_close) {
                tts::speak(&msg_radio_off)?;
                radio::stop_radio();
            } else if contiene(&c, &obj_list) {
                let lista = radio::lista_stazioni(&self.stations_csv);
                println!("{}", lista);
                crate::ui::mostra_nota(&lista);
                tts::speak(&msg_radio_list)?;
            } else if c.contains("volume")
                || contiene(&c, &cmd_upvol) || contiene(&c, &cmd_downvol)
                || contiene(&c, &cmd_setvol) || contiene(&c, &cmd_silent)
                {
                    if let Ok(cfg) = crate::config::load_config() {
                        let risultato = system::set_volume(
                            &c, cfg.deltavolume,
                            &cmd_setvol, &cmd_upvol, &cmd_downvol, &cmd_silent,
                            &msg_volume_set, &msg_volume_increased,
                            &msg_volume_decreased, &msg_volume_muted,
                        );
                        if let Some(msg) = risultato {
                            tts::speak_silent(&msg)?;
                        } else {
                            tts::speak_silent(&msg_not_recognized)?;
                        }
                    }
                } else if contiene(&c, &cmd_change) || contiene(&c, &cmd_open) {
                    radio::search_and_play(&c, &self.stations_csv);
                } else {
                    radio::search_and_play(&c, &self.stations_csv);
                }
                return Ok(());
        }

        // ── YOUTUBE ───────────────────────────────────────────────────────────
        // Il flag youtube_open resta attivo dopo un'apertura, ma deve valere solo
        // come continuazione di una ricerca ("cerca gatti" subito dopo "apri youtube"),
        // non per qualsiasi comando successivo (es. "spegni il computer").
        let continua_ricerca_yt =
        self.youtube_open && (contiene(&c, &cmd_search) || c.contains("cerca"));
        let vuole_chiudere = contiene(&c, &cmd_close);
        if (c.contains("youtube") || continua_ricerca_yt) && !vuole_chiudere {
            // Comando di sola apertura ("apri youtube") senza richiesta di ricerca
            // esplicita: apri la homepage invece di lanciare una ricerca video.
            let vuole_cercare = contiene(&c, &cmd_search) || c.contains("cerca");
            if contiene(&c, &cmd_open) && !vuole_cercare {
                tts::speak(&msg_youtube_on)?;
                if let Ok(cfg) = crate::config::load_config() {
                    if let Some(id) = apri_finestra_dedicata_tracciata(&cfg.browser, "https://www.youtube.com") {
                        self.finestre_bookmark.insert("youtube".to_string(), id);
                    }
                } else {
                    let _ = webbrowser::open("https://www.youtube.com");
                }

                self.youtube_open = true;
                return Ok(());
            }

            let query = Regex::new(r"(?i)cerca su youtube")
            .unwrap()
            .replace_all(&c, "")
            .trim()
            .to_string();
            if !yt_key.is_empty() {
                if let Ok(urls) = crate::ai::search_youtube(&query, yt_key, 3) {
                    for url in &urls { let _ = webbrowser::open(url); }
                    if c.contains("youtube") { self.youtube_open = true; }
                }
            } else {
                let url = format!(
                    "https://www.youtube.com/results?search_query={}",
                    urlencoding::encode(&query)
                );
                let _ = webbrowser::open(&url);
            }
            return Ok(());
        }

        // ── VOLUME ────────────────────────────────────────────────────────────
        if c.contains("volume")
            || contiene(&c, &cmd_upvol)
            || contiene(&c, &cmd_downvol)
            || contiene(&c, &cmd_setvol)
            || contiene(&c, &cmd_silent)
            {
                if let Ok(cfg) = crate::config::load_config() {
                    let risultato = system::set_volume(
                        &c, cfg.deltavolume,
                        &cmd_setvol, &cmd_upvol, &cmd_downvol, &cmd_silent,
                        &msg_volume_set, &msg_volume_increased,
                        &msg_volume_decreased, &msg_volume_muted,
                    );
                    if let Some(msg) = risultato {
                        tts::speak_silent(&msg)?;
                    } else {
                        tts::speak_silent(&msg_not_recognized)?;
                    }
                }
                return Ok(());
            }

            //----Aggiornamento SISTEMA

            if contiene(&c, &cmd_update) && contiene(&c, &obj_pc) {
                tts::speak(&msg_update)?;

                let msg_completed = self.messages["other_messages"]["update_completed"]
                .as_str().unwrap_or("[missing: other_messages.update_completed]")
                .to_string();

                let completo = obj_update_full.iter().any(|w| c.contains(w.as_str()));

                system::aggiorna_sistema(msg_completed,completo);

                return Ok(());
            }


            // ── SPEGNI PC ─────────────────────────────────────────────────────────
            if contiene(&c, &cmd_turnoff) && contiene(&c, &obj_pc) {
                tts::speak(&msg_conferma)?;
                self.awaiting_shutdown = true;
                crate::vocalrecon::set_awaiting_confirmation(true);
                return Ok(());
            }


            // ── RIAVVIA PC ────────────────────────────────────────────────────────
            if contiene(&c, &cmd_restart) && contiene(&c, &obj_pc) {
                tts::speak(&msg_conferma)?;
                self.awaiting_reboot = true;
                crate::vocalrecon::set_awaiting_confirmation(true);
                return Ok(());
            }

            // ── CHIUDI ASSISTENTE ─────────────────────────────────────────────────
            let solo_cmd_exit = cmd_exit.iter().any(|w| c.trim() == w);

            if (contiene(&c, &cmd_exit) && contiene(&c, &obj_program)) || solo_cmd_exit {
                let saluto = messaggio_casuale(&self.messages["goodbye_messages"]);
                println!("🤖 {}: {}", self.botname, saluto);
                tts::speak(&saluto)?;
                std::thread::sleep(std::time::Duration::from_secs(2));
                // Non usciamo direttamente da qui: questo gira sul thread
                // "intent", non su quello Qt. process::exit() da un thread
                // diverso da quello che possiede l'event loop Qt causa un
                // segfault (thread-affinity degli oggetti Qt violata).
                // Il sentinel viene intercettato in ui.rs, dentro il
                // callback che gira già sul thread Qt corretto.
                let _ = self.tx_output.send("__EXIT__".to_string());
                return Ok(());
            }

             // ── CERCA / AI diretta ────────────────────────────────────────────────
            // Deve stare PRIMA di APRI/CHIUDI: altrimenti un comando come
            // "cercami quanto ci mette la stampante 3d..." può essere
            // intercettato da un match accidentale su bookmark/programma
            // dentro il blocco APRI, invece di arrivare qui.
            /*if contiene(&c, &cmd_search) || contiene(&c, &cmd_get_ai) {
                if !api_key.is_empty() {
                    match crate::ai::ask_groq(&c, api_key) {
                        Ok(reply) => {
                            if let Some(url) = estrai_url(&reply) {
                                let _ = webbrowser::open(&url);
                            } else {
                                // Risposta AI: apre solo la finestra note, senza TTS
                                crate::ui::mostra_nota(&reply);

                            }
                        }
                        Err(_) => {
                            let msg = self.messages["error_messages"]["command_not_recognized"]
                            .as_str().unwrap_or("[missing: error_messages.command_not_recognized]");
                            tts::speak(msg)?;
                        }
                    }
                }
                return Ok(());
            }*/


            // ── APRI ──────────────────────────────────────────────────────────────
            if contiene(&c, &cmd_open) {
                // Gestore file
                if obj_filemanager.iter().all(|w| c.contains(w.as_str())) {
                    system::apri_gestore_file(".");
                    return Ok(());
                }
                // Internet/browser
                if contiene(&c, &obj_internet) {
                    let _ = webbrowser::open("https://www.google.it");
                    tts::speak(&msg_browser_on)?;
                    if c.contains("youtube") { self.youtube_open = true; }
                    return Ok(());
                }
                // Musica
                if contiene(&c, &obj_music) {
                    if let Ok(cfg) = crate::config::load_config() {
                        let _ = std::process::Command::new(&cfg.musicplayer).spawn();
                        let msg = self.messages["other_messages"]["music_player_opened"]
                        .as_str().unwrap_or("")
                        .replace("{musicprog}", &cfg.musicplayer);
                        tts::speak(&msg)?;
                    }
                    return Ok(());
                }
                // Bookmark
                if self.apri_bookmark(&c) { return Ok(()); }
                // Programma
                match system::apri_programma(&c, &self.listaprogrammi) {
                    Some(nome) => {
                        let msg = msg_program_opened.replace("{programma}", &nome);
                        tts::speak(&msg)?;
                    }
                    None => {
                        let msg = msg_program_not_found.replace("{program}", &c);
                        tts::speak(&msg)?;
                    }
                }
                return Ok(());
            }

            // ── CHIUDI ────────────────────────────────────────────────────────────
            if contiene(&c, &cmd_close) {
                // Bookmark: se il comando nomina un sito specifico presente nei
                // bookmark (es. "chiudi youtube", "chiudi gmail"...), chiude solo
                // le sue tab (via CDP) e non l'intero browser.
                if self.chiudi_bookmark(&c) { return Ok(()); }
                if contiene(&c, &obj_internet) {
                    self.youtube_open = false;
                    if let Ok(cfg) = crate::config::load_config() {
                        termina_processo(&cfg.browser);
                    }
                    tts::speak(&msg_browser_off)?;
                    return Ok(());
                }
                if contiene(&c, &obj_music) {
                    if let Ok(cfg) = crate::config::load_config() {
                        termina_processo(&cfg.musicplayer);
                    }
                    tts::speak(&msg_music_off)?;
                    return Ok(());
                }
                match system::chiudi_programma(&c, &self.listaprogrammi) {
                    Some(nome) => {
                        let msg = msg_program_closed.replace("{programma}", &nome);
                        tts::speak(&msg)?;
                    }
                    None => {
                        tts::speak(&msg_not_recognized)?;
                    }
                }
                return Ok(());
            }

            // ── FALLBACK AI / ACTION AGENT ────────────────────────────────────────
            // Gli intent locali hanno sempre la precedenza. Solo ciò che non è
            // stato riconosciuto arriva qui: l'AI genera un piano JSON e Rust lo esegue.
            if !api_key.is_empty() {
                let (browser, musicplayer) = match crate::config::load_config() {
                    Ok(cfg) => (cfg.browser, cfg.musicplayer),
                    Err(_) => (String::new(), String::new()),
                };
                let programmi = lista_programmi_per_ai(&self.listaprogrammi, 150);
                match crate::ai::ask_groq_agent(&c, api_key, &browser, &musicplayer, &programmi) {
                    Ok(piano) => match esegui_piano_ai(&piano) {
                        Ok(0) => {
                            // L'AI ha risposto ma non ha trovato nessuna azione eseguibile.
                            let msg = self.messages["error_messages"]["ai_no_action"]
                            .as_str().unwrap_or("[missing: error_messages.ai_no_action]");
                            tts::speak(msg)?;
                        }
                        Ok(n) => {

                            println!("🤖 {}: eseguite {} azioni", self.botname,n);
                            let _ = self.tx_output.send(format!("🤖 {}: eseguite {} azioni", self.botname, n) );
                        }
                        Err(err) => {
                            // Il piano è stato generato ma un'azione è fallita in esecuzione.
                           eprintln!("❌ {}: {}", self.botname, err);
                            let _ = self.tx_output.send(format!("❌ AI Agent: {}", err));
                            let msg = self.messages["error_messages"]["ai_action_failed"]
                            .as_str().unwrap_or("[missing: error_messages.ai_action_failed]");
                            tts::speak(msg)?;
                        }
                    },
                    Err(err) => {
                        // La chiamata a AI agent è fallita (rete, parsing, ecc.).
                        eprintln!("❌ {}: {}", self.botname, err);
                        let _ = self.tx_output.send(format!("❌ {}: {}", self.botname, err));
                        let msg = self.messages["error_messages"]["ai_service_error"]
                        .as_str().unwrap_or("[missing: error_messages.ai_service_error]");
                        tts::speak(msg)?;
                    }
                }
            } else {
                // Nessuna api_key configurata: l'AI non è disponibile.
                let msg = self.messages["error_messages"]["ai_not_configured"]
                .as_str().unwrap_or("[missing: error_messages.ai_not_configured]");
                tts::speak(msg)?;
            }

            Ok(())
    }

    /// Cerca il comando nei bookmark e apre l'URL se trovato
    fn apri_bookmark(&mut self, comando: &str) -> bool {
        use std::fs::File;
        use std::io::{BufRead, BufReader};

        let stopword = kw(&self.messages, "objects", "bookmark_stopwords");
        let file = match File::open(&self.listabookmarks) {
            Ok(f) => f,
            Err(_) => return false,
        };
        for line in BufReader::new(file).lines().flatten() {
            let line = line.trim().to_string();
            if line.is_empty() || line.starts_with('#') { continue; }
            if let Some((nome, url)) = line.split_once('=') {
                let nome = nome.trim();
                let url  = url.trim();
                if bookmark_corrisponde(comando, nome, &stopword) {
                    if nome.to_lowercase().contains("youtube") {
                        // Solo Youtube apre in finestra dedicata, per poterla
                        // chiudere in modo mirato senza toccare il resto.
                        if let Ok(cfg) = crate::config::load_config() {
                            if let Some(id) = apri_finestra_dedicata_tracciata(&cfg.browser, url) {
                                self.finestre_bookmark.insert(nome.to_lowercase(), id);
                            }
                        } else {
                            let _ = webbrowser::open(url);
                        }
                        self.youtube_open = true;
                    } else {
                        // Bookmark normale: apre come tab nella finestra esistente.
                        let _ = webbrowser::open(url);
                    }
                    let msg = self.messages["other_messages"]["program_opened"]
                    .as_str().unwrap_or("[missing: other_messages.program_opened]")
                    .replace("{programma}", nome);
                    let _ = tts::speak(&msg);
                    return true;
                }
            }
        }
        false
    }

    /// Cerca il comando nei bookmark e chiude il sito corrispondente: Youtube
    /// tramite l'ID di finestra tracciato (chiusura mirata, aperta in finestra
    /// dedicata); gli altri bookmark per ora tramite chiusura per titolo
    /// finestra (meno precisa, dato che aprono come tab nella finestra
    /// condivisa — può quindi chiudere anche altre tab in quella finestra).
    fn chiudi_bookmark(&mut self, comando: &str) -> bool {
        use std::fs::File;
        use std::io::{BufRead, BufReader};

        let stopword = kw(&self.messages, "objects", "bookmark_stopwords");
        let file = match File::open(&self.listabookmarks) {
            Ok(f) => f,
            Err(_) => return false,
        };
        for line in BufReader::new(file).lines().flatten() {
            let line = line.trim().to_string();
            if line.is_empty() || line.starts_with('#') { continue; }
            if let Some((nome, _url)) = line.split_once('=') {
                let nome = nome.trim();
                if bookmark_corrisponde(comando, nome, &stopword) {
                    let chiave = nome.to_lowercase();
                    if let Some(id) = self.finestre_bookmark.remove(&chiave) {
                        chiudi_finestra_id(&id);
                    } else {
                        chiudi_finestra_titolo(nome);
                    }
                    let msg = if nome.to_lowercase().contains("youtube") {
                        self.youtube_open = false;
                        self.messages["other_messages"]["youtube_closed"]
                        .as_str().unwrap_or("[missing: other_messages.youtube_closed]").to_string()
                    } else {
                        self.messages["other_messages"]["program_closed"]
                        .as_str().unwrap_or("[missing: other_messages.program_closed]")
                        .replace("{programma}", nome)
                    };
                    let _ = tts::speak(&msg);
                    return true;
                }
            }
        }
        false
    }
}
