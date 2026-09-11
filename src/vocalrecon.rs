use anyhow::Result;
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{SampleFormat, SupportedStreamConfig};
use hound::{WavSpec, WavWriter};
use std::net::{TcpStream, ToSocketAddrs};
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel,Sender};
use std::sync::{OnceLock, Mutex, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, Instant};
use whisper_rs::{WhisperContext, WhisperContextParameters, FullParams, SamplingStrategy};
use crate::tts::is_speaking;
use crate::audio_filter;
use crate::config;
use reqwest::blocking::multipart;
use serde::Deserialize;

/// Legge una stringa da messages_it.json (sezione "other"/"error"), con
/// fallback diagnostico se la chiave manca. Evita di ripetere
/// crate::config::load_messages() ad ogni call site.
fn msg(sezione: &str, chiave: &str) -> String {
    let sezione_json = if sezione == "error" { "error_messages" } else { "other_messages" };
    crate::config::load_messages()
    .ok()
    .and_then(|m| {
        let v = if sezione == "error" { m.error_messages } else { m.other_messages };
        v[chiave].as_str().map(|s| s.to_string())
    })
    .unwrap_or_else(|| format!("[missing: {}.{}]", sezione_json, chiave))
}


// ─── Parametri rilevamento silenzio ────────────────────────────────────────
const MAX_DURATION_SECS: f32 = 10.0;
const POST_SPEECH_COOLDOWN_MS: u64 = 600;
const CONFIRMATION_POST_SPEECH_COOLDOWN_MS: u64 = 900;
// Durante il cooldown la soglia di innesco frase viene moltiplicata per questo
// fattore invece di essere ignorata del tutto, per non perdere una parola
// breve detta interamente dentro la finestra di cooldown (vedi listen_for_command).
const COOLDOWN_THRESHOLD_MULTIPLIER: f32 = 1.8;

// ─── Timeout connessione ──────────────────────────────────────────────────
const CONNECTIVITY_CHECK_TIMEOUT_MS: u64 = 1500;
// Delay prima del secondo tentativo di check_connectivity(), per assorbire
// blip di rete momentanei senza switchare inutilmente a whisper.
const CONNECTIVITY_RETRY_DELAY_MS: u64 = 400;

// ─── Modalità STT ─────────────────────────────────────────────────────────
static USE_STT_ONLINE: AtomicBool = AtomicBool::new(false);
static USE_GROQ_STT: AtomicBool = AtomicBool::new(false);

// ─── Soglia rumore ambientale ─────────────────────────────────────────────
static NOISE_THRESHOLD: OnceLock<f32> = OnceLock::new();

// ─── Modalità conferma sì/no ───────────────────────────────────────────────
static AWAITING_CONFIRMATION: AtomicBool = AtomicBool::new(false);

/// Attivata da intent.rs quando è in attesa di conferma spegnimento/riavvio.
/// Abbassa la soglia minima di durata frase e restringe il riconoscimento
/// a "sì"/"no", perché una singola parola è più corta della soglia normale.
pub fn set_awaiting_confirmation(v: bool) {
    AWAITING_CONFIRMATION.store(v, Ordering::Relaxed);
}

fn awaiting_confirmation() -> bool {
    AWAITING_CONFIRMATION.load(Ordering::Relaxed)
}

const CONFIRMATION_MIN_PHRASE_SECS: f32 = 0.25;
const CONFIRMATION_PAUSE_SECS: f32 = 0.8;


// ─── Parametri VAD GOOGLE ─────────────────────────────────────────────────
// Non più usati nel flusso attivo (vedi GROQ_* sotto), tenuti per il
// ripristino del ramo Google commentato in listen_for_command().
#[allow(dead_code)]
const GOOGLE_PAUSE_SECS: f32 = 1.2;
#[allow(dead_code)]
const GOOGLE_CALIBRATION_SECS: f32 = 0.6;
#[allow(dead_code)]
const GOOGLE_THRESHOLD_FACTOR: f32 = 2.0;
#[allow(dead_code)]
const GOOGLE_DYNAMIC_ENERGY_RATIO: f32 = 1.5;
#[allow(dead_code)]
const GOOGLE_DYNAMIC_DAMPING: f32 = 0.15;
#[allow(dead_code)]
const GOOGLE_MIN_PHRASE_SECS: f32 = 0.5;
#[allow(dead_code)]
const GOOGLE_PREROLL_CHUNKS: usize = 3;


