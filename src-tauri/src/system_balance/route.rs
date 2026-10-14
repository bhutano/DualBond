use std::io;
use std::process::{Command, Output};

pub const ROUTE_NETWORKS: [(&str, &str); 2] =
    [("0.0.0.0", "128.0.0.0"), ("128.0.0.0", "128.0.0.0")];

pub trait RouteExecutor: Send + Sync {
    fn execute(&self, args: &[String]) -> io::Result<Output>;
}

pub struct WindowsRouteExecutor;

impl RouteExecutor for WindowsRouteExecutor {
    fn execute(&self, args: &[String]) -> io::Result<Output> {
        Command::new("route.exe").args(args).output()
    }
}

pub struct RouteGuard<E: RouteExecutor> {
    executor: E,
    if_index: u32,
    active: bool,
}

impl<E: RouteExecutor> RouteGuard<E> {
    pub fn install(executor: E, if_index: u32) -> io::Result<Self> {
        cleanup_routes(&executor, if_index);
        let mut guard = Self {
            executor,
            if_index,
            active: false,
        };
        for (network, mask) in ROUTE_NETWORKS {
            let args = add_args(network, mask, if_index);
            let output = guard.executor.execute(&args)?;
            if !output.status.success() {
                guard.cleanup();
                return Err(command_error("aggiunta route", &output));
            }
        }
        guard.active = true;
        Ok(guard)
    }

    pub fn cleanup(&mut self) {
        cleanup_routes(&self.executor, self.if_index);
        self.active = false;
    }
}

impl<E: RouteExecutor> Drop for RouteGuard<E> {
    fn drop(&mut self) {
        if self.active {
            self.cleanup();
        }
    }
}

pub fn cleanup_routes<E: RouteExecutor>(executor: &E, if_index: u32) {
    for (network, mask) in ROUTE_NETWORKS {
        let _ = executor.execute(&delete_args(network, mask, if_index));
    }
}

fn add_args(network: &str, mask: &str, if_index: u32) -> Vec<String> {
    vec![
        "ADD".into(),
        network.into(),
        "MASK".into(),
        mask.into(),
        "10.77.0.2".into(),
        "IF".into(),
        if_index.to_string(),
        "METRIC".into(),
        "5".into(),
    ]
}

fn delete_args(network: &str, mask: &str, if_index: u32) -> Vec<String> {
    vec![
        "DELETE".into(),
        network.into(),
        "MASK".into(),
        mask.into(),
        "IF".into(),
        if_index.to_string(),
    ]
}

fn command_error(action: &str, output: &Output) -> io::Error {
    let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    io::Error::other(if message.is_empty() {
        format!("{action} fallita con codice {:?}", output.status.code())
    } else {
        format!("{action} fallita: {message}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct FakeExecutor {
        calls: Arc<Mutex<Vec<Vec<String>>>>,
    }

    impl RouteExecutor for FakeExecutor {
        fn execute(&self, args: &[String]) -> io::Result<Output> {
            self.calls.lock().unwrap().push(args.to_vec());
            Command::new("cmd").args(["/C", "exit", "0"]).output()
        }
    }

    #[test]
    fn install_uses_two_non_persistent_half_default_routes_and_drop_cleans_them() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let executor = FakeExecutor {
            calls: calls.clone(),
        };
        {
            let _guard = RouteGuard::install(executor, 42).unwrap();
        }
        let calls = calls.lock().unwrap();
        assert_eq!(
            calls
                .iter()
                .filter(|args| args.first().is_some_and(|arg| arg == "ADD"))
                .count(),
            2
        );
        assert!(calls.iter().all(|args| !args.iter().any(|arg| arg == "-p")));
        assert!(
            calls
                .iter()
                .filter(|args| args.first().is_some_and(|arg| arg == "DELETE"))
                .count()
                >= 4
        );
    }

    #[test]
    fn route_arguments_pin_the_virtual_interface() {
        let args = add_args("0.0.0.0", "128.0.0.0", 77);
        assert_eq!(
            args,
            [
                "ADD",
                "0.0.0.0",
                "MASK",
                "128.0.0.0",
                "10.77.0.2",
                "IF",
                "77",
                "METRIC",
                "5"
            ]
        );
    }
}
