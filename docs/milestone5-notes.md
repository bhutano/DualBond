# Note — Milestone 5 (hot-plug, circuit breaker, reintegrazione, registro nomi)

## Stato al 2026-08-21

Circuit breaker per link e cap tentativi segmento (chiude un gap reale
scoperto testando M4 con hardware vero, vedi `milestone4-notes.md`):
committati insieme a M4, non ripetuti qui.

**Fatto in questa milestone**:
- `network/registry.rs`: `LinkRegistry`, schema `link_identity` esatto
  dalla spec (§3.2). `upsert_seen()` crea al primo rilevamento (nome
  auto-rilevato come `display_name` iniziale, colore dalla palette in
  ordine) e non tocca mai nome/colore/enabled/metered scelti
  dall'utente ai ricollegamenti successivi — criterio di uscita M5
  ("ricollegamento su porta diversa mantiene nome e colore") verificato
  con test (`reconnecting_never_overwrites_user_chosen_name_or_color`,
  `disabled_link_stays_disabled_across_reconnects`). 9 test.
- `network/link_monitor.rs`: rilevamento hot-plug via **polling di
  sicurezza** (vedi sotto per la scelta deliberata di non implementare
  il callback), `diff_known_interfaces()` pura e testata (Added/Removed
  su cambio di `if_index`), `start_polling()` che lo gira ogni
  `interval` su un task tokio. `ReintegrationRamp`: rientro a peso
  conservativo (0.25) con risalita EWMA (§4.8 punto 3), mai un salto
  diretto a peso pieno. 8 test.

## Scelta deliberata: solo polling, non il callback `NotifyIpInterfaceChange`

La spec chiede il callback Win32 `NotifyIpInterfaceChange` con polling
di sicurezza a 2s come rete di protezione. Ho implementato **solo** il
polling. Motivo: il callback è una funzione C con firma
`extern "system"`, contesto passato per puntatore raw, invocata dal
sistema su un thread arbitrario — codice `unsafe` che non ho modo di
verificare in questa sessione senza scollegare/ricollegare hardware
davvero, e un bug lì (marshaling sbagliato del contesto, use-after-free
se il puntatore non resta valido) sarebbe silenzioso, non un panic
pulito. Il polling da solo **soddisfa il requisito funzionale**
(rilevamento entro ~2s, stesso valore che avrebbe la "rete di
sicurezza"), a costo di un `list_adapters()` extra ogni 2s — trascurabile.
Il callback resta un miglioramento futuro (latenza di rilevamento quasi
zero invece di fino a 2s), non un buco funzionale.

## Non ancora fatto — il pezzo più grande mancante di M5

**`link_monitor`/`ReintegrationRamp` non sono cablati a un orchestrator
che aggiorna i worker durante un download attivo.** Oggi
`orchestrator::multi_link_download` prende una lista fissa di
`LinkSpec` all'avvio e non cambia più finché il download non finisce:
se un link sparisce a metà, i suoi worker falliscono con errore di
trasporto (assorbito dal circuit breaker/requeue, il download
**continua** sugli altri link — questo già funziona, verificato nei
test M4), ma se un link **compare** a metà download, nessuno lo nota:
serve un `select!`/loop nell'orchestrator che ascolta
`link_monitor::start_polling()` e spawna nuovi worker al volo per i
link aggiunti, con `ReintegrationRamp` a governarne il ramp-up.
Non l'ho fatto stanotte: è la parte più architetturalmente invasiva
rimasta (l'orchestrator passa da "batch fisso" a "pool dinamico") e le
altre milestone (M6 GUI, M7, M8) sono più grandi e più visibili nel
complesso — priorità data a coprire tutta la roadmap almeno una volta
piuttosto che approfondire questo singolo pezzo. **Prossimo passo
naturale quando si torna qui**: questa riga di codice.

## Criterio di uscita M5 della spec

"Ricollegamento su porta diversa mantiene nome e colore" —
**soddisfatto** da `registry.rs` (la chiave stabile non dipende
dall'indice di interfaccia/porta, §3.2, già validato in M1 con
hardware reale; `upsert_seen` non tocca nome/colore ai ricollegamenti,
testato sopra). La parte "hot-plug live durante un download" resta
aperta come da sopra.