// ─── Parametri VAD GROQ ─────────────────────────────────────────────────
// Partono identici a quelli Google; tarabili indipendentemente se Groq
// si comporta diversamente (latenza, lunghezza frasi, ecc).
#[allow(dead_code)]
const GROQ_PAUSE_SECS: f32 = 1.2;
#[allow(dead_code)]
const GROQ_CALIBRATION_SECS: f32 = 0.6;
#[allow(dead_code)]
const GROQ_THRESHOLD_FACTOR: f32 = 2.0;
#[allow(dead_code)]
const GROQ_DYNAMIC_ENERGY_RATIO: f32 = 1.5;
#[allow(dead_code)]
const GROQ_DYNAMIC_DAMPING: f32 = 0.15;
#[allow(dead_code)]
const GROQ_MIN_PHRASE_SECS: f32 = 0.5;
#[allow(dead_code)]
const GROQ_PREROLL_CHUNKS: usize = 3;

// ─── Parametri VAD WHISPER ─────────────────────────────────────────────────
// Per ora PARTONO uguali a Google.
// Li renderemo più severi successivamente contro le allucinazioni.
const WHISPER_PAUSE_SECS: f32 = 1.2;
const WHISPER_CALIBRATION_SECS: f32 = 0.6;
const WHISPER_THRESHOLD_FACTOR: f32 = 2.5;
const WHISPER_DYNAMIC_ENERGY_RATIO: f32 = 1.5;
const WHISPER_DYNAMIC_DAMPING: f32 = 0.15;
const WHISPER_MIN_PHRASE_SECS: f32 = 1.0;
const WHISPER_PREROLL_CHUNKS: usize = 3;


fn vad_params() -> (
    f32, // pause
    f32, // calibration
    f32, // threshold factor
    f32, // dynamic energy ratio
    f32, // dynamic damping
    f32, // min phrase
    usize, // preroll
) {
    if use_stt_online() {
        (
         /*GROQ_PAUSE_SECS,
         GROQ_CALIBRATION_SECS,
         GROQ_THRESHOLD_FACTOR,
         GROQ_DYNAMIC_ENERGY_RATIO,
         GROQ_DYNAMIC_DAMPING,
         GROQ_MIN_PHRASE_SECS,
         GROQ_PREROLL_CHUNKS,*/
         GOOGLE_PAUSE_SECS,
         GOOGLE_CALIBRATION_SECS,
         GOOGLE_THRESHOLD_FACTOR,
         GOOGLE_DYNAMIC_ENERGY_RATIO,
         GOOGLE_DYNAMIC_DAMPING,
         GOOGLE_MIN_PHRASE_SECS,
         GOOGLE_PREROLL_CHUNKS,
        )
    } else {
        (
            WHISPER_PAUSE_SECS,
         WHISPER_CALIBRATION_SECS,
         WHISPER_THRESHOLD_FACTOR,
         WHISPER_DYNAMIC_ENERGY_RATIO,
         WHISPER_DYNAMIC_DAMPING,
         WHISPER_MIN_PHRASE_SECS,
         WHISPER_PREROLL_CHUNKS,
        )
    }
}


fn has_vulkan_gpu() -> bool {
    let output = std::process::Command::new("vulkaninfo")
    .arg("--summary")
    .output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_lowercase();
            let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();

            let text = format!("{}\n{}", stdout, stderr);

            text.contains("amd radeon")
            || text.contains("radeon rx")
            || text.contains("nvidia")
            || text.contains("intel")
        }

        _ => false,
    }
}

fn total_ram_gb() -> u64 {
    let meminfo = match std::fs::read_to_string("/proc/meminfo") {
        Ok(v) => v,
        Err(_) => return 0,
    };

    for line in meminfo.lines() {
        if let Some(value) = line.strip_prefix("MemTotal:") {
            if let Some(kb) = value.trim().split_whitespace().next() {
                if let Ok(kb) = kb.parse::<u64>() {
                    return kb / 1024 / 1024;
                }
            }
        }
    }

    0
}



/// Priorità: config.json (whisper_model) > env var ASSISTENTE_WHISPER_MODEL > default per GPU/CPU.
pub fn select_whisper_model(config: &config::Config) -> String {
    if let Some(path) = &config.whisper_model {
        return path.clone();
    }

    if let Ok(path) = std::env::var("ASSISTENTE_WHISPER_MODEL") {
        return path;
    }

    let ram_gb = total_ram_gb();
    let gpu = has_vulkan_gpu();

    println!("🖥️ {}", msg("other", "hardware_rilevato")
    .replace("{ram}", &ram_gb.to_string())
    .replace("{gpu}", &gpu.to_string()));

    if gpu && ram_gb >= 16 {
        println!("🧠 {}", msg("other", "whisper_medium_gpu"));
        "models/ggml-medium.bin".to_string()
    } else {
        println!("🧠 {}", msg("other", "whisper_small_cpu"));
        "models/ggml-small.bin".to_string()
    }
}


