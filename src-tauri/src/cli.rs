use crate::doctor::{collect_report, collect_validated_candidates};
use crate::download::download_to_file;
use crate::download::orchestrator::{multi_link_download, LinkSpec};
use crate::network::connector::LinkConnector;
use crate::storage::StateDb;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Comando `dualbond doctor`: elenca i link fisici rilevati e valida
/// empiricamente il binding per ciascuno (criterio di uscita Milestone 1).
/// Stampa a terminale il risultato di [`collect_report`], condiviso con il
/// comando Tauri `doctor` invocato dalla UI.
pub fn run_doctor() {
    let report = collect_report();

    for w in &report.warnings {
        eprintln!("Avviso: {w}");
    }

    println!("Link rilevati:");
    for l in &report.links {
        println!(
            "  [{}] {} — {} — Internet={} — key={} — ip={:?}",
            l.if_index, l.name, l.kind, l.internet_access, l.stable_key, l.ips
        );
    }

    if report.links.is_empty() {
        println!("Nessun link fisico rilevato.");
        return;
    }

    println!("\nValidazione binding (probe ~5s per gateway noto per link):");
    for l in &report.links {
        for p in &l.probes {
            println!(
                "  [{}] {} ({}): {}",
                l.if_index, l.name, p.family, p.outcome
            );
        }
    }
}

/// Comando `dualbond download <URL> [-o <cartella>] [--sha256 <hash>]`
/// (Milestone 2, Parte 9 della spec). Downloader a link singolo: nessuna
/// persistenza/resume (Milestone 3) né multi-link (Milestone 4) ancora.
pub fn run_download(args: &[String]) {
    let Some(url) = args.first() else {
        eprintln!("Uso: dualbond download <URL> [-o <cartella>] [--sha256 <hash>]");
        std::process::exit(2);
    };

    let mut out_dir: Option<PathBuf> = None;
    let mut expected_sha256: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => {
                i += 1;
                out_dir = args.get(i).map(PathBuf::from);
            }
            "--sha256" => {
                i += 1;
                expected_sha256 = args.get(i).cloned();
            }
            other => {
                eprintln!("Opzione sconosciuta: {other}");
                std::process::exit(2);
            }
        }
        i += 1;
    }

    let dir = out_dir.unwrap_or_else(|| PathBuf::from("."));
    let file_name = url
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("download.bin");
    let dest = dir.join(file_name);

    println!("Download: {url}");
    println!("Destinazione: {}", dest.display());

    let rt = tokio::runtime::Runtime::new().expect("impossibile avviare il runtime async");
    let result = rt.block_on(download_to_file(url, &dest, expected_sha256.as_deref()));

    match result {
        Ok(outcome) => {
            println!(
                "Completato: {} byte, SHA-256 {}",
                outcome.bytes_written, outcome.sha256_hex
            );
        }
        Err(e) => {
            eprintln!("Errore: {e}");
            std::process::exit(1);
        }
    }
}

/// Comando `dualbond download-multi <URL> [-o <cartella>] [--sha256 <hash>]
/// [--connections-per-link <N>]` (Milestone 4). Usa tutti i link il cui
/// binding è stato validato empiricamente (§4.3, stesso criterio di
/// `doctor`) — se nessun link è validato, nessun download parte: mai
/// scaricare su un link di cui non siamo certi al 100% dell'isolamento
/// (stesso principio non negoziabile di Milestone 1).
///
/// **Limite noto**: nessuna persistenza reale — lo stato vive in un DB in
/// memoria per la durata del solo processo, quindi un'interruzione non è
/// riprendibile (vedi `docs/milestone4-notes.md`, punto 2). Per un download
/// ripristinabile usare `dualbond download` (single-link, Milestone 3).
pub fn run_download_multi(args: &[String]) {
    let Some(url) = args.first() else {
        eprintln!("Uso: dualbond download-multi <URL> [-o <cartella>] [--sha256 <hash>] [--connections-per-link <N>]");
        std::process::exit(2);
    };

    let mut out_dir: Option<PathBuf> = None;
    let mut expected_sha256: Option<String> = None;
    let mut connections_per_link: u32 = 4; // bootstrap §4.4
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => {
                i += 1;
                out_dir = args.get(i).map(PathBuf::from);
            }
            "--sha256" => {
                i += 1;
                expected_sha256 = args.get(i).cloned();
            }
            "--connections-per-link" => {
                i += 1;
                connections_per_link =
                    args.get(i).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
                        eprintln!("--connections-per-link richiede un numero");
                        std::process::exit(2);
                    });
            }
            other => {
                eprintln!("Opzione sconosciuta: {other}");
                std::process::exit(2);
            }
        }
        i += 1;
    }

    let dir = out_dir.unwrap_or_else(|| PathBuf::from("."));
    let file_name = url
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("download.bin");
    let dest = dir.join(file_name);

    println!("Validazione link (probe ~5s per gateway noto per link)...");
    let (report, candidates) = collect_validated_candidates();
    for w in &report.warnings {
        eprintln!("Avviso: {w}");
    }
    if candidates.is_empty() {
        eprintln!(
            "Nessun link con binding validato: vedi `dualbond doctor` per la diagnosi. Nessun download avviato."
        );
        std::process::exit(1);
    }

    let links: Vec<LinkSpec> = candidates
        .iter()
        .filter_map(|c| {
            let local_ip = crate::network::discovery::preferred_local_ip(&c.ips)?;
            Some(LinkSpec {
                key: c.stable_key.clone(),
                connector: LinkConnector::Bound {
                    if_index: c.if_index,
                    local_ip,
                },
                enabled: true,
                connections: connections_per_link,
            })
        })
        .collect();

    println!("Link usati:");
    for l in &links {
        println!("  {} (x{} connessioni)", l.key, l.connections);
    }
    println!("Download: {url}");
    println!("Destinazione: {}", dest.display());

    let db = Arc::new(Mutex::new(
        StateDb::open_in_memory().expect("impossibile aprire lo stato in memoria"),
    ));
    let rt = tokio::runtime::Runtime::new().expect("impossibile avviare il runtime async");
    let result = rt.block_on(multi_link_download(
        db,
        url,
        &dest,
        expected_sha256.as_deref(),
        links,
    ));

    match result {
        Ok(outcome) => {
            println!(
                "Completato: {} byte, SHA-256 {}",
                outcome.bytes_written, outcome.sha256_hex
            );
        }
        Err(e) => {
            eprintln!("Errore: {e}");
            std::process::exit(1);
        }
    }
}
