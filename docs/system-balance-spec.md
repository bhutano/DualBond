# DualBond 1.0 — Specifica “Bilanciamento di sistema”

Data: 22 agosto 2026
Stato: implementata e verificata

## 1. Esperienza utente

La schermata principale espone una sola scelta primaria: **Bilanciamento di sistema**. Dopo l'installazione il servizio è pronto ma il tunnel si attiva solo quando l'utente abilita lo switch. La scelta persiste.

Quando attivo:

- tutto il traffico IPv4 TCP e UDP di Windows passa dall'adattatore virtuale DualBond;
- ogni nuovo flusso viene assegnato automaticamente a un link abilitato e sano;
- collegamenti aggiunti o rimossi sono rilevati entro due secondi;
- i flussi già attivi restano sul proprio link finché possibile; se il link sparisce, il flusso termina in modo pulito e l'applicazione originaria può riconnettersi su un altro link;
- la UI mostra stato del servizio, traffico combinato istantaneo e medio, traffico per link, flussi attivi e ultimo errore;
- il download URL integrato resta disponibile come acceleratore per singoli file divisibili.

## 2. Modello di rete

### 2.1 Adattatore virtuale

- Nome: `DualBond`.
- IPv4 locale: `10.77.0.1/30`.
- MTU: 1500.
- Route catturate: `0.0.0.0/1` e `128.0.0.0/1` con metrica bassa. Le due /1 prevalgono sulla default route fisica senza cancellarla.
- La configurazione 1.0 non cattura IPv6. Windows conserva IPv6 nativo sui link fisici; la UI lo dichiara. Questo evita perdite silenziose finché il percorso IPv6 e il rollback non sono validati end-to-end.

### 2.2 Uscita fisica

Ogni socket TCP/UDP outbound è:

1. creato nella stessa famiglia dell'indirizzo remoto;
2. configurato con `IP_UNICAST_IF` o `IPV6_UNICAST_IF`;
3. bindato all'indirizzo locale della scheda scelta;
4. connesso direttamente alla destinazione originale.

Il binding esplicito impedisce che le route /1 del tunnel catturino anche l'uscita del servizio e generino un loop.

### 2.3 TCP

Lo stack user-mode accetta la connessione proveniente da Windows, sceglie un link una sola volta e apre il socket remoto. Dopo il connect esegue copia bidirezionale con backpressure. Timeout connect: 10 secondi. Nessuna migrazione a metà flusso. Se l'utente disabilita il link, i flussi assegnati vengono chiusi e l'applicazione può riconnettersi su un altro percorso.

### 2.4 UDP

Ogni coppia sorgente/destinazione crea una sessione UDP vincolata al link scelto. I datagrammi e le risposte conservano i confini. Idle timeout: 60 secondi. DNS e QUIC sono trattati come UDP ordinario. Nessuna promessa di migrazione di una sessione QUIC esistente.

## 3. Scheduler

Lo scheduler usa weighted least-connections:

`score = (active_flows + 1) / effective_weight`

`effective_weight` combina:

- preferenza utente: link disabilitato = escluso;
- stato fisico: offline = escluso;
- costo: link a consumo escluso salvo assenza di alternative o consenso esplicito;
- salute recente: penalità esponenziale dopo connect error;
- capacità osservata: EWMA dei byte inoltrati, limitata per evitare oscillazioni;
- reintegrazione: peso iniziale 0,25 dopo un hot-plug, poi crescita graduale.

I pareggi sono risolti round-robin. Il contatore active flow è decrementato tramite guardia RAII anche su errore o cancellazione.

## 4. Sicurezza e controllo

- Il servizio gira come `LocalSystem` perché gestione Wintun e route richiede privilegi.
- La UI non esegue comandi shell arbitrari nel servizio.
- Il controllo è un protocollo JSON a righe su `127.0.0.1`, autenticato da un token casuale a 256 bit creato all'installazione/primo avvio e salvato in `%ProgramData%\DualBond\control.token`.
- Comandi ammessi: status, enable/disable system mode, refresh, enable/disable link e set metered.
- Payload massimo 64 KiB; parsing stretto; timeout 3 secondi; nessun URL o comando processo accettato.
- I log non contengono token, contenuto del traffico o URL completi.