/// Verifica la raggiungibilità di Groq (DNS + TCP:443), con un retry prima
/// di dichiarare offline, per assorbire blip di rete momentanei che prima
/// causavano un flip-flop whisper→groq nel giro di un ciclo di
/// start_connection_monitor() (2s).
fn check_connectivity() -> bool {
    for attempt in 0..2 {
        let reachable = "api.groq.com:443"
        .to_socket_addrs()
        .ok()
        .and_then(|mut addrs| addrs.next())
        .map(|addr| {
            TcpStream::connect_timeout(&addr, Duration::from_millis(CONNECTIVITY_CHECK_TIMEOUT_MS))
            .is_ok()
        })
        .unwrap_or(false);

        if reachable || attempt == 1 {
            return reachable;
        }

        std::thread::sleep(Duration::from_millis(CONNECTIVITY_RETRY_DELAY_MS));
    }

    false
}

/// Determina se usare Groq Google o whisper, controllando la connessione una sola volta.
/// Chiamalo una volta all'avvio, prima di warmup_whisper().
/// Etichetta del motore STT attualmente in uso, calcolata al volo dallo
/// stato reale (non da un valore fissato una tantum all'avvio).
pub fn stt_engine_label() -> String {
    if !use_stt_online() {
        msg("other", "stt_mode_whisper_label")
    } else if USE_GROQ_STT.load(Ordering::Relaxed) {
        msg("other", "stt_mode_groq_label")
    } else {
        msg("other", "stt_mode_google_label")
    }
}


pub fn detect_stt_mode() -> String {
    let online = check_connectivity();

    USE_STT_ONLINE.store(online, Ordering::Relaxed);

    if online {
        if USE_GROQ_STT.load(Ordering::Relaxed) {
            println!("📡 {}", msg("other", "stt_mode_groq_log"));
        } else {
            println!("📡 {}", msg("other", "stt_mode_google_log"));
        }
    } else {
        println!("📴 {}", msg("other", "stt_mode_whisper_log"));
    }

    stt_engine_label()
}

/// Espone lo stato online/offline già rilevato, per essere riusato da altri moduli (es. tts.rs)
pub fn use_stt_online() -> bool {
    USE_STT_ONLINE.load(Ordering::Relaxed)
}

// ─── Whisper: contesto globale caricato una sola volta ─────────────────────
static WHISPER_CTX: OnceLock<Mutex<Option<WhisperContext>>> = OnceLock::new();

fn whisper_ctx_cell() -> &'static Mutex<Option<WhisperContext>> {
    WHISPER_CTX.get_or_init(|| Mutex::new(None))
}

fn model_path() -> String {
    std::env::var("ASSISTENTE_WHISPER_MODEL")
    .unwrap_or_else(|_| "models/ggml-medium.bin".to_string())
}

fn n_threads() -> i32 {
    std::env::var("ASSISTENTE_WHISPER_THREADS")
    .ok()
    .and_then(|v| v.parse().ok())
    .unwrap_or(4)
}

/// Carica il modello whisper in RAM se non è già caricato. Idempotente.
fn ensure_whisper_loaded() {
    let mut guard = whisper_ctx_cell().lock().unwrap();
    if guard.is_some() {
        return;
    }

    let path = model_path();

    let ram_gb = total_ram_gb();
    let use_gpu = has_vulkan_gpu() && ram_gb >= 16;

    println!("🔧 {}", msg("other", "whisper_model_loading").replace("{path}", &path));

    let mut params = WhisperContextParameters::default();
    params.use_gpu(use_gpu);

    let backend = if use_gpu {
        msg("other", "whisper_accel_gpu")
    } else {
        msg("other", "whisper_accel_cpu")
    };
    println!("⚙️ {}", msg("other", "whisper_backend_log").replace("{backend}", &backend));

    let ctx = WhisperContext::new_with_params(&path, params)
    .unwrap_or_else(|e| {
        panic!("{}", msg("error", "whisper_model_load_error")
        .replace("{path}", &path)
        .replace("{e}", &e.to_string()))
    });

    *guard = Some(ctx);
}

/// Scarica il modello whisper dalla RAM (~1.5GB liberati), se caricato.
/// Chiamalo quando la connessione torna stabile e non serve più il fallback offline.
pub fn unload_whisper(tx_output: &Sender<String>) {
    let mut guard = whisper_ctx_cell().lock().unwrap();
    if guard.take().is_some() {
        let msg_testo = "🧹 Whisper scaricato dalla RAM (connessione stabile)".to_string();
        println!("{}", msg_testo);
        let _ = tx_output.send(msg_testo);
    }
}

