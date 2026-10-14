# DualBond 1.0 — Brainstorm e decisione architetturale

Data: 22 agosto 2026
Stato: chiuso

## Obiettivo reale

DualBond deve diventare una sorgente Internet unica per Windows, usando automaticamente più collegamenti fisici (Ethernet, Wi-Fi e tethering USB) con interazione minima. Il prodotto deve anche conservare il download HTTP segmentato già esistente, perché è l'unico modo locale per sommare davvero le velocità sullo stesso file quando il server supporta le richieste Range.

## Vincoli non aggirabili

- Senza un relay remoto non esiste un unico IP pubblico stabile: ogni nuova connessione esce con l'IP pubblico del collegamento scelto.
- Una singola connessione TCP/TLS/QUIC già aperta non può essere spezzata in modo trasparente tra IP pubblici diversi senza rompere stato, cifratura o anti-abuso del server.
- Il traffico generale può invece essere distribuito automaticamente per flusso. Browser, Steam e Windows non devono essere configurati uno per uno.
- Un file HTTP/HTTPS può usare più link contemporaneamente solo a livello applicativo, aprendo più richieste Range e ricomponendo i segmenti. Il motore download interno lo fa già senza intercettare o decifrare il traffico di altre applicazioni.
- Non viene introdotto un certificato root e non viene eseguito un MITM TLS globale.

## Alternative considerate

### Modificare le metriche delle route fisiche

Semplice e nativo, ma Windows tende a scegliere un percorso prevalente. Non dà un bilanciamento adattivo per flusso, un controllo affidabile di failover o metriche attribuibili a DualBond. Scartato come motore principale.

### Windows Filtering Platform con driver callout

Può intercettare e reindirizzare connessioni, ma un callout kernel firmato aumenta enormemente rischio, tempi di manutenzione e superficie di sicurezza. La firma EV/attestation non può essere prodotta dal repository. Scartato per la prima release completa.

### Proxy di sistema WinHTTP/WinINET

Non copre tutto il traffico: molte applicazioni ignorano il proxy di Windows, UDP e QUIC non passano da un proxy HTTP tradizionale. Scartato come fondazione.

### Adattatore virtuale Wintun più stack TCP/IP in user mode

Windows instrada il traffico verso un adattatore L3 firmato. Un servizio locale termina i flussi virtuali e apre socket reali vincolati al link scelto tramite `IP_UNICAST_IF`/`IPV6_UNICAST_IF`. Copre TCP e UDP, consente failover sui nuovi flussi, non richiede server remoto e riusa la validazione hardware già presente. Scelto.

### Estensione browser obbligatoria

Utile solo per consegnare automaticamente un URL scaricabile al motore Range. Non migliora il bilanciamento del traffico generale e i browser non permettono un'installazione silenziosa consumer affidabile. Rimane opzionale e fuori dal percorso critico della modalità Sistema.

## Decisione

L'architettura 1.0 è ibrida:

1. `DualBondService`, servizio Windows ad avvio automatico, possiede adattatore virtuale, route, stack TCP/UDP, selezione dei link, rollback e stato runtime.
2. `DualBond` è l'interfaccia minima. Mostra uno switch “Bilanciamento di sistema”, connessioni vive, traffico istantaneo e medio, stato e diagnostica.
3. Il traffico generico viene assegnato per flusso al link sano con il miglior punteggio pesato. Tutti i pacchetti dello stesso flusso restano sullo stesso link.
4. Il download HTTP segmentato resta disponibile per sommare i link sullo stesso file quando tecnicamente divisibile.
5. Il servizio non promette bonding di una singola sessione cifrata e non nasconde la pluralità degli IP pubblici.

## Componenti scelti

- Wintun 0.14.1, driver TUN firmato e distribuito tramite `wintun.dll` ufficiale.
- crate `tun` per la sessione Wintun asincrona.
- crate `ipstack` per trasformare pacchetti L3 in flussi TCP/UDP user-mode.
- socket2 e le API Winsock già presenti per vincolare ogni uscita a `if_index` e IP locale.
- servizio Windows nello stesso eseguibile, attivato con l'argomento interno `system-service`.
- file di stato/configurazione atomici in `%ProgramData%\DualBond`, con protocollo di controllo locale autenticato.

## Criterio di completezza

La release è completa quando installazione/disinstallazione gestiscono il servizio, Wintun e il rollback; TCP e UDP vengono inoltrati sui link selezionati; il motore non crea loop; uno stop o un crash non lascia route persistenti; UI, test unitari, test di integrazione locali, build release e installer NSIS passano.
