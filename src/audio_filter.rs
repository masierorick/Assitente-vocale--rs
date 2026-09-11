use std::process::Command;
use std::sync::{Mutex, OnceLock};

use cpal::traits::{DeviceTrait, HostTrait};

static RADIO_REFERENCE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
static AEC_DEVICE: OnceLock<Mutex<Option<cpal::Device>>> = OnceLock::new();

fn radio_reference_cell() -> &'static Mutex<Option<String>> {
    RADIO_REFERENCE.get_or_init(|| Mutex::new(None))
}

fn aec_device_cell() -> &'static Mutex<Option<cpal::Device>> {
    AEC_DEVICE.get_or_init(|| Mutex::new(None))
}

// ==========================
// VERIFICA AEC ESISTENTE
// ==========================

fn aec_exists() -> bool {
    if std::env::consts::OS != "linux" {
        return false;
    }

    let output = Command::new("pactl")
        .args(["list", "short", "sources"])
        .output();

    match output {
        Ok(out) => String::from_utf8_lossy(&out.stdout).contains("assistente_aec"),
        Err(_) => false,
    }
}

// ==========================
// INIT GENERICO PER PIATTAFORMA
// ==========================

pub fn init_audio_filter() {
    match std::env::consts::OS {
        "linux" => init_linux_aec(),
        "windows" => init_windows_aec(),
        "macos" => init_macos_aec(),
        _ => {}
    }
}

// ==========================
// LINUX PIPEWIRE AEC
// ==========================

fn run_pactl(args: &[&str]) {
    let _ = Command::new("pactl")
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

fn init_linux_aec() {
    if !aec_exists() {
        run_pactl(&[
            "load-module",
            "module-echo-cancel",
            "aec_method=webrtc",
            "source_name=assistente_aec",
            "sink_name=assistente_aec_sink",
        ]);
    }

    run_pactl(&["set-default-source", "assistente_aec"]);

    // Instrada TUTTE le app di sistema attraverso il sink AEC,
    // non solo la radio: cosi' qualsiasi audio in uscita (browser,
    // player video, RaiPlay, ecc.) viene usato come riferimento e
    // cancellato dal microfono.
    run_pactl(&["set-default-sink", "assistente_aec_sink"]);

    // Cattura anche le app audio gia' aperte prima dell'avvio
    // dell'assistente (es. browser con RaiPlay gia' in riproduzione)
    move_existing_streams_to_aec();

    // Enumerazione fatta UNA SOLA VOLTA qui (non ad ogni ascolto):
    // trova e mette in cache il device "pulse", che e' quello che
    // rispetta il default-source appena impostato sopra.
    let device = find_input_device_by_name("pulse");

    match &device {
        Some(d) => {
            let nome = d.name().unwrap_or_else(|_| "?".to_string());
            println!("🎤 Linux AEC attivo: {}", nome);
        }
        None => println!("AEC Linux non disponibile: nessun device 'pulse' trovato, uso default di sistema"),
    }

    if let Ok(mut cell) = aec_device_cell().lock() {
        *cell = device;
    }
}

/// Sposta tutte le applicazioni audio gia' in riproduzione (es. browser con
/// RaiPlay gia' aperto) sul sink AEC, cosi' vengono usate come riferimento
/// e cancellate dal microfono anche se erano partite prima dell'assistente.
fn move_existing_streams_to_aec() {
    let output = Command::new("pactl")
        .args(["list", "sink-inputs", "short"])
        .output();

    let Ok(out) = output else {
        eprintln!("Impossibile spostare gli stream esistenti sull'AEC");
        return;
    };

    let stdout = String::from_utf8_lossy(&out.stdout);

    for riga in stdout.lines() {
        let riga = riga.trim();
        if riga.is_empty() {
            continue;
        }

        let Some(stream_id) = riga.split_whitespace().next() else {
            continue;
        };

        run_pactl(&["move-sink-input", stream_id, "assistente_aec_sink"]);
    }
}

// ==========================
// WINDOWS
// ==========================

fn init_windows_aec() {
    println!("🎤 Windows AEC WebRTC");
    // per ora ritorna microfono standard
    // il filtro verrà inserito nel flusso PCM
}

// ==========================
// MACOS
// ==========================

fn init_macos_aec() {
    println!("🎤 macOS AEC WebRTC");
    // per ora ritorna microfono standard
}

// ==========================
// TROVA MICROFONO PER NOME (chiamata una sola volta, all'avvio)
// ==========================

fn find_input_device_by_name(nome: &str) -> Option<cpal::Device> {
    let host = cpal::default_host();
    let devices = host.input_devices().ok()?;

    for device in devices {
        if let Ok(name) = device.name() {
            if name.to_lowercase().contains(&nome.to_lowercase()) {
                return Some(device);
            }
        }
    }

    None
}

// ==========================
// SORGENTE AUDIO PER SISTEMA
// ==========================

pub fn get_audio_source() -> Option<cpal::Device> {
    match std::env::consts::OS {
        "linux" => get_linux_microphone(),
        "windows" => get_windows_microphone(),
        "macos" => get_macos_microphone(),
        _ => None,
    }
}

fn get_linux_microphone() -> Option<cpal::Device> {
    // Device "pulse" gia' risolto UNA VOLTA in init_linux_aec() e messo
    // in cache: nessuna enumerazione ripetuta ad ogni ascolto (niente
    // warning ALSA jack/oss), e si continua a usare il device corretto
    // invece del default ALSA di sistema, che puo' non coincidere.
    if let Ok(cell) = aec_device_cell().lock() {
        if let Some(device) = cell.as_ref() {
            return Some(device.clone());
        }
    }

    // Fallback se l'AEC non e' stato inizializzato o "pulse" non e' stato trovato
    cpal::default_host().default_input_device()
}

fn get_windows_microphone() -> Option<cpal::Device> {
    // futuro backend WASAPI + AEC
    cpal::default_host().default_input_device()
}

fn get_macos_microphone() -> Option<cpal::Device> {
    // futuro backend CoreAudio + AEC
    cpal::default_host().default_input_device()
}

// ==========================
// RIFERIMENTO RADIO AEC
// ==========================

pub fn set_radio_reference(stream: Option<String>) {
    if let Ok(mut cell) = radio_reference_cell().lock() {
        *cell = stream;
    }
}

pub fn get_radio_reference() -> Option<String> {
    radio_reference_cell().lock().ok().and_then(|g| g.clone())
}