pub fn start_connection_monitor(tx_output: Sender<String>) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(2)); //esegue ogni 2 secondi per vedere la connessione

            let online = check_connectivity();

            let previous = USE_STT_ONLINE.swap(online, Ordering::Relaxed);

            if online != previous {
                if online {
                    let msg_testo = format!("🌐 {}", msg("other", "connection_restored"));
                    println!("{}", msg_testo);
                    let _ = tx_output.send(msg_testo);
                    // Connessione tornata stabile: non serve più whisper come
                    // fallback, libera la RAM se era stato caricato.
                    unload_whisper(&tx_output);
                } else {
                    let msg_testo = format!("📴 {}", msg("other", "connection_lost"));
                    println!("{}", msg_testo);
                    let _ = tx_output.send(msg_testo);
                }
                let _ = tx_output.send(format!("🎙️ Riconoscimento vocale: {}", stt_engine_label()));
            }
        }
    });
}



/// Precarica il modello whisper in RAM. Chiamalo solo se la modalità è offline
/// (evita di caricare ~1.5GB in RAM inutilmente quando si usa Groq).
pub fn warmup_whisper() {
    ensure_whisper_loaded();
}


/// Calibra il microfono registrando il rumore ambiente. Chiamalo una volta all'avvio.
pub fn warmup_audio() {
    std::env::set_var("CPAL_ALSA_BACKEND", "pulse");

    if let Some(device) = audio_filter::get_audio_source() {
        if let Ok(config) = device.default_input_config() {

            let (
                _pause_secs,
                 calibration_secs,
                 threshold_factor,
                 _dynamic_energy_ratio,
                 _dynamic_damping,
                 _min_phrase_secs,
                 _preroll_chunks,
            ) = vad_params();

            let t = calibrate_threshold(
                &device,
                &config,
                calibration_secs,
                threshold_factor,
            );

            NOISE_THRESHOLD.set(t).ok();

            // println!("🎚️ Soglia rumore calibrata: {:.4}", t);
        }
    }
}

fn get_threshold() -> f32 {
    *NOISE_THRESHOLD.get().unwrap_or(&0.01)
}

