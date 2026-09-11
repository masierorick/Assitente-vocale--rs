#![allow(non_snake_case)]

use qmetaobject::prelude::*;
use qmetaobject::{QmlEngine, QObjectBox, QString};
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};

// ─── Config dir cross-platform ────────────────────────────────────────────
//
// Determina la cartella di configurazione dell'app secondo le convenzioni
// del sistema operativo e la crea se non esiste. Non usa crate esterni:
// - Windows:   %APPDATA%\assistente-rs
// - macOS:     ~/Library/Application Support/assistente-rs
// - Linux/BSD: $XDG_CONFIG_HOME/assistente-rs oppure ~/.config/assistente-rs
//
// In caso di problemi (variabili d'ambiente mancanti, permessi, ecc.)
// ripiega sulla directory corrente, così l'app non va mai in crash per
// questo motivo: al massimo le impostazioni finestra non persistono.
fn config_dir() -> PathBuf {
    let base: Option<PathBuf> = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };

    let dir = base.unwrap_or_else(|| PathBuf::from(".")).join("assistente-rs");

    if let Err(e) = std::fs::create_dir_all(&dir) {
        let msg = crate::config::load_messages()
        .ok()
        .and_then(|m| m.error_messages["config_dir_error"].as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "[missing: error_messages.config_dir_error]".to_string())
        .replace("{dir}", &format!("{:?}", dir))
        .replace("{e}", &e.to_string());
        eprintln!("⚠️  {}", msg);
        return PathBuf::from(".");
    }

    dir
}

// Converte un path assoluto in un vero file:// URL, come richiesto dalla
// proprietà `location` (tipo url) di QtCore.Settings — passare un path
// nudo viene interpretato da QML come URL relativo al file .qml stesso,
// il che rompe silenziosamente l'inizializzazione di QSettings.

fn path_to_file_url(path: &std::path::Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    if normalized.starts_with('/') {
        format!("file://{}", normalized)
    } else {
        format!("file:///{}", normalized)
    }
}

// ─── AnimationManager ────────────────────────────────────────────────────────

#[derive(QObject, Default)]
#[allow(non_snake_case)]
pub struct AnimationManager {
    base: qt_base_class!(trait QObject),

    newOutput: qt_signal!(msg: QString),
    colorChanged: qt_signal!(color: QString),

    sendCommand: qt_method!(fn sendCommand(&mut self, command: QString) {
        let cmd = command.to_string();
        if cmd.trim().is_empty() { return; }
        if let Some(tx) = self.command_tx.lock().unwrap().as_ref() {
            let _ = tx.send(format!("__GUI__{}", cmd));
        }
    }),

    stop_process: qt_method!(fn stop_process(&mut self) {
        std::process::exit(0);
    }),

    checkColor: qt_method!(fn checkColor(&mut self) {
        let val = *self.attivo_interno.lock().unwrap();
        if self.attivo != val {
            self.attivo = val;
            self.attivo_changed();
            let color: QString = if val { "red".into() } else { "white".into() };
            self.colorChanged(color);
        }
    }),

    loadWindow: qt_method!(fn loadWindow(&mut self) {
        let path = self.config_path.lock().unwrap().clone();
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(&content) {
                let current = json["layout"].as_str().unwrap_or("uniwindow");
                let nuovo = if current == "main" { "uniwindow" } else { "main" };
                json["layout"] = serde_json::json!(nuovo);
                if let Ok(updated) = serde_json::to_string_pretty(&json) {
                    let _ = std::fs::write(&path, updated);
                }
            }
        }
        let args: Vec<String> = std::env::args().collect();
        std::process::Command::new(&args[0]).args(&args[1..]).spawn().ok();
        std::process::exit(0);
    }),

    setAlwaysOnTop: qt_method!(fn setAlwaysOnTop(&mut self, value: bool) {
        if self.alwaysOnTop != value {
            self.alwaysOnTop = value;
            self.alwaysOnTop_changed();
        }
        let path = self.config_path.lock().unwrap().clone();
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(&content) {
                json["always_on_top"] = serde_json::json!(value);
                if let Ok(updated) = serde_json::to_string_pretty(&json) {
                    let _ = std::fs::write(&path, updated);
                }
            }
        }
    }),

    attivo: qt_property!(bool; NOTIFY attivo_changed),
    attivo_changed: qt_signal!(),

    alwaysOnTop: qt_property!(bool; NOTIFY alwaysOnTop_changed),
    alwaysOnTop_changed: qt_signal!(),

    pub command_tx:    Arc<Mutex<Option<mpsc::Sender<String>>>>,
    pub attivo_interno: Arc<Mutex<bool>>,
    pub config_path:   Arc<Mutex<String>>,
}

// ─── ProcessManager ──────────────────────────────────────────────────────────

#[derive(QObject, Default)]
pub struct ProcessManager {
    base: qt_base_class!(trait QObject),

