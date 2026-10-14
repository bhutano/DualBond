# Note di sessione — DualBond 0.1.3

## Stato al 2026-08-21

La release 0.1.3 corregge il comportamento degli switch durante un download
attivo e rende le metriche coerenti con il traffico realmente gestito da
DualBond.

## Correzioni

- `set_link_enabled` aggiorna sia il registro persistente sia i controlli
  condivisi dai worker del download corrente.
- Disabilitare un link interrompe connect, retry o body HTTP in corso entro
  un polling breve; il segmento torna libero e può essere ripreso da un altro
  collegamento senza avanzare il watermark durevole.
- Un link validato ma disabilitato dispone di worker dormienti, quindi può
  essere riattivato durante lo stesso download.
- I worker dormienti terminano quando lo scheduler è completo e non bloccano
  la chiusura del download.
- Annullamento e disabilitazione interrompono realmente le richieste, non
  soltanto la rappresentazione grafica.
- Link disabilitati, download in pausa, annullati o terminati pubblicano
  velocità istantanea, media e picco pari a zero.
- Il sampler viene reimpostato sul contatore corrente, evitando che EWMA e
  media precedenti ricompaiano alla ripresa.
- Lo switch della dashboard mostra immediatamente `0 Mbps` e lo stato
  `Esclusa dal download` in attesa dello snapshot successivo.

## Verifiche

- Test reale simulato con un server TCP che accetta la connessione senza
  rispondere: lo switch chiude la richiesta in volo e libera il segmento.
- Test di completamento con un link attivo e uno inizialmente disabilitato.
- `cargo test --quiet`: 174 test totali, 170 superati, 4 ignorati, 0 falliti.
- `npm run build`: completato.
- `npm run tauri build`: completato; generati NSIS e MSI 0.1.3.
- SHA-256 NSIS:
  `A247A444758567219E424A127479682C6D87D71CBFDADE6C9A90C037C8BAEDFF`.
