use super::model::{ControlCommand, ControlResponse, CONTROL_PORT};
use super::runtime::ServiceRuntime;
use std::io;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

const MAX_REQUEST: u64 = 65_536;

pub async fn serve(
    runtime: Arc<ServiceRuntime>,
    token: String,
    mut stop: watch::Receiver<bool>,
) -> io::Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", CONTROL_PORT)).await?;
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let runtime = runtime.clone();
                let token = token.clone();
                tokio::spawn(async move {
                    let _ = handle(stream, runtime, &token).await;
                });
            }
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() { return Ok(()); }
            }
        }
    }
}

async fn handle(stream: TcpStream, runtime: Arc<ServiceRuntime>, token: &str) -> io::Result<()> {
    stream.set_nodelay(true)?;
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader).take(MAX_REQUEST + 1);
    let mut payload = Vec::new();
    let count = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        reader.read_until(b'\n', &mut payload),
    )
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "timeout controllo"))??;

    let response = if count == 0 || count as u64 > MAX_REQUEST || !payload.ends_with(b"\n") {
        error_response(runtime.snapshot(), "Richiesta di controllo non valida")
    } else {
        match serde_json::from_slice::<ControlCommand>(&payload) {
            Ok(command) if constant_time_eq(command.token().as_bytes(), token.as_bytes()) => {
                apply(runtime, command)
            }
            Ok(_) => error_response(runtime.snapshot(), "Autenticazione non valida"),
            Err(_) => error_response(runtime.snapshot(), "JSON di controllo non valido"),
        }
    };

    let mut encoded = serde_json::to_vec(&response)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    encoded.push(b'\n');
    writer.write_all(&encoded).await?;
    writer.shutdown().await
}

fn apply(runtime: Arc<ServiceRuntime>, command: ControlCommand) -> ControlResponse {
    let result = match command {
        ControlCommand::Status { .. } => Ok(runtime.snapshot()),
        ControlCommand::SetEnabled { enabled, .. } => runtime.set_enabled(enabled),
        ControlCommand::Refresh { .. } => Ok(runtime.refresh_links()),
        ControlCommand::SetLinkEnabled { key, enabled, .. } => {
            runtime.set_link_enabled(&key, enabled)
        }
        ControlCommand::SetLinkMetered { key, metered, .. } => {
            runtime.set_link_metered(&key, metered)
        }
    };
    match result {
        Ok(status) => ControlResponse {
            ok: true,
            status,
            error: None,
        },
        Err(error) => error_response(runtime.snapshot(), &error.to_string()),
    }
}

fn error_response(status: super::model::SystemBalanceStatus, message: &str) -> ControlResponse {
    ControlResponse {
        ok: false,
        status,
        error: Some(message.chars().take(300).collect()),
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    let length = left.len().max(right.len());
    for index in 0..length {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        difference |= (left_byte ^ right_byte) as usize;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system_balance::model::ServiceConfig;
    use crate::system_balance::store::ServicePaths;

    #[test]
    fn token_comparison_handles_equal_different_and_different_lengths() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abc0"));
    }

    #[test]
    fn errors_are_bounded() {
        let response = error_response(Default::default(), &"x".repeat(1_000));
        assert_eq!(response.error.unwrap().len(), 300);
    }

    #[tokio::test]
    async fn authenticated_status_round_trips_over_the_local_protocol() {
        let paths = ServicePaths::under(
            std::env::temp_dir().join(format!("dualbond-control-{}", std::process::id())),
        );
        let runtime = ServiceRuntime::new(paths, ServiceConfig::default());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn({
            let runtime = runtime.clone();
            async move {
                let (stream, _) = listener.accept().await.unwrap();
                handle(stream, runtime, "secret").await.unwrap();
            }
        });

        let mut stream = TcpStream::connect(address).await.unwrap();
        let request = serde_json::to_string(&ControlCommand::Status {
            token: "secret".into(),
        })
        .unwrap();
        stream
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap();
        let mut response = String::new();
        BufReader::new(stream)
            .read_line(&mut response)
            .await
            .unwrap();
        let response: ControlResponse = serde_json::from_str(&response).unwrap();
        assert!(response.ok);
        assert!(response.status.service_available);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn wrong_token_is_rejected_without_applying_the_command() {
        let paths = ServicePaths::under(
            std::env::temp_dir().join(format!("dualbond-control-bad-{}", std::process::id())),
        );
        let runtime = ServiceRuntime::new(paths, ServiceConfig::default());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn({
            let runtime = runtime.clone();
            async move {
                let (stream, _) = listener.accept().await.unwrap();
                handle(stream, runtime, "correct").await.unwrap();
            }
        });

        let mut stream = TcpStream::connect(address).await.unwrap();
        let request = serde_json::to_string(&ControlCommand::SetEnabled {
            token: "wrong".into(),
            enabled: true,
        })
        .unwrap();
        stream
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap();
        let mut response = String::new();
        BufReader::new(stream)
            .read_line(&mut response)
            .await
            .unwrap();
        let response: ControlResponse = serde_json::from_str(&response).unwrap();
        assert!(!response.ok);
        assert!(!runtime.config.read().unwrap().enabled);
        server.await.unwrap();
    }
}