/// Ascolta il microfono, rileva una frase completa e la trascrive con il motore
/// deciso una sola volta all'avvio .
pub fn listen_for_command(_groq_api_key: &str, _tx_output: &Sender<String>) -> Result<String> {
    // Aspetta che il TTS finisca di parlare, ma NON usciamo qui con un semplice
    // sleep: apriamo subito lo stream e ignoriamo solo l'energia rilevata durante
    // la finestra di cooldown (echo/coda del TTS), tenendola comunque nel preroll.
    // Così se l'utente inizia a parlare presto, il preroll cattura comunque
    // l'inizio della parola invece di perderlo.
    while is_speaking() {
        std::thread::sleep(Duration::from_millis(50));
    }

    let device = audio_filter::get_audio_source()
    .ok_or_else(|| anyhow::anyhow!(msg("error", "no_microphone")))?;

    let supported_config = device.default_input_config()?;
    let sample_rate = supported_config.sample_rate().0;
    let channels = supported_config.channels() as usize;

    let (
        pause_secs,
         _,
         _,
         dynamic_energy_ratio,
         dynamic_damping,
         min_phrase_secs,
         preroll_chunks,
    ) = vad_params();

    // In attesa di conferma: "sì"/"no" sono più corti della soglia normale,
    // e vogliamo anche meno silenzio di coda registrato dopo la parola,
    // per alzare il rapporto parlato/silenzio nella clip inviata a Google/Whisper.
    let (min_phrase_secs, pause_secs, cooldown_ms) = if awaiting_confirmation() {
        (CONFIRMATION_MIN_PHRASE_SECS, CONFIRMATION_PAUSE_SECS, CONFIRMATION_POST_SPEECH_COOLDOWN_MS)
    } else {
        (min_phrase_secs, pause_secs, POST_SPEECH_COOLDOWN_MS)
    };

    // Il preroll circolare deve coprire almeno tutta la finestra di cooldown,
    // altrimenti un "sì" detto durante il cooldown verrebbe comunque perso
    // (i chunk più vecchi del preroll vengono scartati man mano che arrivano nuovi).
    let cooldown_chunks = ((cooldown_ms as f32) / 100.0).ceil() as usize + 1;
    let preroll_chunks = preroll_chunks.max(cooldown_chunks);

    let ignore_until = Instant::now() + Duration::from_millis(cooldown_ms);

    let mut threshold = get_threshold();

    let (tx_audio, rx_audio) = channel::<Vec<f32>>();
    let stream = build_stream(&device, &supported_config, tx_audio, channels)?;
    stream.play()?;

    let chunk_samples = (sample_rate as f32 * 0.1) as usize * channels;
    let pause_chunks = (pause_secs / 0.1) as usize;
    let max_chunks = (MAX_DURATION_SECS / 0.1) as usize;

    let mut all_samples: Vec<f32> = Vec::new();
    let mut phrase_started = false;
    let mut silent_chunks = 0usize;
    let mut total_chunks = 0usize;
    let mut chunk_buf: Vec<f32> = Vec::new();
    let mut preroll: std::collections::VecDeque<Vec<f32>> =
    std::collections::VecDeque::with_capacity(preroll_chunks);

    'outer: loop {
        if is_speaking() {
            all_samples.clear();
            phrase_started = false;
            break 'outer;
        }

        let deadline = Instant::now() + Duration::from_millis(100);
        while chunk_buf.len() < chunk_samples {
            if Instant::now() > deadline {
                break;
            }
            if let Ok(mut samples) = rx_audio.recv_timeout(Duration::from_millis(10)) {
                chunk_buf.append(&mut samples);
            }
        }

        if chunk_buf.is_empty() {
            total_chunks += 1;
            if total_chunks >= max_chunks { break; }
            continue;
        }

        let chunk: Vec<f32> = chunk_buf.drain(..chunk_samples.min(chunk_buf.len())).collect();
        let mono: Vec<f32> = chunk
        .chunks(channels)
        .map(|c| c.iter().sum::<f32>() / c.len() as f32)
        .collect();
        let rms = (mono.iter().map(|s| s * s).sum::<f32>() / mono.len() as f32).sqrt();

        // Durante la finestra di cooldown NON ignoriamo del tutto l'energia:
        // se una parola breve ("sì") viene detta e finisce interamente dentro
        // il cooldown, ignorarla del tutto la farebbe perdere per intero (il
        // volume torna al silenzio prima che il cooldown scada, quindi la
        // soglia normale non scatterebbe mai dopo). Alziamo invece la soglia:
        // l'eco residua del TTS è più debole del parlato reale, quindi una
        // soglia più alta filtra l'eco ma lascia comunque passare una parola
        // detta ad alta voce subito dopo il prompt.
        let in_cooldown = Instant::now() < ignore_until;
        let effective_threshold = if in_cooldown {
            threshold * COOLDOWN_THRESHOLD_MULTIPLIER
        } else {
            threshold
        };

        // Soglia dinamica: si adatta solo prima che inizi il parlato, per non
        // "inseguire" la voce stessa e alzarsi durante la frase.
        if !phrase_started && !in_cooldown {
            let target = rms * dynamic_energy_ratio;
            threshold = threshold * (1.0 - dynamic_damping) + target * dynamic_damping;
            threshold = threshold.max(0.005).min(0.05);
        }

        if rms > effective_threshold {
            if !phrase_started {
                // Primo superamento soglia: prependi l'audio silenzioso già bufferizzato,
                // così eventuali attacchi deboli (consonanti sorde) non vengono persi,
                // incluso quanto detto durante il cooldown.
                for buffered in preroll.drain(..) {
                    all_samples.extend(&buffered);
                }
            }
            phrase_started = true;
            silent_chunks = 0;
            all_samples.extend(&mono);
        } else if phrase_started {
            all_samples.extend(&mono);
            silent_chunks += 1;
            if silent_chunks >= pause_chunks {
                break 'outer;
            }
        } else {
            // Ancora silenzio (o in cooldown): alimenta il pre-buffer circolare.
            preroll.push_back(mono);
            if preroll.len() > preroll_chunks {
                preroll.pop_front();
            }
        }

        total_chunks += 1;
        if total_chunks >= max_chunks { break; }
    }

    drop(stream);

    if all_samples.is_empty() || !phrase_started {
        return Ok(String::new());
    }

    let duration_secs = all_samples.len() as f32 / sample_rate as f32;
    if duration_secs < min_phrase_secs {
        return Ok(String::new());
    }

    // ── Versione originale (Google STT), commentata per ripristino futuro ──
    // NB: se ripristini questo ramo, riporta anche vad_params() sopra a
    // usare le costanti GOOGLE_* invece di GROQ_*.
     if use_stt_online() {
       match transcribe_google(&all_samples, sample_rate) {
        Ok(text) => Ok(text),
        Err(e) => {
           eprintln!("⚠️ Google STT non disponibile: {}", e);
           USE_STT_ONLINE.store(false, Ordering::Relaxed);
           println!("📴 Google STT non raggiungibile → uso Whisper offline");
           transcribe_whisper(&all_samples, sample_rate)
        }
       }
     } else {
         transcribe_whisper(&all_samples, sample_rate)
     }

    /*if use_stt_online() {
        match transcribe_groq(&all_samples, sample_rate, groq_api_key) {
            Ok(text) => Ok(text), // stringa vuota = nessun parlato rilevato, non un errore: nessun fallback
            Err(e) => {
                eprintln!("⚠️ Groq STT non disponibile: {}", e);
                USE_STT_ONLINE.store(false, Ordering::Relaxed);
                println!("📴 Groq non raggiungibile → uso Whisper offline");
                let _ = tx_output.send(format!("🎙️ Riconoscimento vocale: {}", stt_engine_label()));
                transcribe_whisper(&all_samples, sample_rate)
            }
        }
    } else {
        transcribe_whisper(&all_samples, sample_rate)
    }*/
}

