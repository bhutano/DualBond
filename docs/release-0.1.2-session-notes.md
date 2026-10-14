# Note di sessione — DualBond 0.1.2

## Stato al 2026-08-21

Questa sessione ha prodotto una revisione ampia della UI/UX, metriche più
leggibili e la correzione del falso negativo che escludeva il tethering USB
Samsung dalla schedulazione multi-link.

## Problema Samsung: causa e correzione

- Hardware verificato durante la sessione:
  - Wi-Fi: `ifIndex 11`.
  - Telefono Android via USB RNDIS: `ifIndex 23`, IP `10.x.x.x`, gateway
    `10.x.x.y`.
- Entrambi i link erano abilitati nel registro dell'app.
- Un probe manuale vincolato con `IP_UNICAST_IF` al Samsung ha confermato che
  Windows instrada davvero traffico su quell'interfaccia.
- Il vecchio validatore inviava pacchetti da soli 32 byte e dichiarava valido
  il link con il maggiore incremento globale di `OutOctets`. Il normale
  traffico in background del Wi-Fi poteva quindi "vincere" e far escludere
  erroneamente il Samsung.
- Il nuovo validatore misura il delta dell'interfaccia attesa e lo confronta
  con il volume di payload che il socket ha effettivamente accettato. Il
  traffico contemporaneo sugli altri link resta disponibile come diagnostica,
  ma non invalida più il binding corretto.
- Il probe usa ora payload da 1200 byte ogni 50 ms, rendendo il segnale nei
  contatori inequivocabile senza generare traffico rilevante.
- Verifica su hardware reale dopo la correzione:
  - `[11] Wi-Fi (IPv4): OK (binding corretto)`.
  - `[23] Ethernet 2 / Samsung (IPv4): OK (binding corretto)`.

## UI/UX e metriche

- Dashboard e flusso download ridisegnati per rendere più chiari stato,
  azioni principali e link realmente attivi.
- Dopo annullamento o completamento è possibile inserire subito un nuovo URL,
  senza riavviare l'app.
- Aggiunti rilevamento periodico delle connessioni e azione manuale
  `Rileva ora`; i link offline sono separati da quelli vivi.
- Velocità istantanea stabilizzata con EWMA e campionamento iniziale corretto,
  evitando il picco prodotto dal primo snapshot dei contatori cumulativi.
- Aggiunte media complessiva a 10 secondi e medie per singolo link in tempo
  reale.
- Finestra Tauri e layout resi più adattabili alle diverse dimensioni.
- Versione applicazione e bundle aggiornata a `0.1.2`.

## Verifiche completate

- `cargo test --quiet`: 169 test totali, 165 superati, 4 ignorati, 0 falliti.
- `npm run build`: completato.
- `git diff --check`: nessun errore.
- `npm run tauri build`: completato; generati NSIS e MSI.
- Installer NSIS:
  `src-tauri/target/release/bundle/nsis/DualBond_0.1.2_x64-setup.exe`.
- SHA-256 installer NSIS:
  `D40935102B3B2FA963228083DAFC636049F1095596B39894050962BC1A7425CC`.

## Limite architetturale ancora aperto

DualBond accelera i download gestiti direttamente dall'app distribuendo le
richieste sui link validati. Non è ancora una sorgente Internet trasparente per
l'intero sistema operativo: per aggregare automaticamente browser e altre app
servirebbe un livello VPN/proxy/TUN di sistema, con routing, DNS, gestione TCP
e un endpoint remoto capace di ricomporre i flussi. Questa funzione non fa
parte della release 0.1.2.
