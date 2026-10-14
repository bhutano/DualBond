# DualBond 1.0.0 — Note di consegna

Data: 22–24 agosto 2026

## Risultato

DualBond ora offre una modalità di sistema controllata da un solo switch. Un servizio Windows `LocalSystem` crea il tunnel Wintun, cattura il traffico IPv4 TCP/UDP e assegna ogni nuovo flusso a Ethernet, Wi-Fi o tethering USB abilitati. Le connessioni vengono aggiornate automaticamente ogni due secondi e possono essere rilevate anche manualmente.

La schermata mostra velocità istantanea e media complessiva, velocità istantanea e media per collegamento, flussi attivi e stato dei percorsi. Un link disattivato interrompe i suoi flussi e viene azzerato subito; lo spegnimento generale azzera subito tutte le metriche e rimuove entrambe le route /1.

Il downloader segmentato rimane disponibile come funzione secondaria per sommare realmente la banda sullo stesso file quando il server supporta HTTP Range.

## Collaudo

- Suite principale: 193 test superati, 4 test diagnostici ignorati, 0 fallimenti.
- Crash-injection del download riprendibile: 100 cicli, test superato.
- Frontend: build di produzione superata.
- Rust: `cargo check --all-targets` e clippy superati.
- Runtime: servizio installato e in esecuzione automatica; tunnel attivo; 2 route /1 presenti durante l'uso e 0 dopo lo stop.
- Rete reale: download HTTPS completati e byte attribuiti sia a Ethernet sia a Wi-Fi.
- Controlli live: spegnimento di un link durante un trasferimento verificato; flusso interrotto e metriche del link azzerate.

## Correzione runtime del 24 agosto 2026

Un test sull'istanza installata ha riprodotto il mancato caricamento di due
pagine concorrenti: entrambe scadevano dopo 12 secondi. Windows conservava
l'ultimo indirizzo DHCP e la route del Wi-Fi disconnesso; la discovery non
controllava `OperStatus` e lo scheduler continuava quindi ad assegnare flussi
al link non raggiungibile. Il log mostrava `WSAEHOSTUNREACH` (`os error 10065`)
su `10.x.x.x`.

La correzione, commit `c24695a`, comprende:

- esclusione degli adapter il cui stato operativo non è `Up`, anche quando
  conservano IP e route stale;
- esclusione dei link penalizzati finché è disponibile un'alternativa sana;
- rifiuto del traffico IPv6 prima dello scheduler della modalità IPv4-only,
  evitando penalizzazioni spurie dei link IPv4;
- log persistente del servizio in
  `C:\ProgramData\DualBond\service.log`, con link selezionato ed esito dei
  flussi TCP/UDP.

Collaudo della correzione: 26 test del bilanciamento e 14 test della discovery
superati. Sul servizio installato, lo stesso test concorrente ha restituito
HTTP 200 per entrambe le pagine, con zero nuovi errori e solo il tethering USB
tra i link schedulabili mentre il Wi-Fi era disconnesso. Al termine della
sessione UI e servizio sono stati chiusi intenzionalmente; `DualBondService`
risulta `STOPPED` e non ci sono processi `dualbond.exe` attivi.

## Limiti intenzionali

Il bilanciamento generale è per flusso: non spezza una singola sessione TLS/QUIC o un singolo flusso Steam tra più IP pubblici. IPv6 resta sul percorso nativo nella 1.0.0. Questi vincoli sono mostrati nell'interfaccia e non richiedono server remoto o intercettazione TLS.

## Artefatto

Installer: `src-tauri/target/release/bundle/nsis/DualBond_1.0.0_x64-setup.exe`

Dimensione: 5.023.583 byte.

SHA-256: `4DE71C5A69B1CD981494E321EC68BA25C84D683613DAAAEC2EE4AA9C8CDB2F5F`.

L'installer applicativo non è firmato; Windows SmartScreen può quindi mostrare un avviso. La DLL Wintun inclusa conserva una firma Authenticode valida.