## 5. Stato e rollback

File:

- `service-config.json`: preferenze persistenti e schema versionato;
- `service-state.json`: snapshot runtime sostituito atomicamente;
- `control.token`: segreto locale;
All'avvio il servizio elimina eventuali route DualBond residue e ricrea lo stato desiderato. Allo stop disattiva le route prima di chiudere l'adattatore. Le route non sono persistenti (`route -p` è vietato), quindi un crash o riavvio non può lasciare il PC senza rete in modo permanente.

## 6. Installazione

Installer NSIS per macchina:

1. richiede elevazione una sola volta;
2. installa eseguibile e `wintun.dll` in Program Files;
3. registra `DualBondService` con avvio automatico;
4. avvia il servizio;
5. su upgrade ferma il servizio prima di sostituire i file e lo riavvia dopo;
6. su disinstallazione ferma/elimina il servizio, rimuove route residue e adattatore.

Il pacchetto MSI non è il deliverable primario perché gli hook del servizio sono definiti per NSIS.

## 7. Contratto IPC UI

Nuovi comandi Tauri:

- `get_system_balance_status() -> SystemBalanceStatus`
- `set_system_balance_enabled(enabled) -> SystemBalanceStatus`
- `refresh_system_links() -> SystemBalanceStatus`
- `set_system_link_enabled(key, enabled) -> SystemBalanceStatus`
- `set_system_link_metered(key, metered) -> SystemBalanceStatus`

La UI effettua polling dello stato servizio ogni secondo; il monitor download Tauri continua a usare l'evento `stats` a 4 Hz. Un servizio non raggiungibile viene mostrato come errore ripristinabile, mai come tunnel attivo.

## 8. Osservabilità

Per servizio e per link:

- byte in/out;
- Mbps istantanei su finestra 1 s;
- Mbps medi EWMA 10 s;
- flussi TCP/UDP attivi;
- connect error;
- stato online/degraded/offline/disabled;
- timestamp ultimo aggiornamento;
- errore operativo più recente, redatto.

## 9. Limiti dichiarati in prodotto

- Bilanciamento per flusso, non bonding di una singola sessione cifrata.
- IP pubblico diverso tra flussi se i link usano operatori diversi.
- IPv6 non catturato nella 1.0.
- App che vincolano autonomamente un'interfaccia o usano driver propri possono bypassare il tunnel.
- Accelerazione dello stesso file disponibile nel downloader DualBond quando il server supporta Range.

## 10. Test di accettazione

- Scheduler: esclusione disabled/offline, fallback metered, fairness, penalità e reintegrazione.
- Config: default sicuro, migrazione schema, scrittura atomica, input corrotti.
- Route: comandi esatti, idempotenza, cleanup sempre prima dello start e allo stop.
- TCP locale: forma del proxy bidirezionale, chiusura e contatori.
- UDP locale: famiglie IP, inoltro, timeout e contatori.
- Controllo: token errato, payload grande, comando sconosciuto, status e toggle.
- UI: build TypeScript/Svelte senza errori, stati service unavailable/idle/active/error.
- Regressione: suite Rust preesistente e build frontend.
- Packaging: release build, presenza `wintun.dll`, registrazione hook NSIS, installer generato e hash SHA-256.

## 11. Esito release 1.0.0

- 193 test automatici verdi nella suite principale, 4 test diagnostici di rete ignorati e nessun fallimento.
- Test crash-injection ×100 eseguito separatamente e superato: totale effettivo 194 test verdi.
- Build Svelte/TypeScript e lint Rust superati.
- Installer, upgrade, servizio automatico, Wintun, traffico HTTPS reale, uso simultaneo Ethernet + Wi-Fi, spegnimento link, azzeramento immediato e rollback delle route verificati su Windows.