/// Trascrive tramite Google Speech-to-Text.
/// Tenuta per il momento come fallback/riferimento, non più usata nel flusso online.
#[allow(dead_code)]
fn transcribe_google(samples: &[f32], sample_rate: u32) -> Result<String> {
    let wav_path = std::env::temp_dir().join("assistente_stt_input.wav");
    let flac_path = std::env::temp_dir().join("assistente_stt_input.flac");

    {
        let spec = WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = WavWriter::create(&wav_path, spec)?;
        for sample in samples {
            let s = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            writer.write_sample(s)?;
        }
        writer.finalize()?;
    }

    let status = Command::new("ffmpeg")
    .args([
        "-loglevel", "quiet",
        "-y",
        "-i", wav_path.to_str().unwrap(),
          "-ar", "16000",
          "-ac", "1",
          "-f", "flac",
          flac_path.to_str().unwrap(),
    ])
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status()?;

    if !status.success() {
        anyhow::bail!(msg("error", "flac_conversion_error"));
    }

    let api_key = std::env::var("GOOGLE_STT_KEY")
    .unwrap_or_else(|_| "AIzaSyBOti4mM-6x9WDnZIjIeyEU21OpBXqWBgw".to_string());

    let url = format!(
        "https://www.google.com/speech-api/v2/recognize?client=chromium&lang=it-IT,en-US&key={}",
        api_key
    );

    let audio_data = std::fs::read(&flac_path)?;

    let response = ureq::post(&url)
    .timeout(Duration::from_secs(5))
    .set("Content-Type", "audio/x-flac; rate=16000")
    .send_bytes(&audio_data)?;

    let body = response.into_string()?;

    let _ = std::fs::remove_file(&wav_path);
    let _ = std::fs::remove_file(&flac_path);

    for line in body.lines() {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(transcript) = json["result"][0]["alternative"][0]["transcript"].as_str() {
                let t = transcript.trim().to_lowercase();
                if !t.is_empty() {
                    return Ok(t);
                }
            }
        }
    }

    Ok(String::new())
}




#[derive(Deserialize)]
struct TranscriptionSegment {
    text: String,
    avg_logprob: f32,
    compression_ratio: f32,
    no_speech_prob: f32,
}

#[derive(Deserialize)]
struct TranscriptionResponse {
    text: String,
    segments: Option<Vec<TranscriptionSegment>>,
}

// Guida il modello (Groq e whisper.cpp) verso i nomi/comandi che vengono
// trascritti male, così da ridurre gli errori a monte invece di correggerli
// dopo con adatta_lingua() in intent.rs. Non è garantito al 100%:
// adatta_lingua resta come rete di sicurezza per i casi che il prompt non
// risolve.
const STT_VOCAB_PROMPT: &str =
    "Comandi vocali in italiano. \
     Parole possibili: mitology, krita, konsole, kaffeine, kate, dolphin, \
     creami, crea, creare, cartella, directory, elimina, cancella, \
     spegni il computer, spegni, spegnere. \
     Trascrivi soltanto ciò che viene effettivamente pronunciato. \
     Non inventare parole o comandi quando l'audio contiene solo rumore, respiri o schiarimenti di voce.";

