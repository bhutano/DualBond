# Note — Milestone 4 (multi-link: work-stealing, tail-splitting, AIMD)

## CORREZIONE (2026-08-21): il WSAEFAULT NON era ambientale

Le sezioni sotto ("Falso allarme, non un bug", occorrenze del blocco
loopback) concludevano che il `WSAEFAULT` osservato testando con
hardware reale fosse transitorio/ambientale. **Era sbagliato.**

Root cause vero, trovato il 21/08 testando di nuovo con un download
reale da 10GB scelto dall'utente: `main.rs`/`cli.rs` sceglievano
`local_ip` con `c.ips.first()` — e per il link Wi-Fi reale, il primo
IP nella lista di Windows è l'IPv6 **link-local** (`fe80::...`), non
l'IPv4 pubblico. Un indirizzo link-local non è instradabile su
internet: bindarci sopra una connessione verso un host reale fallisce,
con un errore basso livello poco leggibile (`WSAEFAULT`) invece che un
fallimento pulito — e sembra "intermittente" perché l'esito dipende da
quale famiglia (IPv4/IPv6) il resolver DNS restituisce quel momento
per l'host di destinazione, che varia.

Due fix, entrambi con TDD, in `network::discovery::preferred_local_ip`
(sceglie IPv4 globale, poi IPv6 globale, mai link-local — sostituisce
`ips.first()` ovunque) e `network::connector::pick_matching_family`
(anche scegliendo l'IP giusto, il resolver può restituire un indirizzo
di famiglia diversa da quella del link: va scartato, non usato alla
cieca). **Verificato con lo stesso identico scenario che falliva**:
download reale da 10GB, stesso link Wi-Fi, byte reali scritti su
disco entro 30s invece di WSAEFAULT ripetuti.

Le sezioni sotto restano come promemoria di *come* si è arrivati alla
diagnosi sbagliata la prima volta (utile: la tecnica di isolare con
test manuali `#[ignore]` era comunque corretta, la conclusione tratta
da quei risultati era incompleta — un singolo tentativo isolato con
`local_ip` **hardcoded manualmente all'IPv4 corretto** nel test non
esercitava affatto il bug, che vive nella selezione automatica).

## Stato al 2026-08-20 (sessione overnight)

Prima chiusa la coda aperta di Milestone 3 (vedi `milestone3-notes.md`):
`http_client.rs` mappava connect/handshake/send falliti su
`DualBondError::Http` (mai ritentabile). Ora usano `Transport`, e
`resumable::download_ranged_resumable` ritenta fino a 3 volte solo su
`Transport` via `http_client::send_once_with_retry`. 64/64 test verdi
subito dopo, incluso il crash-injection che era flaky. M3 è chiusa.

### Moduli nuovi di Milestone 4, tutti con test TDD

- **`download/chunk.rs`**: `segment_size_bytes(ewma_bps, rtt)`, formula
  clamp target_time/BDP/64MiB di §4.4. 6 test.
- **`download/concurrency.rs`**: `AimdController`, 4→16 connessioni per
  link, dimezza su errori/429/503/regressione goodput, isteresi al
  plateau. 10 test. **Non ancora cablato** al ciclo di vita del worker
  pool: il retuning è calcolabile ma nessuno lo chiama in corsa.
- **`download/scheduler.rs`**: `Scheduler`, coda condivisa di segmenti,
  `lease_next`/`release_lease`/`complete`/`report_progress`,
  `try_steal_tail` (tronca atomicamente il lease del segmento col
  maggior residuo, assegna la seconda metà al worker idle — nessun
  buco, nessuna sovrapposizione, verificato esplicitamente nei test).
  13 test, tutta logica pura in memoria.
- **`network/socket_bind.rs`**: `bind_tcp_socket_to_interface`, condivide
  `IP_UNICAST_IF` con la variante UDP di Milestone 1.
- **`network/connector.rs`**: `LinkConnector::{Bound,Plain}`. `Bound`
  apre TCP bindato al link via `spawn_blocking` (il connect di socket2
  è bloccante). `Plain` usa la route di default — è quello che i test
  usano per impersonare più "link" sullo stesso host locale. **`Bound`
  non è testabile in automatico in questo ambiente**: serve hardware
  multi-NIC reale, stesso vincolo già affrontato in Milestone 1
  (validato allora a mano con `cargo run -- doctor`). Da rifare a mano
  con USB tethering/hotspot reali prima di considerare il binding
  realmente provato per M4.
- **`storage/state_db.rs`**: `create_download_with_segments`,
  `list_chunks`, `insert_chunk`, `update_chunk_end`, `set_chunk_state`
  — persistenza N-chunk-per-download richiesta dallo scheduler.
