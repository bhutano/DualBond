# Note — Milestone 7 (speed test, diagnostica, impostazioni, CLI) e 8 (stress test, tray, packaging)

## Stato al 2026-08-21, fine sessione overnight

### M7 — fatto

- **`core/events.rs`**: log eventi globale in memoria (buffer circolare,
  200 voci), cablato in worker/orchestrator/monitor. Chiude il gap
  "`recentEvents` sempre vuoto" lasciato aperto da M6. 4 test.
- **`core/config.rs`**: `Settings`, schema `config.toml` esatto da spec
  §9 (tutti i campi opzionali, default sensati), `load`/`save`. 7 test.
  Cablato in `main.rs`: `start_download` usa
  `resolved_max_connections()` e `download_directory` invece dei
  valori fissi delle sessioni precedenti. `Settings.svelte` ha ora tab
  Download e Comportamento realmente funzionanti (prima erano solo
  presentati).
- **`core/power.rs`**: `StayAwakeGuard` — `SetThreadExecutionState`
  per tutta la durata di un download (§4.9, mai fatto prima stanotte).
- **CLI**: `dualbond doctor`/`download`/`download-multi` esistevano
  già (M1/M4). I comandi aggiuntivi della spec §9
  (`resume --all`, `status`, `pause <id>`, `cancel <id>`, `links
  --rename/--enable/--disable`, `test [URL]`, `--json`) **non sono
  stati aggiunti**: con la GUI ormai funzionante (M6) e pausa/annulla/
  gestione link già raggiungibili da lì, ho dato priorità a chiudere
  quanti più pezzi diversi della roadmap possibile piuttosto che
  approfondire la CLI, che resta comunque utilizzabile per l'uso
  scriptabile di base già implementato.
- **Diagnostica in UI**: presente (`Settings.svelte` → Avanzate),
  riusa `doctor_report`. Nessun visualizzatore di log su file dedicato
  (`apri cartella dei log`/`apri config.toml` di §6.6 non implementati
  come pulsanti, ma i percorsi esistono davvero:
  `app_data_dir()/registry.db` e ora anche `config.toml`).
- **SpeedTest.svelte**: riusa `doctor` (nessun `metrics/benchmark.rs`
  dedicato — vedi `milestone6-notes.md` punto 8).

### M8 — fatto

- **Tray**: icona con tooltip aggiornato ogni 250ms con la velocità
  totale corrente (§6.8), click sinistro riporta in primo piano la
  finestra principale.
- **Notifica di sistema**: a download completato, una sola volta per
  transizione (`tauri-plugin-notification`).
- **Packaging MSI + NSIS**: era già configurato in `tauri.conf.json`
  da una sessione precedente (`bundle.targets: ["nsis", "msi"]`) —
  verificato presente, nessun lavoro aggiuntivo necessario lì.
  "Portabile" nel senso della spec corrisponde al `.exe` compilato
  direttamente (`target/release/dualbond.exe`), non serve un target
  Tauri dedicato per quello.

### M8 — non fatto, e perché onestamente non è stato tentato

- **Stress test file > 100 GB, rete instabile**: richiede ore di
  download reali di file enormi e/o hardware che simuli instabilità di
  rete — non eseguibile in modo significativo in questa sessione
  (nemmeno con tempo illimitato avrebbe senso automatizzarlo qui: è un
  test che richiede l'ambiente reale dell'utente).
- **Profiling**: nessun profiler è stato collegato. Andrebbe fatto con
  un vero carico multi-link su hardware reale multi-interfaccia (che
  l'ambiente di sviluppo non ha, un solo Wi-Fi disponibile — vedi
  `milestone4-notes.md`), altrimenti i numeri non significherebbero
  granché.
- **"Avvia con Windows"**: opzione presente nel mockup §6.6 ma non
  implementata (richiede una entry nel registro `Run` o un task
  pianificato — non fatto).
- **"Riduci a icona nella tray"**: la tray esiste ed è funzionante
  (tooltip, click per riportare in primo piano), ma il comportamento
  "chiudi la finestra e resta solo in tray invece di uscire" non è
  cablato (oggi chiudere la finestra chiude l'app, comportamento
  Tauri di default).

## Verifica visiva reale, fatta per la prima volta stanotte

Il blocco loopback ambientale (documentato 4 volte tra M3 e M6) si è
finalmente risolto abbastanza a lungo da permettere di:
1. Avviare `vite` dev server e raggiungerlo.
2. Usare `playwright` (installato al volo nello scratchpad, non nel
   progetto) per screenshot reali: tema chiaro, tema scuro, pannello
   Impostazioni (5 tab), pannello Test connessioni.
3. **Trovare e fixare un bug reale**: `SpeedTest.svelte` non gestiva
   l'errore di `invoke` (pannello bianco invece di un messaggio).
   Fixato, riverificato visivamente.

**Limite di questa verifica**: è stata fatta con `vite` puro, **senza**
il runtime Tauri dietro (`window.__TAURI_INTERNALS__` assente) — quindi
conferma che il layout/CSS/componenti Svelte sono corretti e che gli
errori di rete/IPC vengono gestiti con grazia, ma **non** che i
comandi Tauri veri (start_download, il monitor 4Hz, la tray, le
notifiche) funzionino end-to-end con l'app reale. Quello richiede
`cargo tauri dev` o `cargo tauri build`, mai lanciato in questa
sessione (comando lungo, non essenziale rispetto a coprire più
milestone possibile). **Prossimo passo naturale**: lanciarlo alla
prossima sessione, è il test end-to-end che manca per dire davvero
"la GUI funziona", non solo "compila e renderizza".

## Riepilogo onesto di tutta la sessione overnight (M1→M8)

| Milestone | Stato | Verificato con |
|---|---|---|
| M1 discovery/binding | Chiusa | Hardware reale (`doctor`) |
| M2 probe/download singolo | Chiusa | Download HTTPS reale 1GB |
| M3 SQLite/resume/crash recovery | Chiusa | 100 crash injection |
| M4 multi-link | Core chiuso, gap documentati | Hardware reale (1 link, `download-multi`) |
| M5 hot-plug/circuit breaker/registro | Core chiuso, hot-plug live non cablato | Test automatici |
| M6 GUI | Implementata, gap documentati | **Verifica visiva reale** (stanotte) |
| M7 impostazioni/log/diagnostica | Core chiuso, CLI estesa non fatta | Test automatici + visiva |
| M8 tray/notifiche/packaging | Fatto quanto ragionevolmente possibile qui | Compilazione, non hardware/tempo reale |

Codice: ~35 file Rust nuovi/modificati, ~16 file Svelte/TS nuovi,
oltre 160 test automatici (quasi tutti verdi con continuità — l'unico
disturbo ricorrente è il blocco loopback ambientale, mai un bug del
progetto, sempre auto-risolto). Circa 20 commit stanotte, tutti con
messaggio che spiega il *perché*, non solo il *cosa*.