/// Trascrive tramite Groq (Whisper large-v3-turbo). Sostituisce Google
/// come motore online principale in listen_for_command().
/// Trascrive tramite Groq (Whisper large-v3-turbo).
/// Usa verbose_json per poter scartare audio che Whisper considera
/// probabilmente non parlato, come respiri, rumori e schiarimenti di voce.
#[allow(dead_code)]
fn transcribe_groq(samples: &[f32], sample_rate: u32, api_key: &str) -> Result<String> {
    let wav_path = std::env::temp_dir().join("assistente_stt_groq.wav");

    {
        let spec = WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let mut writer = WavWriter::create(&wav_path, spec)?;

        for sample in samples {
            let s = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            writer.write_sample(s)?;
        }

        writer.finalize()?;
    }

    if api_key.is_empty() {
        let _ = std::fs::remove_file(&wav_path);
        return Err(anyhow::anyhow!("API_KEY_GROQ non impostata (config/.env)"));
    }

    let form = multipart::Form::new()
        .file("file", &wav_path)?
        .text("model", "whisper-large-v3-turbo")
        .text("language", "it")
        .text("prompt", STT_VOCAB_PROMPT)
        .text("response_format", "verbose_json")
        .text("temperature", "0");

    let client = reqwest::blocking::Client::new();

    let response = client
        .post("https://api.groq.com/openai/v1/audio/transcriptions")
        .bearer_auth(api_key)
        .multipart(form)
        .timeout(Duration::from_secs(10))
        .send()?
        .error_for_status()?;

    let parsed: TranscriptionResponse = response.json()?;

    let _ = std::fs::remove_file(&wav_path);

    // Se Groq non ha restituito segmenti, consideriamo comunque il testo
    // ma passiamo attraverso il filtro anti-non-parlato.
    let Some(segments) = parsed.segments else {
        let result = parsed.text.trim().to_lowercase();

        if is_non_speech_output(&result) {
            return Ok(String::new());
        }

        return Ok(result);
    };

    if segments.is_empty() {
        return Ok(String::new());
    }

    // Accettiamo solamente segmenti con caratteristiche compatibili
    // con parlato reale.
    //
    // no_speech_prob:
    //   più vicino a 1 = Whisper ritiene probabile che NON ci sia parlato.
    //
    // avg_logprob:
    //   valori molto negativi = trascrizione poco affidabile.
    //
    // compression_ratio:
    //   valori anomali possono indicare allucinazioni/ripetizioni.
    const MAX_NO_SPEECH_PROB: f32 = 0.55;
    const MIN_AVG_LOGPROB: f32 = -1.0;
    const MAX_COMPRESSION_RATIO: f32 = 2.8;

    let mut accepted_text = String::new();
    let mut accepted_segments = 0usize;

    for segment in segments {

        let valid = segment.no_speech_prob <= MAX_NO_SPEECH_PROB
            && segment.avg_logprob >= MIN_AVG_LOGPROB
            && segment.compression_ratio <= MAX_COMPRESSION_RATIO;

        if valid {
            accepted_text.push_str(segment.text.trim());
            accepted_text.push(' ');
            accepted_segments += 1;
        }
    }

    // Nessun segmento sufficientemente affidabile:
    // NON deve arrivare né a intent.rs né all'AI Agent.
    if accepted_segments == 0 {
         return Ok(String::new());
    }

    let result = accepted_text.trim().to_lowercase();

    if result.is_empty() || is_non_speech_output(&result) {
        return Ok(String::new());
    }

    Ok(result)
}

/// Trascrive campioni f32 mono con whisper.cpp locale. Richiede 16kHz.
fn transcribe_whisper(samples: &[f32], sample_rate: u32) -> Result<String> {
    if samples.is_empty() {
        return Ok(String::new());
    }

    let resampled = if sample_rate != 16000 {
        resample_linear(samples, sample_rate, 16000)
    } else {
        samples.to_vec()
    };

    ensure_whisper_loaded();
    let guard = whisper_ctx_cell().lock().unwrap();
    let ctx = guard.as_ref().expect("whisper context appena caricato da ensure_whisper_loaded");
    let mut state = ctx.create_state()?;

    let mut params = FullParams::new(
        SamplingStrategy::Greedy { best_of: 1 }
    );

    // Italiano
    params.set_language(Some("it"));
    if awaiting_confirmation() {
        params.set_initial_prompt("Sì. No.");
    } else {
        params.set_initial_prompt(STT_VOCAB_PROMPT);
    }


    //CPU / GPU
    params.set_n_threads(n_threads());

    // Output
    params.set_print_progress(false);
    params.set_print_special(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    // La frase catturata dal VAD deve essere trattata come un singolo segmento
    params.set_single_segment(true);
    params.set_suppress_nst(true);

    // Evita di trascinare il testo della trascrizione precedente
    params.set_no_context(true);

    // Anti-allucinazione: soglie severe, impostate una sola volta
    // (prima erano impostate due volte, e la seconda sovrascriveva
    // la prima con valori più permissivi, vanificando l'effetto)
    params.set_no_speech_thold(0.75);
    params.set_logprob_thold(-0.8);
    params.set_entropy_thold(2.0);

    state.full(params, &resampled)?;

    let num_segments = state.full_n_segments();

    let mut text = String::new();

    for i in 0..num_segments {
        if let Some(segment) = state.get_segment(i) {
            if let Ok(segment_text) = segment.to_str() {
                text.push_str(segment_text);
            }
        }
    }

    let result = text.trim().to_lowercase();

    if is_non_speech_output(&result) {
        return Ok(String::new());
    }

    Ok(result)
}

/// Filtra output di whisper che non rappresentano parlato reale:
/// tag come [musica], [applausi], o frasi ricorrenti da sottotitoli di training.
fn is_non_speech_output(text: &str) -> bool {
    let t = text.trim();
    if (t.starts_with('[') && t.ends_with(']')) || (t.starts_with('(') && t.ends_with(')')) {
        return true;
    }

    // Allucinazioni di Whisper relative a respiri/sospiri
    if t.contains("*sospiro*")
        || t.contains("sospiro")
        || t.contains("*respira*")
        || t.contains("respira")
        {
            return true;
        }

        // Frasi ricorrenti generate da Whisper in assenza di parlato reale
        let patterns = [
            "sottotitoli",
            "sigla",
            "qtss",
            "sperrini",
            "amara.org",
            "untertitel",
            "grazie per la visione",
            "grazie della visione",
            "grazie per aver visto",
            "thanks for watching",
        ];

        if patterns.iter().any(|p| t.contains(p)) {
            return true;
        }

        // "grazie." isolato (senza altro testo) è un'allucinazione ricorrente
        // di Whisper su respiro/rumore di fondo, non un comando reale
        let t_bare = t.trim_end_matches(['.', '!', '?', ',']).trim();
        matches!(t_bare, "grazie")
}

//Questa funzione fa resampling lineare di un segnale audio: converte un array di campioni da una frequenza di campionamento (from_rate) a un'altra (to_rate), ad esempio da 44100 Hz a 16000 Hz.

fn resample_linear(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if samples.is_empty() || from_rate == to_rate {
        return samples.to_vec();
    }
    let ratio = from_rate as f64 / to_rate as f64;
    let out_len = (samples.len() as f64 / ratio) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_pos = i as f64 * ratio;
        let idx = src_pos as usize;
        let frac = (src_pos - idx as f64) as f32;
        let a = samples.get(idx).copied().unwrap_or(0.0);
        let b = samples.get(idx + 1).copied().unwrap_or(a);
        out.push(a + (b - a) * frac);
    }
    out
}