- **`download/worker.rs`**: `run_worker`, ciclo per-link: lease o
  tail-steal, GET Range con retry via `send_via_connector_with_retry`,
  barriera di durabilità (fsync + DB) a fine segmento,
  `scheduler.report_progress`+`complete`. Su fallimento di trasporto
  dopo i retry: segmento rimesso in coda (`release_lease` +
  `set_chunk_state('free')`), non abort del download intero (§4.7). Su
  status inatteso o ETag divergente: il worker (cioè quel link) esce
  con errore, gli altri continuano.
- **`download/orchestrator.rs`**: `multi_link_download()` — probe,
  crea `Scheduler` + righe DB multi-segmento, spawna N worker per link
  (`LinkSpec.connections`, bootstrap fisso a 4 se non specificato
  altrove — non ancora, vedi sotto), attende tutti, verifica SHA-256.

### Verificato end-to-end

Test `two_links_download_the_same_file_concurrently_and_produce_correct_hash`:
2 link × 2 connessioni = 4 worker, body 15 MB (> soglia bootstrap di
10 MB, quindi almeno 2 segmenti reali, non uno solo preso dal primo
worker), tutti via `LinkConnector::Plain` verso lo stesso server di
test locale. Hash finale corretto, contesa reale sulla coda condivisa.
104/104 test verdi nell'intero progetto dopo l'aggiunta.

## Blocco ambientale, occorrenza più lunga (fine sessione)

Verso la fine della sessione il blocco è ricomparso una quarta volta e
stavolta **non si è auto-risolto entro ~20 minuti** (le tre volte
precedenti si erano risolte da sole in pochi minuti). Confermato
ancora una volta con lo stesso test nativo `HttpListener` in
PowerShell puro (`GetContextAsync().Wait(6000)` mai completato) —
100% indipendente dal codice del progetto. La suite `cargo test`
completa era rimasta bloccata (CPU del processo ferma, zero
avanzamento per 5 minuti pieni), diverso dai casi precedenti dove
falliva rapidamente per timeout; qui `resumable::crash_injection_...`
da solo continua a fallire deterministicamente all'iterazione 0 con lo
stesso `timeout di connessione`.

**Non è una regressione del codice di stanotte**: tutti i 107 test
automatici + le verifiche manuali con hardware reale erano già passati
prima che questo blocco ricomparisse. **Prossima sessione**: prima di
toccare codice, rilanciare `cargo test --quiet` (o solo
`download::resumable::` per un check rapido) — se ancora rosso con lo
stesso pattern, ripetere la verifica nativa PowerShell sopra prima di
sospettare un bug reale.

## Blocco ambientale ricomparso (stesso di Milestone 3, non un bug)

A metà sessione il blocco `accept()` su loopback documentato in
`milestone3-notes.md` è ricomparso (stessi sintomi: timeout ~10s sulla
connect, confermato indipendente dal codice con un test nativo
`System.Net.HttpListener` in PowerShell puro che si blocca
identicamente). **Si è risolto da solo** dopo qualche minuto, come
l'altra volta — non è mai stato necessario alcun intervento. Se in una
sessione futura i test `download::*`/`network::connector::*` timeout
tutti insieme con lo stesso pattern, prima di sospettare un bug
verificare con un test nativo PowerShell (`HttpListener` +
`Invoke-WebRequest`, vedi `milestone3-notes.md` per lo script) e
aspettare: finora si è sempre risolto senza intervento entro la
sessione.

## Semplificazioni note, non bug — prossimi passi in ordine di priorità

1. **AIMD non cablato al worker pool**: `concurrency.rs` esiste ed è
   testato, ma nessuno lo chiama durante il download. Il numero di
   connessioni per link è fisso per tutta la durata
   (`LinkSpec.connections`, deciso a monte). Cablarlo richiede un
   sampler periodico (10s, §4.4) che legga goodput/RTT/err_rate per
   link e possa far salire/scendere il numero di worker attivi — non
   banale con l'attuale modello "un `tokio::spawn` per connessione", va
   deciso se un pool dinamico o un semaforo per link.
2. **Nessun resume da riavvio multi-segmento**: `multi_link_download`
   crea sempre un nuovo `download`. Il resume single-link
   (`resumable_download`, M3) resta la via valida per riprendere un
   download interrotto; unificare i due percorsi (rivalidazione ETag
   per-segmento su riavvio) è lavoro a parte.
3. **DNS non pinnato per link** (`resolver.rs` di §5 non esiste):
   `connector.rs` usa il resolver di sistema condiviso. Corretto nella
   stragrande maggioranza dei casi, non per anycast/split-horizon.
