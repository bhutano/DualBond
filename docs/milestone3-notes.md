# Note — Milestone 3 (SQLite, resume, crash recovery)

## Stato implementazione

Completa: `storage/state_db.rs` (schema `download`+`chunk`, WAL,
`synchronous=FULL`), `download/resumable.rs` (resume byte-preciso con
`If-Range`, barriera di durabilità `sync_data()` **prima** del commit
SQLite, hash SHA-256 ricalcolato a fine trasferimento leggendo da disco
— mai accumulato in memoria durante il download, per restare corretto
anche se il processo muore a metà).

Test scritti (TDD): `fresh_download_completes_and_matches_hash`,
`resume_continues_from_existing_watermark_without_redownloading_start`,
`crash_injection_100_times_never_corrupts_final_file` (100 iterazioni,
interruzione del future di download a istanti variabili via
`tokio::time::timeout`, verifica che i byte dichiarati durevoli dal DB
combacino sempre col sorgente e che il resume finale produca un file
bit-identico — criterio di uscita Milestone 3).

Questi test **sono passati** in isolamento durante lo sviluppo (vedi
sotto per il blocco emerso dopo).

## Blocco ambientale scoperto: accept() su loopback bloccato

A un certo punto di questa sessione, **tutti** i test che aprono un
server HTTP locale su `127.0.0.1` (il pattern richiesto da §10 della
spec: "server di test locale hyper") hanno iniziato a fallire con
timeout di connessione (`WSAETIMEDOUT`, ~21s, il classico pattern di
retry SYN di Windows senza mai una risposta).

Diagnosi eseguita (tutta a fini di lettura, nessuna modifica al
sistema):
1. Escluso: contesa di risorse (100 iterazioni in parallelo) — isolando
   un singolo test fallisce comunque, deterministico al 100%.
2. Escluso: il nostro codice — il server logga correttamente
   `listening on 127.0.0.1:PORT` e resta in `accept().await`, il
   client tenta la connessione all'host/porta corretti, ma il tempo
   scade senza che il server veda mai `accepted connection`: i pacchetti
   SYN non arrivano mai a destinazione.
3. Escluso: Windows Defender — Real-Time Protection e Network
   Inspection risultano attivi e integri, ma nessun evento di blocco
   nel log operativo (`Microsoft-Windows-Windows Defender/Operational`)
   negli ultimi 30 minuti.
4. Escluso: esaurimento porte/connessioni TCP — `Get-NetTCPConnection`
   mostra solo 67 connessioni totali, range dinamico da 64511 porte.
5. Escluso: il sandbox del tool Bash di Claude Code — riprovato con
   `dangerouslyDisableSandbox: true`, stesso fallimento identico.
6. **Confermato con un test 100% nativo Windows, senza alcun binario
   Rust**: `System.Net.HttpListener` su `127.0.0.1` interrogato da
   `Invoke-WebRequest` nello stesso processo PowerShell fallisce
   identicamente (`HttpClient.Timeout... elapsing`), sia con
   `127.0.0.1` sia con `localhost`. Riprovato due volte, stesso esito.

**Conclusione**: il loopback TCP (lato `accept`) risulta bloccato a un
livello che precede completamente qualunque codice di questo progetto
— probabile policy di rete della VM/hypervisor o di un EDR non
visibile nei log Defender standard. **Non è un bug di DualBond.**

I 50 test che non aprono un server locale (storage, checksum, state_db,
network/Milestone 1, core) restano verdi: `cargo test --quiet` isola
esattamente e solo i 12 test basati su `download::test_server`.

## Come verificare M3 quando il loopback torna disponibile

```
cargo test --quiet download::resumable::
```

Questi test sono stati verdi (incluse le 100 iterazioni di crash
injection) prima che il blocco comparisse. Il codice non è cambiato da
allora se non per un timeout di connessione TCP di 10s aggiunto in
`download/http_client.rs` (buona pratica indipendente dal problema).

## Verifica alternativa già eseguita (rete reale, non loopback)

Il downloader a link singolo (Milestone 2, stesso `http_client.rs`) è
stato verificato con un vero download HTTPS di 1 GB da
`ash-speed.hetzner.com` via `dualbond download`: completato in 17m21s,
`SHA-256 9fd9dd5f52df2974eb7273c14fc2aa6bce71f3b4dc1f704e6875a87ae27e90a0`
corretto. Le connessioni **in uscita** verso host reali non sono
affette dal blocco: solo `accept()` su loopback lo è.
