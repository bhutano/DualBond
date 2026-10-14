#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli;
mod core;
mod doctor;
mod download;
mod metrics;
mod network;
mod storage;
mod system_balance;

use core::config::Settings;
use download::orchestrator::{
    multi_link_download_with_handle, DownloadHandle, DownloadRunState, LinkSpec,
};
use network::connector::LinkConnector;
use network::registry::{LinkIdentity, LinkRegistry};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use storage::StateDb;
use tauri::Manager;

struct AppState {
    registry: Arc<Mutex<LinkRegistry>>,
    active_download: Arc<Mutex<Option<DownloadHandle>>>,
    settings_path: PathBuf,
    system_balance: system_balance::SystemBalanceClient,
}

#[tauri::command]
async fn get_system_balance_status(
    state: tauri::State<'_, AppState>,
) -> Result<system_balance::SystemBalanceStatus, String> {
    Ok(state.system_balance.status().await)
}

#[tauri::command]
async fn set_system_balance_enabled(
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<system_balance::SystemBalanceStatus, String> {
    state
        .system_balance
        .set_enabled(enabled)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn refresh_system_links(
    state: tauri::State<'_, AppState>,
) -> Result<system_balance::SystemBalanceStatus, String> {
    state
        .system_balance
        .refresh()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn set_system_link_enabled(
    state: tauri::State<'_, AppState>,
    key: String,
    enabled: bool,
) -> Result<system_balance::SystemBalanceStatus, String> {
    state
        .registry
        .lock()
        .unwrap()
        .set_enabled(&key, enabled)
        .map_err(|error| error.to_string())?;
    state
        .system_balance
        .set_link_enabled(key, enabled)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn set_system_link_metered(
    state: tauri::State<'_, AppState>,
    key: String,
    metered: bool,
) -> Result<system_balance::SystemBalanceStatus, String> {
    state
        .registry
        .lock()
        .unwrap()
        .set_metered(&key, metered)
        .map_err(|error| error.to_string())?;
    state
        .system_balance
        .set_link_metered(key, metered)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_settings(state: tauri::State<AppState>) -> Result<Settings, String> {
    Settings::load(&state.settings_path)
}

#[tauri::command]
fn save_settings(state: tauri::State<AppState>, settings: Settings) -> Result<(), String> {
    settings.save(&state.settings_path)
}

#[tauri::command]
async fn doctor_report() -> doctor::DoctorReport {
    let collection = tauri::async_runtime::spawn_blocking(doctor::collect_report);
    match tokio::time::timeout(std::time::Duration::from_secs(18), collection).await {
        Ok(Ok(report)) => report,
        Ok(Err(error)) => doctor::DoctorReport {
            links: Vec::new(),
            warnings: vec![format!("Diagnostica interrotta: {error}")],
        },
        Err(_) => doctor::DoctorReport {
            links: Vec::new(),
            warnings: vec![
                "Diagnostica scaduta dopo 18 secondi. Il tunnel continua a funzionare; riprova dopo aver stabilizzato le connessioni.".into(),
            ],
        },
    }
}

/// Avvia un download multi-link con tutti i link il cui binding è
/// validato (stesso criterio di `dualbond download-multi`, §4.3 non
/// negoziabile). Ritorna subito: il download continua in background,
/// progresso/pausa/annulla passano dal `DownloadHandle` in `AppState`,
/// letto dal monitor 4Hz (`metrics::monitor`) per lo `stats` event.
#[tauri::command]
async fn start_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    url: String,
) -> Result<(), String> {
    {
        let active = state.active_download.lock().unwrap();
        if let Some(h) = active.as_ref() {
            let busy = matches!(
                &*h.state.lock().unwrap(),
                DownloadRunState::Running | DownloadRunState::Verifying
            );
            if busy {
                return Err("Un download è già in corso.".into());
            }
        }
    }

    let (report, candidates) =
        tauri::async_runtime::spawn_blocking(doctor::collect_validated_candidates)
            .await
            .map_err(|e| e.to_string())?;
    for w in &report.warnings {
        tracing::warn!("{w}");
    }
    if candidates.is_empty() {
        return Err("Nessun link con binding validato. Controlla la Diagnostica.".into());
    }

    let settings = Settings::load(&state.settings_path)?;
    let connections = settings.resolved_max_connections();

    // Le connessioni nuove sono abilitate automaticamente; un'eventuale
    // esclusione manuale resta persistente nel registro.
    let links: Vec<LinkSpec> = {
        let reg = state.registry.lock().unwrap();
        candidates
            .iter()
            .filter_map(|c| {
                let local_ip = network::discovery::preferred_local_ip(&c.ips)?;
                let identity = reg.get(&c.stable_key).ok().flatten()?;
                Some(LinkSpec {
                    key: c.stable_key.clone(),
                    connector: LinkConnector::Bound {
                        if_index: c.if_index,
                        local_ip,
                    },
                    enabled: identity.enabled,
                    connections,
                })
            })
            .collect()
    };
    if !links.iter().any(|link| link.enabled) {
        return Err(
            "Nessuna connessione abilitata. Riattivane almeno una nelle Impostazioni prima di scaricare.".into(),
        );
    }

    let dest_dir = if settings.download_directory.is_empty() {
        app.path().download_dir().map_err(|e| e.to_string())?
    } else {
        PathBuf::from(&settings.download_directory)
    };
    let file_name = url
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("download.bin");
    let dest = dest_dir.join(file_name);

    let db = Arc::new(Mutex::new(
        StateDb::open_in_memory().map_err(|e| e.to_string())?,
    ));
    let (tx, rx) = tokio::sync::oneshot::channel();
    let url_owned = url.clone();
    tauri::async_runtime::spawn(async move {
        let _ = multi_link_download_with_handle(db, &url_owned, &dest, None, links, Some(tx)).await;
    });

    let handle = rx
        .await
        .map_err(|_| "avvio del download fallito".to_string())?;
    *state.active_download.lock().unwrap() = Some(handle);
    Ok(())
}

#[tauri::command]
fn pause_download(state: tauri::State<AppState>) -> Result<(), String> {
    let active = state.active_download.lock().unwrap();
    let h = active.as_ref().ok_or("nessun download attivo")?;
    h.paused.store(true, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
fn resume_download(state: tauri::State<AppState>) -> Result<(), String> {
    let active = state.active_download.lock().unwrap();
    let h = active.as_ref().ok_or("nessun download attivo")?;
    h.paused.store(false, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
fn cancel_download(state: tauri::State<AppState>) -> Result<(), String> {
    let active = state.active_download.lock().unwrap();
    let h = active.as_ref().ok_or("nessun download attivo")?;
    h.cancelled.store(true, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
fn list_links(state: tauri::State<AppState>) -> Result<Vec<LinkIdentity>, String> {
    state
        .registry
        .lock()
        .unwrap()
        .list_all()
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn rename_link(state: tauri::State<AppState>, key: String, name: String) -> Result<(), String> {
    state
        .registry
        .lock()
        .unwrap()
        .set_display_name(&key, &name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_link_color(state: tauri::State<AppState>, key: String, color: String) -> Result<(), String> {
    state
        .registry
        .lock()
        .unwrap()
        .set_color(&key, &color)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_link_enabled(
    state: tauri::State<'_, AppState>,
    key: String,
    enabled: bool,
) -> Result<(), String> {
    state
        .registry
        .lock()
        .unwrap()
        .set_enabled(&key, enabled)
        .map_err(|e| e.to_string())?;

    // Il registro decide lo stato ai prossimi avvii; il flag condiviso
    // applica la stessa scelta ai worker del download corrente.
    if let Some(handle) = state.active_download.lock().unwrap().as_ref() {
        if let Some(flag) = handle.link_enabled.get(&key) {
            flag.store(enabled, Ordering::Release);
            let action = if enabled { "riattivata" } else { "esclusa" };
            core::events::push("info", format!("Connessione {action} durante il download"));
        }
    }
    // Le due modalità condividono la stessa preferenza: una modifica
    // nelle Impostazioni vale sia per i download sia per il tunnel.
    let _ = state.system_balance.set_link_enabled(key, enabled).await;
    Ok(())
}

#[tauri::command]
async fn set_link_metered(
    state: tauri::State<'_, AppState>,
    key: String,
    metered: bool,
) -> Result<(), String> {
    state
        .registry
        .lock()
        .unwrap()
        .set_metered(&key, metered)
        .map_err(|e| e.to_string())?;
    let _ = state.system_balance.set_link_metered(key, metered).await;
    Ok(())
}

#[tauri::command]
fn forget_link(state: tauri::State<AppState>, key: String) -> Result<(), String> {
    state
        .registry
        .lock()
        .unwrap()
        .forget(&key)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn open_download_folder(app: tauri::AppHandle) -> Result<(), String> {
    let dir = app.path().download_dir().map_err(|e| e.to_string())?;
    std::process::Command::new("explorer")
        .arg(dir)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("doctor") => {
            core::logging::init();
            cli::run_doctor();
            return;
        }
        Some("download") => {
            core::logging::init();
            cli::run_download(&args[2..]);
            return;
        }
        Some("download-multi") => {
            core::logging::init();
            cli::run_download_multi(&args[2..]);
            return;
        }
        Some("system-service") => {
            let _ = system_balance::service::dispatch();
            return;
        }
        Some("install-service") => {
            std::process::exit(if system_balance::service::install().is_ok() {
                0
            } else {
                1
            });
        }
        Some("remove-service") => {
            std::process::exit(if system_balance::service::remove().is_ok() {
                0
            } else {
                1
            });
        }
        _ => {}
    }

    tauri::Builder::default()
        // Deve essere il primo plugin registrato. I sottocomandi CLI e
        // `system-service` escono prima del builder, quindi l'istanza singola
        // riguarda esclusivamente la UI per sessione utente.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(|app| {
            core::logging::init();

            // Tray con la velocità corrente nel tooltip (§6.8) —
            // aggiornata dal monitor 4Hz via `tray_by_id`. `visible: false`
            // di default nella maggior parte dei tauri.conf.json standard
            // non si applica qui: la finestra principale resta quella
            // primaria, la tray è solo un accesso rapido in più.
            if let Some(icon) = app.default_window_icon().cloned() {
                let tray_menu = tauri::menu::MenuBuilder::new(app)
                    .text("show", "Apri / Open DualBond")
                    .separator()
                    .text("quit", "Esci / Quit")
                    .build()?;
                tauri::tray::TrayIconBuilder::with_id("main-tray")
                    .icon(icon)
                    .tooltip("DualBond")
                    .menu(&tray_menu)
                    .show_menu_on_left_click(false)
                    .on_menu_event(|app, event| match event.id().as_ref() {
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.unminimize();
                                let _ = window.set_focus();
                            }
                        }
                        "quit" => app.exit(0),
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let tauri::tray::TrayIconEvent::Click {
                            button: tauri::tray::MouseButton::Left,
                            button_state: tauri::tray::MouseButtonState::Up,
                            ..
                        } = event
                        {
                            if let Some(window) = tray.app_handle().get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.unminimize();
                                let _ = window.set_focus();
                            }
                        }
                    })
                    .build(app)?;
            }

            let registry_path = app.path().app_data_dir()?;
            std::fs::create_dir_all(&registry_path).ok();
            let registry = LinkRegistry::open(&registry_path.join("registry.db"))
                .map_err(|e| e.to_string())?;
            let registry = Arc::new(Mutex::new(registry));
            let active_download: Arc<Mutex<Option<DownloadHandle>>> = Arc::new(Mutex::new(None));
            // §9: "%APPDATA%\DualBond\config.toml" — app_data_dir() risolve
            // già a %APPDATA%\<identifier>, coerente con la spec.
            let settings_path = registry_path.join("config.toml");

            metrics::monitor::spawn(
                app.handle().clone(),
                registry.clone(),
                active_download.clone(),
            );

            app.manage(AppState {
                registry,
                active_download,
                settings_path,
                system_balance: system_balance::SystemBalanceClient::default(),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            doctor_report,
            start_download,
            pause_download,
            resume_download,
            cancel_download,
            list_links,
            rename_link,
            set_link_color,
            set_link_enabled,
            set_link_metered,
            forget_link,
            open_download_folder,
            get_settings,
            save_settings,
            get_system_balance_status,
            set_system_balance_enabled,
            refresh_system_links,
            set_system_link_enabled,
            set_system_link_metered,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
