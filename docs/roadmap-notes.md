# Note di roadmap

Decisioni prese fuori dalla spec formale, valide finché non superate da
una nuova discussione esplicita.

## Milestone 8 — stress test differito

Data: 2026-08-20.

L'utente non deve rilasciare DualBond a breve — è uso personale, non
c'è deadline di pubblicazione. Il criterio di uscita della Milestone 8
(file >100 GB, rete instabile) richiede uno sforzo di validazione reale
(ore di download, simulazione instabilità rete), sproporzionato rispetto
al codice che aggiunge (tray icon, notifiche, packaging MSI + portable
sono piccoli). Decisione: **non pianificare né dimensionare quel test ora
— si deciderà quando/se arriverà il momento di un rilascio vero.** Le
altre milestone (2-7) procedono nell'ordine della spec (Parte 11), incluso
il vincolo esplicito lì dichiarato: resume/crash-recovery (M3) prima del
multi-link (M4), perché costruire lo scheduler su una persistenza non
ancora verificata rende impossibile distinguere bug dello scheduler da
bug di durabilità.