4. **Hot-plug assente** (`link_monitor.rs` di §5 non esiste): nessuna
   gestione di link che spariscono/ricompaiono a metà download (§4.7
   "Interfaccia scomparsa", §4.8). Se un link fisico cade a metà
   worker, il worker fallisce con `Transport` dopo i retry e il
   segmento torna in coda — gli altri link lo riprendono, quindi il
   download **completa comunque**, ma non c'è rilevamento attivo né
   riconnessione automatica del link caduto.
5. **Circuit breaker assente**: un link che fallisce ripetutamente non
   viene mai escluso/riprovato con probe leggeri a intervalli
   crescenti (§4.4) — continua a essere riprovato allo stesso ritmo
   degli altri.
6. **CLI/IPC non cablati**: `cli.rs` chiama ancora solo
   `download_to_file` (Milestone 2) — né `resumable_download` (M3) né
   `multi_link_download` (M4) sono raggiungibili da fuori i test. Non
   è un'anomalia: nemmeno M3 lo era prima d'ora. Cablare M4 alla CLI
   richiede `network::discovery` per ottenere `LinkCandidate` reali e
   costruire `LinkConnector::Bound` da essi — a quel punto la
   validazione con hardware reale (USB/hotspot) diventa finalmente
   possibile, come già fatto per `doctor` in Milestone 1.

**Aggiornamento — punto (6) fatto nella stessa sessione, non solo
consigliato**: `dualbond download-multi <URL>` cablato (`cli.rs` +
dispatch in `main.rs`), usa `doctor::collect_validated_candidates()`
(nuova: come `collect_report` ma ritorna anche i `LinkCandidate` con
`IpAddr` reali, persi nella serializzazione JSON di `DoctorReport`) e
costruisce `LinkConnector::Bound` solo per i link validati.

**Validato con hardware reale** (unico link disponibile in questo
ambiente: Wi-Fi `mac:xx:xx:xx:xx:xx:xx`, if_index 10,
`192.168.x.x`): `download-multi` ha rilevato il link, validato il
binding, aperto connessioni TCP reali bindate con `IP_UNICAST_IF`
verso `ash-speed.hetzner.com`, e scaricato segmenti reali con
successo — fino a incontrare un **429 vero** dal server (rate limit
dopo troppe connessioni concorrenti in rapida successione).

**Bug reale trovato e fixato testando con hardware vero** (mai
sarebbe emerso solo con `LinkConnector::Plain` su loopback):
`worker.rs` trattava qualunque status ≠ 206 come errore fatale,
compreso 429/503 — abortiva l'intero download invece di rispettare
`Retry-After` e rimettere il segmento in coda (§4.7). Fixato con TDD
(`test_server::Scenario::RateLimitedThenRangeSupported`, nuovo).
Insieme: backoff esponenziale con full jitter (`0,5s × 2^n`, cap 60s,
`http_client::full_jitter_backoff`) al posto del fisso 200ms nei due
retry loop di trasporto, come richiesto da §4.7 ma non ancora fatto
prima.

**Falso allarme, non un bug**: durante il primo tentativo di download
reale, i worker hanno fallito ripetutamente con `WSAEFAULT (os error
10014)` su `connect_timeout` — sembrava un bug serio (nessun byte
scritto su disco dopo 15 minuti, nessuna connessione TCP attiva,
processo silenziosamente bloccato). Diagnosticato a fondo con test
manuali isolati (`network::connector::tests::debug_*`, `#[ignore]`):
connect singolo, 20 connect sequenziali, 4 connect concorrenti — tutti
riusciti perfettamente. Il retry immediatamente successivo dello
STESSO path reale (`orchestrator::tests::
debug_real_multi_link_download_against_hetzner`) è andato a segno,
arrivando fino al 429 reale sopra. Conclusione: transitorio/ambientale
(terzo caso della sessione con lo stesso pattern del blocco loopback
di Milestone 3, che si era già auto-risolto due volte stanotte), non
un bug di codice — probabile effetto collaterale del burst di
centinaia di connect() falliti rapidissimi nel run precedente. **Se
ricompare**: non modificare `connector.rs` a caldo, prima riprovare
con i test `#[ignore]` isolati per confermare che sia ambientale come
le altre due volte.

**Gap reale scoperto grazie a questo, non ancora chiuso**: nessun
limite al numero di volte che un segmento può essere rimesso in coda e
ritentato — se un fallimento fosse *davvero* permanente (non
transitorio), il download girerebbe in eterno in silenzio invece di
fallire con un errore chiaro. Non ha causato il problema di stanotte
(che si è risolto da solo), ma resta un buco: manca un contatore di
tentativi totali per segmento con soglia oltre la quale si rinuncia
con un errore esplicito. Prossimo passo naturale insieme al circuit
breaker (punto 5 sotto).

**Prossimo passo consigliato ora**: (1) AIMD live, poi il contatore di
tentativi totali + circuit breaker (punto 5) — motivato da un
problema reale osservato stanotte, non più solo teorico.
