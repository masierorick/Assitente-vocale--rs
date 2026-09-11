use anyhow::Result;

/// Invia un prompt all'IA Groq e restituisce la risposta in italiano.
///
/// # Configurazione
/// - **Modello**: "openai/gpt-oss-120b"
/// - **Endpoint**: `https://api.groq.com/openai/v1/chat/completions`
/// - **Autenticazione**: Bearer token tramite `api_key` (da `API_KEY_GROQ` o `API_KEY` nel `.env`)
/// - La risposta viene sempre richiesta in italiano tramite prefisso nel prompt.
#[allow(dead_code)]
pub fn ask_groq(prompt: &str, api_key: &str) -> Result<String> {
    let body = serde_json::json!({
        "model": "openai/gpt-oss-120b",
        "messages": [{"role": "user", "content": format!("Rispondi in italiano: {}", prompt)}]
    });
    let response = ureq::post("https://api.groq.com/openai/v1/chat/completions")
        .set("Authorization", &format!("Bearer {}", api_key))
        .set("Content-Type", "application/json")
        .send_string(&serde_json::to_string(&body)?)?;
    let data: serde_json::Value = serde_json::from_str(&response.into_string()?)?;
    Ok(data["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string())
}


/// Chiede a Groq di trasformare un comando vocale in un piano di azioni JSON.
/// Il piano è volutamente limitato ad azioni non distruttive: l'esecuzione
/// effettiva resta sotto il controllo di `intent.rs`.
///
/// `browser` e `musicplayer` sono i binari realmente configurati in
/// `config.json` (`cfg.browser`, `cfg.musicplayer`). `programs` è l'elenco
/// "Nome (eseguibile)" costruito da `intent.rs::lista_programmi_per_ai` a
/// partire dallo stesso file usato da `system::apri_programma` per gli
/// intent locali. Senza queste informazioni il modello indovinava nomi
/// generici da un elenco di esempio ("firefox", "chromium") che potevano
/// non essere installati sul sistema dell'utente.
pub fn ask_groq_agent(
    prompt: &str,
    api_key: &str,
    browser: &str,
    musicplayer: &str,
    programs: &str,
) -> Result<serde_json::Value> {
    let programs_line = if programs.is_empty() {
        "(nessuno rilevato)".to_string()
    } else {
        programs.to_string()
    };

    let system = format!(
        r#"
Sei il motore di automazione desktop di assistente-rs su Linux KDE/Wayland.
Trasforma il comando dell'utente in un piano JSON da eseguire sul computer.

Rispondi ESCLUSIVAMENTE con JSON valido, senza markdown, spiegazioni o testo extra.
Formato obbligatorio:
{{"actions":[{{"action":"...", ...}}]}}

Azioni consentite:
1. {{"action":"open_program","program":"..."}}
2. {{"action":"open_url","url":"https://..."}}
3. {{"action":"type_text","text":"..."}}
4. {{"action":"key","key":"..."}}
5. {{"action":"hotkey","keys":["CTRL","C"]}}
6. {{"action":"wait","ms":500}}

Programmi realmente installati e configurati su questo sistema — usa SEMPRE
questi nomi esatti per aprire browser o player musicale, MAI nomi generici
come "firefox" o "chromium" se non compaiono qui sotto:
- browser: {browser}
- musicplayer: {musicplayer}

Altri programmi disponibili su questo sistema, nel formato "Nome (eseguibile)":
usa SEMPRE l'eseguibile esatto tra parentesi in open_program, mai il Nome e
mai un programma che non compare in questo elenco né corrisponde a
browser/musicplayer sopra:
{programs_line}

Regole:
- Usa più azioni quando il comando richiede una sequenza.
- Per key usa nomi compatibili con XKB/wtype: Enter, Escape, Tab, BackSpace, Delete, Left, Right, Up, Down, Home, End, Page_Up, Page_Down, F1-F12.
- Per hotkey usa solo CTRL, ALT, SHIFT, SUPER e tasti semplici o nomi XKB.
- Non usare shell, terminal commands, sudo, rm, kill, shutdown, reboot, format, delete o altre azioni distruttive.
- Non inventare risultati di operazioni che non puoi eseguire.
- Se il comando non è eseguibile con le azioni consentite, o richiede un
  programma che non è nell'elenco, restituisci {{"actions":[]}}.
- Non includere la wakeword.
"#,
        browser = browser,
        musicplayer = musicplayer,
        programs_line = programs_line,
    );

    let body = serde_json::json!({
        "model": "openai/gpt-oss-120b",
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": prompt}
        ],
        "temperature": 0,
        "response_format": {"type": "json_object"}
    });

    let response = ureq::post("https://api.groq.com/openai/v1/chat/completions")
        .set("Authorization", &format!("Bearer {}", api_key))
        .set("Content-Type", "application/json")
        .send_string(&serde_json::to_string(&body)?)?;

    let data: serde_json::Value = serde_json::from_str(&response.into_string()?)?;
    let content = data["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .trim();

    Ok(serde_json::from_str(content)?)
}

/// Cerca video su YouTube tramite le Data API v3 e restituisce una lista di URL.
///
/// # Configurazione
/// - **Endpoint**: `https://www.googleapis.com/youtube/v3/search`
/// - **Autenticazione**: API key tramite `yt_key` (da `API_KEY_YOUTUBE` nel `.env`)
/// - **Parametri**: `part=snippet`, `type=video`, `maxResults` configurabile
pub fn search_youtube(query: &str, api_key: &str, max_results: usize) -> Result<Vec<String>> {
    let url = format!(
        "https://www.googleapis.com/youtube/v3/search?part=snippet&q={}&type=video&maxResults={}&key={}",
        urlencoding::encode(query),
        max_results,
        api_key
    );
    let response = ureq::get(&url).call()?;
    let data: serde_json::Value = serde_json::from_str(&response.into_string()?)?;
    let mut urls = Vec::new();
    if let Some(items) = data["items"].as_array() {
        for item in items {
            if let Some(video_id) = item["id"]["videoId"].as_str() {
                urls.push(format!("https://www.youtube.com/watch?v={}", video_id));
            }
        }
    }
    Ok(urls)
}
