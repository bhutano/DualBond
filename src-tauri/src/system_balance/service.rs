use super::control;
use super::runtime::{self, ServiceRuntime};
use super::store::{load_config, load_or_create_token, ServicePaths};
use std::ffi::OsString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;
use windows_service::define_windows_service;
use windows_service::service::{
    ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept,
    ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_dispatcher;
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

pub const SERVICE_NAME: &str = "DualBondService";
const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

define_windows_service!(ffi_service_main, service_main);

pub fn dispatch() -> Result<(), String> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main).map_err(|error| error.to_string())
}

pub fn install() -> Result<(), String> {
    remove().ok();
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .map_err(|error| error.to_string())?;
    let executable_path = std::env::current_exe().map_err(|error| error.to_string())?;
    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from("DualBond System Balance"),
        service_type: SERVICE_TYPE,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path,
        launch_arguments: vec![OsString::from("system-service")],
        dependencies: Vec::new(),
        account_name: None,
        account_password: None,
    };
    let access = ServiceAccess::START
        | ServiceAccess::STOP
        | ServiceAccess::QUERY_STATUS
        | ServiceAccess::CHANGE_CONFIG
        | ServiceAccess::DELETE;
    let service = manager
        .create_service(&info, access)
        .map_err(|error| error.to_string())?;
    service
        .set_description("Bilanciamento automatico delle connessioni Internet di Windows.")
        .map_err(|error| error.to_string())?;
    // Senza azioni di ripristino un crash lascia il servizio fermo fino al
    // prossimo riavvio del PC (che con sospensione/ibernazione può arrivare
    // dopo giorni) e l'app non vede più nessuna connessione.
    service
        .update_failure_actions(failure_actions())
        .map_err(|error| error.to_string())?;
    // Riavvia anche quando il servizio si ferma da solo con un codice di
    // errore (runtime terminato senza richiesta di stop), non solo su crash.
    service
        .set_failure_actions_on_non_crash_failures(true)
        .map_err(|error| error.to_string())?;
    service
        .start::<&str>(&[])
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn failure_actions() -> ServiceFailureActions {
    let restart = |seconds| ServiceAction {
        action_type: ServiceActionType::Restart,
        delay: Duration::from_secs(seconds),
    };
    ServiceFailureActions {
        // il contatore dei fallimenti si azzera dopo un giorno senza crash
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(24 * 60 * 60)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![restart(5), restart(15), restart(60)]),
    }
}

pub fn remove() -> Result<(), String> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|error| error.to_string())?;
    let access = ServiceAccess::STOP | ServiceAccess::QUERY_STATUS | ServiceAccess::DELETE;
    let service = match manager.open_service(SERVICE_NAME, access) {
        Ok(service) => service,
        Err(windows_service::Error::Winapi(error))
            if error.raw_os_error()
                == Some(windows_sys::Win32::Foundation::ERROR_SERVICE_DOES_NOT_EXIST as i32) =>
        {
            return Ok(())
        }
        Err(error) => return Err(error.to_string()),
    };
    if service
        .query_status()
        .map_err(|error| error.to_string())?
        .current_state
        != ServiceState::Stopped
    {
        service.stop().ok();
    }
    service.delete().map_err(|error| error.to_string())?;
    drop(service);
    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_secs(8) {
        std::thread::sleep(Duration::from_millis(250));
        match manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS) {
            Err(windows_service::Error::Winapi(error))
                if error.raw_os_error()
                    == Some(
                        windows_sys::Win32::Foundation::ERROR_SERVICE_DOES_NOT_EXIST as i32,
                    ) =>
            {
                return Ok(())
            }
            _ => {}
        }
    }
    Ok(())
}

fn service_main(_arguments: Vec<OsString>) {
    let _ = run_service();
}

fn run_service() -> windows_service::Result<()> {
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
    let event_handler = move |event| -> ServiceControlHandlerResult {
        match event {
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = shutdown_tx.send(());
                ServiceControlHandlerResult::NoError
            }
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };
    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;
    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    let worker = std::thread::spawn(move || run_async(shutdown_rx));
    let clean = worker.join().unwrap_or(false);
    // Un codice diverso da zero fa scattare le azioni di ripristino di SCM
    // (vedi `failure_actions`): il servizio riparte da solo.
    let exit_code = if clean {
        0
    } else {
        windows_sys::Win32::Foundation::ERROR_SERVICE_SPECIFIC_ERROR
    };

    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(exit_code),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;
    Ok(())
}

/// Restituisce `true` solo se il servizio si è fermato perché Windows lo ha
/// chiesto (stop o spegnimento); qualunque altra uscita è un guasto.
fn run_async(shutdown_rx: mpsc::Receiver<()>) -> bool {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return false,
    };
    runtime.block_on(async move {
        let paths = ServicePaths::system();
        if paths.ensure().is_err() {
            return false;
        }
        let _ = crate::core::logging::init_file(&paths.log);
        crate::core::logging::install_panic_hook();
        tracing::info!(
            version = env!("CARGO_PKG_VERSION"),
            log = %paths.log.display(),
            "servizio DualBond avviato"
        );
        let config = load_config(&paths).unwrap_or_default();
        let token = match load_or_create_token(&paths) {
            Ok(token) => token,
            Err(error) => {
                tracing::error!(%error, "token di controllo non disponibile");
                return false;
            }
        };
        let shared = ServiceRuntime::new(paths, config);
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let stop_tx = std::sync::Arc::new(stop_tx);
        let windows_requested = std::sync::Arc::new(AtomicBool::new(false));
        let stop_bridge = {
            let stop_tx = stop_tx.clone();
            let windows_requested = windows_requested.clone();
            std::thread::spawn(move || {
                let _ = shutdown_rx.recv();
                windows_requested.store(true, Ordering::SeqCst);
                stop_tx.send_replace(true);
            })
        };

        let control_task = {
            let shared = shared.clone();
            let control_stop = stop_rx.clone();
            let stop_tx = stop_tx.clone();
            tokio::spawn(async move {
                // Senza il canale di controllo l'app non riceve più lo stato e
                // non vede nessuna connessione: fermiamo il runtime in modo
                // ordinato (rotte ripristinate) e lasciamo che SCM riavvii.
                if let Err(error) = control::serve(shared, token, control_stop).await {
                    tracing::error!(%error, "canale di controllo terminato");
                    stop_tx.send_replace(true);
                }
            })
        };
        let runtime_result = runtime::run(shared, stop_rx).await;
        control_task.abort();
        let requested = windows_requested.load(Ordering::SeqCst);
        match &runtime_result {
            Ok(()) if requested => tracing::info!("servizio DualBond arrestato su richiesta"),
            Ok(()) => tracing::error!("runtime terminato senza richiesta di arresto"),
            Err(error) => tracing::error!(%error, "runtime terminato con errore"),
        }
        if requested {
            let _ = stop_bridge.join();
        }
        requested && runtime_result.is_ok()
    })
}