    close_window: qt_method!(fn close_window(&mut self) {}),

    check_text: qt_method!(fn check_text(&mut self, testo: QString) {
        self.testo = testo;
        self.testo_changed();
    }),

    testo: qt_property!(QString; NOTIFY testo_changed),
    testo_changed: qt_signal!(),
}

// ─── ConfigData ──────────────────────────────────────────────────────────────

#[derive(QObject, Default)]
pub struct ConfigData {
    base: qt_base_class!(trait QObject),
    botname: qt_property!(QString; CONST),
    variant: qt_property!(QString; CONST),

}

// ─── avvia_gui ───────────────────────────────────────────────────────────────

pub fn avvia_gui(
    layout: &str,
    config_path: &str,
    botname: &str,
    command_tx: mpsc::Sender<String>,
    output_rx: mpsc::Receiver<String>,
    attivo_flag: Arc<Mutex<bool>>,
) {
    let qml_file = if layout == "main" { "ui/main.qml" } else { "ui/uniwindow.qml" };

    let always_on_top = std::fs::read_to_string(config_path)
    .ok()
    .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
    .and_then(|j| j["always_on_top"].as_bool())
    .unwrap_or(false);


    let mut am = AnimationManager::default();
    am.command_tx     = Arc::new(Mutex::new(Some(command_tx)));
    am.attivo_interno = attivo_flag.clone();
    am.config_path    = Arc::new(Mutex::new(config_path.to_string()));
    am.alwaysOnTop    = always_on_top;

    let mut cd = ConfigData::default();
    cd.botname = botname.into();
    cd.variant = "rust".into();


    let am_box: &'static QObjectBox<AnimationManager> =
    Box::leak(Box::new(QObjectBox::new(am)));
    let pm_box: &'static QObjectBox<ProcessManager> =
    Box::leak(Box::new(QObjectBox::new(ProcessManager::default())));
    let cd_box: &'static QObjectBox<ConfigData> =
    Box::leak(Box::new(QObjectBox::new(cd)));

    let am_pinned = am_box.pinned();
    let pm_pinned = pm_box.pinned();
    let cd_pinned = cd_box.pinned();

    let mut engine = QmlEngine::new();

    // Cartella di configurazione cross-platform (Windows/macOS/Linux),
    // creata qui — garantita esistente prima che uniwindow.qml istanzi
    // il componente Settings, che usa questo path come fileName.
    let settings_dir = config_dir();
    let settings_path = settings_dir.join("settings.conf");
    engine.set_property(
        "settingsPath".into(),
                        QString::from(path_to_file_url(&settings_path)).into(),
    );

    engine.set_object_property("animationManager".into(), am_pinned.clone());
    engine.set_object_property("processManager".into(),   pm_pinned);
    engine.set_object_property("configData".into(),       cd_pinned);

    // Thread: pompa output_rx → signal newOutput ogni 50ms
    let am_cb = am_pinned.clone();
    let output_rx = Arc::new(Mutex::new(output_rx));
    let output_rx_cb = output_rx.clone();

    let cb = qmetaobject::queued_callback(move |_: ()| {
        let rx = output_rx_cb.lock().unwrap();
        while let Ok(msg) = rx.try_recv() {
            if msg == "__EXIT__" {
                // Eseguito qui: siamo dentro un callback dispatchato sul
                // thread Qt (quello che possiede l'event loop), quindi
                // process::exit() qui è sicuro. Chiamarlo da un altro
                // thread (es. il thread "intent") causa un segfault per
                // via della thread-affinity degli oggetti Qt.
                std::process::exit(0);
            }
            am_cb.borrow_mut().newOutput(msg.into());
        }
    });

    std::thread::spawn(move || {
        loop {
            cb(());
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    });


    // Aggiungi Connections per colorChanged ai QML tramite contesto
    engine.load_file(qml_file.into());
    if layout == "main" {
        engine.load_file("ui/listcom.qml".into());
    }
    engine.exec();
}

// ─── Note ────────────────────────────────────────────────────────────────────

pub fn avvia_finestra_note(testo: &str) {
    let mut pm = ProcessManager::default();
    pm.testo = testo.into();

    let pm_box: &'static QObjectBox<ProcessManager> =
    Box::leak(Box::new(QObjectBox::new(pm)));

    let mut engine = QmlEngine::new();

    let settings_path = config_dir().join("settings.conf");
    engine.set_property(
        "settingsPath".into(),
                        QString::from(path_to_file_url(&settings_path)).into(),
    );

    engine.set_object_property("processManager".into(), pm_box.pinned());
    engine.load_file("ui/notes.qml".into());
    engine.exec();
}

pub fn mostra_nota(testo: &str) {
    let testo = testo.to_string();
    std::thread::spawn(move || {
        let args: Vec<String> = std::env::args().collect();
        std::process::Command::new(&args[0])
        .args(["--note", &testo])
        .spawn()
        .ok();
    });
}
