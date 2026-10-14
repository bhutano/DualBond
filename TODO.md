# TODO — prossima versione (1.0.3)

Problemi osservati e concordati il 29 agosto 2026 durante l'uso reale della
versione installata 1.0.2.

## P0 — Mostrare solo i collegamenti realmente disponibili

- [x] Conservare un solo tachimetro grande per la velocità complessiva, pari
  alla somma dei collegamenti utilizzabili.
- [x] Mostrare i tachimetri piccoli esclusivamente per i link abilitati che
  hanno una connessione presente, stabile e utilizzabile per Internet.
- [x] Considerare visibile un link stabile anche quando in quel momento non sta
  trasferendo byte e il suo tachimetro indica zero.
- [x] Nascondere completamente dalla schermata principale adattatori offline,
  disabilitati, solo locali o altrimenti inutilizzabili; devono riapparire e
  scomparire automaticamente al cambiare dello stato reale della connessione.
- [x] Lasciare che il totale e l'elenco visibile derivino dalla stessa fonte di
  verità sulla salute dei link, evitando discrepanze tra tachimetro principale
  e tachimetri secondari.

### Criterio di accettazione

Con Wi-Fi e tether/Ethernet entrambi stabili sono visibili un tachimetro totale
e due tachimetri piccoli, anche se uno dei due segna zero. Se un link perde
Internet, il relativo tachimetro piccolo scompare automaticamente e il totale
continua a rappresentare soltanto i link ancora utilizzabili.

## P0 — Rilevare l'accesso Internet reale per ogni link

- [x] Non considerare automaticamente `online` un adattatore solo perché è
  operativo e possiede un IP/gateway.
- [x] Distinguere almeno gli stati `online`, `solo rete locale`, `degradato` e
  `offline` nella UI e nello stato del servizio.
- [x] Evitare di assegnare nuovi flussi a un link senza accesso Internet quando
  esiste un'alternativa funzionante; mantenerlo disponibile per una nuova
  verifica e reintegrarlo automaticamente quando torna raggiungibile.
- [x] Non penalizzare definitivamente un intero link per il fallimento di un
  singolo endpoint remoto.
- [x] Aggiungere test per il caso osservato: tether USB Samsung con gateway
  raggiungibile e profilo Windows `LocalNetwork`, ma connessioni Internet in
  timeout.

### Criterio di accettazione

Con Wi-Fi funzionante e tether USB limitato alla rete locale, il tether deve
essere mostrato come non utilizzabile per Internet e nessun nuovo download deve
restare in attesa del timeout del tether. Quando Internet torna disponibile sul
tether, questo deve rientrare automaticamente nello scheduler.

## P0 — Rendere atomica l'assegnazione dei flussi

- [x] Unire la scelta del link e l'incremento del relativo contatore in una sola
  operazione sotto lo stesso lock (per esempio `choose_and_start`).
- [x] Eliminare la finestra tra `FlowScheduler::choose` e `flow_started` che
  permette a più task simultanei di osservare gli stessi contatori.
- [x] Aggiungere un test concorrente con una raffica di nuovi flussi e due link
  equivalenti.

### Criterio di accettazione

Una raffica simultanea di connessioni non deve essere assegnata quasi interamente
allo stesso link a causa della race; a parità di salute e peso, la distribuzione
deve restare bilanciata.

## P0 — Impedire più istanze contemporanee dell'app

- [x] Consentire una sola istanza della UI DualBond per sessione utente.
- [x] Se l'utente avvia nuovamente l'app, portare in primo piano e ripristinare
  la finestra già aperta, quindi terminare la nuova istanza.
- [x] Non confondere il processo UI con `dualbond.exe system-service`: una UI e
  il servizio Windows devono continuare a poter funzionare insieme.
- [x] Gestire anche avvii quasi simultanei, finestra minimizzata nel tray e
  riapertura tramite collegamento Start/Desktop.
- [x] Aggiungere test/manual check per avvio doppio, tray, servizio attivo e
  aggiornamento tramite installer.

### Criterio di accettazione

Con il servizio attivo, qualunque numero di avvii dell'app deve lasciare un solo
processo UI e un solo processo di servizio; gli avvii successivi devono mostrare
la finestra esistente senza perdere lo stato corrente.
