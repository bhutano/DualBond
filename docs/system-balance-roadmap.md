# DualBond 1.0 — Roadmap esecutiva

Aggiornata: 22 agosto 2026

| # | Fase | Stato | Criterio di chiusura |
|---:|---|---|---|
| 1 | Consolidamento baseline 0.1.3 | Completata | Fix live-link committato, 174 test, installer 0.1.3 verificato |
| 2 | Brainstorm e ricerca architetturale | Completata | Alternative confrontate; scelta Wintun + stack user-mode + servizio Windows |
| 3 | Specifica di prodotto e tecnica | Completata | Scope, UX, sicurezza, rollback, installer e test di accettazione documentati |
| 4 | Core System Balance | Completata | Scheduler, config, stato, socket TCP/UDP, tunnel Wintun e rollback route implementati |
| 5 | Servizio, controllo e IPC | Completata | Servizio automatico, protocollo locale autenticato e cinque comandi Tauri integrati |
| 6 | UI minimale completa | Completata | Switch primario, stato, metriche e link vivi; acceleratore file ripiegato in un pannello secondario |
| 7 | Hardening e test completi | Completata | 194 test eseguiti e verdi (incluso crash-injection ×100), 4 debug-test ignorati; build frontend e clippy verdi; tunnel e switch link verificati dal vivo |
| 8 | Installer finale | Completata | NSIS per-machine, Wintun e servizio inclusi; installazione, upgrade e disinstallazione verificati realmente |
| 9 | Chiusura | Completata | Documentazione aggiornata, commit finale creato e hash installer registrato |

## Registro decisioni

- 2026-08-22: nessun relay remoto e nessun MITM TLS.
- 2026-08-22: il traffico generico viene bilanciato per flusso; il downloader Range gestisce i singoli file divisibili.
- 2026-08-22: Wintun evita lo sviluppo e la firma di un nuovo driver kernel.
- 2026-08-22: servizio privilegiato separato logicamente dalla UI, con protocollo di controllo ristretto.
- 2026-08-22: route IPv4 /1 non persistenti per rollback sicuro; IPv6 resta nativo nella 1.0.
- 2026-08-22: core System Balance completato con conteggio byte in tempo reale per direzione e failover sui nuovi flussi.
- 2026-08-22: UI ridotta a uno switch di sistema; il downloader Range resta accessibile ma secondario.
- 2026-08-22: gli adattatori virtuali senza route predefinita sono esclusi; i pareggi dello scheduler ruotano realmente tra link.
- 2026-08-22: spegnere il sistema azzera subito velocità, medie e flussi; spegnere un link interrompe i flussi assegnati e lo porta subito a zero.
- 2026-08-22: collaudo reale superato su Ethernet + Wi-Fi: entrambe hanno trasferito dati, le due route /1 sono state create e rimosse senza residui.
- 2026-08-22: installer definitivo generato (5.018.146 byte), SHA-256 `DAACB7A33FC9CC0246CA92DA3A48BE3C393779BE5679BF2030B070BF45E88283`.
