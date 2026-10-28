use super::model::{ControlCommand, ControlResponse, SystemBalanceStatus, CONTROL_PORT};
use super::store::{load_or_create_token, ServicePaths};
use std::io;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

#[derive(Clone)]
pub struct SystemBalanceClient {
    paths: ServicePaths,
}

impl Default for SystemBalanceClient {
    fn default() -> Self {
        Self {
            paths: ServicePaths::system(),
        }
    }
}

impl SystemBalanceClient {
    pub async fn status(&self) -> SystemBalanceStatus {
        match self.command(|token| ControlCommand::Status { token }).await {
            Ok(status) => status,
            Err(error) => SystemBalanceStatus::unavailable(error.to_string()),
        }
    }

    pub async fn set_enabled(&self, enabled: bool) -> io::Result<SystemBalanceStatus> {
        self.command(|token| ControlCommand::SetEnabled { token, enabled })
            .await
    }

    pub async fn refresh(&self) -> io::Result<SystemBalanceStatus> {
        self.command(|token| ControlCommand::Refresh { token })
            .await
    }

    pub async fn set_link_enabled(
        &self,
        key: String,
        enabled: bool,
    ) -> io::Result<SystemBalanceStatus> {
        self.command(|token| ControlCommand::SetLinkEnabled {
            token,
            key,
            enabled,
        })
        .await
    }

    pub async fn set_link_metered(
        &self,
        key: String,
        metered: bool,
    ) -> io::Result<SystemBalanceStatus> {
        self.command(|token| ControlCommand::SetLinkMetered {
            token,
            key,
            metered,
        })
        .await
    }

    async fn command<F>(&self, build: F) -> io::Result<SystemBalanceStatus>
    where
        F: FnOnce(String) -> ControlCommand,
    {
        let token = load_or_create_token(&self.paths)?;
        let command = build(token);
        let stream = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            TcpStream::connect(("127.0.0.1", CONTROL_PORT)),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "servizio non raggiungibile"))??;
        stream.set_nodelay(true)?;
        let (reader, mut writer) = stream.into_split();
        let mut encoded = serde_json::to_vec(&command)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        encoded.push(b'\n');
        writer.write_all(&encoded).await?;
        writer.shutdown().await?;

        let mut line = Vec::new();
        let mut reader = BufReader::new(reader);
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            reader.read_until(b'\n', &mut line),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "risposta servizio scaduta"))??;
        if line.len() > 65_536 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "risposta troppo grande",
            ));
        }
        let response: ControlResponse = serde_json::from_slice(&line)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if response.ok {
            Ok(response.status)
        } else {
            Err(io::Error::other(
                response.error.unwrap_or_else(|| "comando rifiutato".into()),
            ))
        }
    }
}

/// Avvia il servizio di sistema chiedendo l'elevazione (UAC): serve quando il
/// servizio si è fermato e Windows non lo ha ancora fatto ripartire. Non
/// attende la fine: l'interfaccia vede il servizio tornare tramite il polling.
pub fn start_service_elevated() -> io::Result<()> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }
    let verb = wide("runas");
    let file = wide("sc.exe");
    let parameters = wide(&format!("start {}", super::service::SERVICE_NAME));
    // SAFETY: tutte le stringhe sono UTF-16 terminate da zero e restano vive
    // per tutta la durata della chiamata.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            parameters.as_ptr(),
            std::ptr::null(),
            SW_HIDE,
        )
    };
    // ShellExecuteW restituisce un valore > 32 in caso di successo.
    if result as isize > 32 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