/// Calibra il rumore ambiente e restituisce il valore RMS grezzo.
/// Il fattore della soglia viene applicato successivamente da get_threshold()
fn calibrate_threshold(
    device: &cpal::Device,
    config: &SupportedStreamConfig,
    secs: f32,
    threshold_factor: f32,
) -> f32 {
    let channels = config.channels() as usize;
    let (tx, rx) = channel::<Vec<f32>>();

    let stream = match build_stream(device, config, tx, channels) {
        Ok(s) => s,
        Err(_) => return 0.01,
    };

    let _ = stream.play();

    std::thread::sleep(Duration::from_secs_f32(secs));

    drop(stream);

    let mut all: Vec<f32> = Vec::new();

    while let Ok(samples) = rx.try_recv() {
        let mono: Vec<f32> = samples
        .chunks(channels)
        .map(|c| c.iter().sum::<f32>() / c.len() as f32)
        .collect();

        all.extend(mono);
    }

    if all.is_empty() {
        return 0.01;
    }

    let rms = (
        all.iter()
        .map(|s| s * s)
        .sum::<f32>()
        / all.len() as f32
    ).sqrt();

    let soglia_raw = rms * threshold_factor;

    soglia_raw.max(0.006).min(0.02)
}

/// Costruisce lo stream di input gestendo i diversi formati campione (f32/i16/u16)
fn build_stream(
    device: &cpal::Device,
    config: &SupportedStreamConfig,
    tx: std::sync::mpsc::Sender<Vec<f32>>,
    _channels: usize,
) -> Result<cpal::Stream> {
    let cfg = config.config();

    let stream = match config.sample_format() {
        SampleFormat::F32 => device.build_input_stream(
            &cfg,
            move |data: &[f32], _| { let _ = tx.send(data.to_vec()); },
                                                       |e| eprintln!("{}", msg("error", "audio_stream_error").replace("{e}", &e.to_string())),
                                                       None,
        )?,
        SampleFormat::I16 => {
            let tx2 = tx.clone();
            device.build_input_stream(
                &cfg,
                move |data: &[i16], _| {
                    let f: Vec<f32> = data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                    let _ = tx2.send(f);
                },
                |e| eprintln!("{}", msg("error", "audio_stream_error").replace("{e}", &e.to_string())),
                                      None,
            )?
        }
        SampleFormat::U16 => {
            let tx3 = tx.clone();
            device.build_input_stream(
                &cfg,
                move |data: &[u16], _| {
                    let f: Vec<f32> = data.iter()
                    .map(|&s| (s as f32 / u16::MAX as f32) * 2.0 - 1.0)
                    .collect();
                    let _ = tx3.send(f);
                },
                |e| eprintln!("{}", msg("error", "audio_stream_error").replace("{e}", &e.to_string())),
                                      None,
            )?
        }
        _ => anyhow::bail!(msg("error", "audio_format_unsupported")),
    };

    Ok(stream)
}
