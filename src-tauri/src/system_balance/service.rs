use super::control;
use super::runtime::{self, ServiceRuntime};
use super::store::{load_config, load_or_create_token, ServicePaths};
use std::ffi::OsString;
use std::sync::mpsc;
use std::time::Duration;
use windows_service::define_windows_service;
use windows_service::service::{
    ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
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
    service
        .start::<&str>(&[])
        .map_err(|error| error.to_string())?;
    Ok(())
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
    let _ = worker.join();

    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;
    Ok(())
}

fn run_async(shutdown_rx: mpsc::Receiver<()>) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return,
    };
    runtime.block_on(async move {
        let paths = ServicePaths::system();
        if paths.ensure().is_err() {
            return;
        }
        let _ = crate::core::logging::init_file(&paths.log);
        tracing::info!(log = %paths.log.display(), "servizio DualBond avviato");
        let config = load_config(&paths).unwrap_or_default();
        let token = match load_or_create_token(&paths) {
            Ok(token) => token,
            Err(_) => return,
        };
        let shared = ServiceRuntime::new(paths, config);
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let stop_bridge = std::thread::spawn(move || {
            let _ = shutdown_rx.recv();
            stop_tx.send_replace(true);
        });

        let control_task = tokio::spawn(control::serve(shared.clone(), token, stop_rx.clone()));
        let runtime_result = runtime::run(shared, stop_rx).await;
        control_task.abort();
        let _ = runtime_result;
        let _ = stop_bridge.join();
    });
}
