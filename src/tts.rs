use anyhow::Result;
use rodio::{Decoder, OutputStream, Sink};
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};

static IS_SPEAKING: AtomicBool = AtomicBool::new(false);


// ─── UI hook: canale + botname, impostati una volta da main.rs ─────────────
static UI_SENDER: OnceLock<Mutex<Option<Sender<String>>>> = OnceLock::new();
static BOTNAME: OnceLock<String> = OnceLock::new();

fn ui_sender_cell() -> &'static Mutex<Option<Sender<String>>> {
    UI_SENDER.get_or_init(|| Mutex::new(None))
}

/// Registra il canale verso la UI e il botname. Chiamalo una volta sola in
/// main.rs, prima di qualunque chiamata a speak(), cosi' ogni tts::speak()
/// in qualsiasi modulo (intent.rs, radio.rs, ecc.) mostra automaticamente
/// il messaggio in UI senza doverlo fare manualmente ad ogni call site.
pub fn init_ui_output(tx_output: Sender<String>, botname: String) {
    if let Ok(mut cell) = ui_sender_cell().lock() {
        *cell = Some(tx_output);
    }
    BOTNAME.set(botname).ok();
}

fn notify_ui(text: &str) {
    let botname = BOTNAME.get().map(|s| s.as_str()).unwrap_or("assistente");
    if let Ok(cell) = ui_sender_cell().lock() {
        if let Some(tx) = cell.as_ref() {
            let _ = tx.send(format!("🤖 {}: {}", botname, text));
        }
    }
}

pub fn is_speaking() -> bool {
    IS_SPEAKING.load(Ordering::SeqCst)
}

/// Determina se usare Google TTS o Piper offline, in base alla connessione.
/// Riusa la stessa modalità già rilevata da vocalrecon (se disponibile),
/// o esegue un proprio controllo se chiamata indipendentemente.
pub fn detect_tts_mode(_online: bool) -> String {
    let messages = crate::config::load_messages().ok();
    let leggi = |chiave: &str| -> String {
        messages.as_ref()
            .and_then(|m| m.other_messages[chiave].as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| format!("[missing: other_messages.{}]", chiave))
    };
    //if online {
       // println!("🔊 {}", leggi("tts_mode_google_log"));
       //leggi("tts_mode_google_label")
   // } else {
        println!("🔊 {}", leggi("tts_mode_piper_log"));
        leggi("tts_mode_piper_label")
    //}
}

fn use_stt_online() -> bool {
    crate::vocalrecon::use_stt_online()
}

// ─── Override path piper: config.json > env var > default hardcoded ────────
static PIPER_BIN: OnceLock<Option<String>> = OnceLock::new();
static PIPER_MODEL: OnceLock<Option<String>> = OnceLock::new();

/// Registra i path piper letti da config.json. Chiamalo una volta in
/// main.rs, subito dopo config::load_config(). Passa None per i campi
/// non impostati nel json: si ricade su env var poi su default.
pub fn init_piper_paths(bin: Option<String>, model: Option<String>) {
    PIPER_BIN.set(bin).ok();
    PIPER_MODEL.set(model).ok();
}

fn expand_home(path: String) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}/{}", home, rest);
        }
    }
    path
}


fn piper_bin() -> String {
    let path = PIPER_BIN
        .get()
        .and_then(|o| o.clone())
        .or_else(|| std::env::var("ASSISTENTE_PIPER_BIN").ok())
        .unwrap_or_else(|| "~/.local/bin/piper".to_string());

    expand_home(path)
}

fn piper_model() -> String {
    let path = PIPER_MODEL
        .get()
        .and_then(|o| o.clone())
        .or_else(|| std::env::var("ASSISTENTE_PIPER_MODEL").ok())
        .unwrap_or_else(|| {
            "~/.local/share/assistente/piper/voce_marco.onnx".to_string()
        });

    expand_home(path)
}

/// Parla E mostra il messaggio in UI (comportamento di default).
pub fn speak(text: &str) -> Result<()> {
    notify_ui(text);
    speak_silent(text)
}

/// Parla SENZA mostrare nulla in UI. Da usare per i casi in cui l'echo
/// visivo non serve (es. messaggi volume).
pub fn speak_silent(text: &str) -> Result<()> {
    if text.trim().is_empty() {
        return Ok(());
    }
    IS_SPEAKING.store(true, Ordering::SeqCst);
    let result = if use_stt_online() {
        speak_google(text)
    } else {
        speak_piper(text)
    };
    IS_SPEAKING.store(false, Ordering::SeqCst);
    result
}

/// TTS tramite Google Translate (online)
fn speak_google(text: &str) -> Result<()> {
    for chunk in split_chunks(text, 200) {
        let encoded = urlencoding::encode(&chunk);
        let url = format!(
            "https://translate.google.com/translate_tts?ie=UTF-8&q={}&tl=it&client=tw-ob",
            encoded
        );
        let response = ureq::get(&url)
            .set("User-Agent", "Mozilla/5.0")
            .call()?;
        let mut reader = response.into_reader();
        let mut audio_bytes = Vec::new();
        std::io::Read::read_to_end(&mut reader, &mut audio_bytes)?;
        let cursor = Cursor::new(audio_bytes);
        let (_stream, stream_handle) = OutputStream::try_default()?;
        let sink = Sink::try_new(&stream_handle)?;
        let source = Decoder::new(cursor)?;
        sink.append(source);
        sink.sleep_until_end();
    }
    Ok(())
}



/// TTS tramite Piper
fn speak_piper(text: &str) -> Result<()> {
    let wav_path = std::env::temp_dir().join("assistente_tts_output.wav");

    let mut child = std::process::Command::new(piper_bin())
        .arg("--model").arg(piper_model())
        .arg("--output_file").arg(&wav_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .spawn()?;

    use std::io::Write;
    child.stdin.as_ref().unwrap().write_all(text.as_bytes())?;
    drop(child.stdin.take());

    let status = child.wait()?;

    if !status.success() {
        anyhow::bail!("Piper TTS terminato con errore: {}", status);
    }

    let file = std::fs::File::open(&wav_path)?;
    let (_stream, stream_handle) = OutputStream::try_default()?;
    let sink = Sink::try_new(&stream_handle)?;
    let source = Decoder::new(std::io::BufReader::new(file))?;

    sink.append(source);
    sink.sleep_until_end();

    let _ = std::fs::remove_file(&wav_path);
    Ok(())
}


/// Suddivide il testo in chunk da max `max_len` caratteri, spezzando su spazi
/// (necessario solo per Google, che ha un limite ~200 caratteri per richiesta)
fn split_chunks(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && current.len() + 1 + word.len() > max_len {
            chunks.push(current.trim().to_string());
            current = String::new();
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.trim().is_empty() {
        chunks.push(current.trim().to_string());
    }
    chunks
}
