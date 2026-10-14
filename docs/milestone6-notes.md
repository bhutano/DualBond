# Note — Milestone 6 (interfaccia grafica)

## Stato al 2026-08-21

GUI completa implementata: monitor sempre attivo (§6.1), gauge radiale
SVG a mano con scala log e isteresi (§6.3), elenco link (§6.4), grafico
uPlot ad area impilata (§6.5), pannello download/URL bar (§6.2),
Impostazioni con gestione connessioni reale (§6.6), primo rilevamento
con gate di conferma esplicita (§7), tema chiaro/scuro/di sistema.

Build Vite pulita (0 errori). **Verifica visiva in browser non
completata questa sessione**: il blocco loopback ambientale (già
documentato 3 volte in M3/M4/M5) è tornato una settima volta e stavolta
non si è mai risolto da solo in oltre un'ora — ha impedito di
raggiungere sia i test `cargo test` sui moduli `download::test_server`
sia il dev server Vite (`curl localhost:5183` si blocca esattamente
come `HttpListener` nel test nativo PowerShell già usato per
diagnosticare le altre volte). **Prima cosa da fare alla prossima
sessione**: verificare se si è risolto, poi aprire l'app
(`npm run tauri dev` o `cargo tauri dev`) e controllare visivamente
gauge/grafico/pannelli — non sono mai stati visti a schermo.

## Gap noti, documentati invece che scoperti in silenzio

1. **`rttMs` sempre `null`**: il monitor (`metrics/monitor.rs`) non
   misura mai l'RTT — richiederebbe un probe periodico leggero per
   link (ping o mini-richiesta HTTP), non ancora scritto. La UI lo
   gestisce già correttamente come opzionale (non mostra la riga se
   `null`).
2. **`connections`/`errors` sempre 0** in `LinkStats`: il monitor non
   legge questi dati dai worker attivi (che pure li avrebbero, tramite
   lo scheduler/circuit breaker) — servirebbe un canale monitor↔worker
   non ancora costruito. `LinkCard.svelte` mostra questi campi ma
   restano a zero anche durante un download reale.
3. **`etaSec` sempre `null`**: `metrics/monitor.rs::download_stats` non
   lo calcola (richiede la velocità aggregata del solo download, non
   il traffico totale del link che include anche traffico esterno
   all'app). `DownloadPanel.svelte` lo stima lato frontend da
   `totalMbps`, impreciso se c'è traffico non-download sullo stesso
   link.
4. **`recentEvents` sempre vuoto**: nessun log applicativo strutturato
   ancora esiste (§6.5 "striscia con gli ultimi eventi rilevanti").
5. **Stato link limitato a online/offline/disabled**: `probing` e
   `degraded` (§6.4) non sono mai emessi dal monitor — la validazione
   vera (quella che potrebbe dire "non riesco a usarlo separatamente")
   resta un'azione a parte (`doctor`), non incorporata nel loop 4Hz
   (motivato in testa a `monitor.rs`: farla ogni 250ms bloccherebbe la
   UI per secondi).
6. **Gate di conferma link nuovo (§7), semplificazione**: la spec
   distingue "mai deciso" da "esplicitamente Non usare" (quest'ultimo
   non richiede mai più conferma). Qui entrambi condividono
   `enabled=false` — "Non usare" nella UI aggiunge solo la chiave a un
   elenco locale non persistito, quindi potrebbe ripresentarsi a un
   riavvio dell'app. Persistere la distinzione richiede un campo in
   più nello schema `link_identity` (non nella spec originale).
7. **Impostazioni → Download/Comportamento**: presenti nel mockup
   della spec ma non implementate in `Settings.svelte` (solo
   Connessioni e Aspetto sono reali). Nessun `config.toml` scritto
   dalla UI. Cartella di destinazione fissa (`download_dir()` di
   sistema), connessioni massime fisse a 4, nessun limite di velocità.
8. **SpeedTest.svelte riusa `doctor`**: nessun benchmark di throughput
   puro dedicato (`metrics/benchmark.rs` del file tree della spec non
   scritto) — mostra l'esito della validazione del binding, che
   include comunque un vero trasferimento di prova per link (§4.3),
   ma non è un vero "quanto è veloce questo link" isolato.
9. **Notifica di sistema a download completato, tray con velocità nel
   tooltip** (§6.8): non implementati — competenza di Milestone 8.

## Cosa invece È reale, non un mock

- Il monitor legge davvero `list_adapters()`+`get_if_counters()` ogni
  250ms e calcola Mbps/EWMA/picco/gainFactor/scaleMax (con isteresi)
  dai contatori di sistema veri.
- `start_download`/`pause`/`resume`/`cancel` guidano il vero
  orchestrator M4/M5 (worker reali, scheduler reale, circuit breaker
  reale).
- Rinomina/colore/abilita/disabilita/dimentica scrivono davvero nel
  `LinkRegistry` persistente (`app_data_dir/registry.db`).
- Il tema chiaro/scuro/di sistema è reale e persistito
  (`localStorage`).
