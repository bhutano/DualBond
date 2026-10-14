// Localizzazione UI (italiano / inglese). La chiave è il testo italiano:
// `t("Impostazioni")` restituisce "Settings" in inglese. Una chiave senza
// traduzione ricade sul testo italiano, quindi niente stringa rimane vuota.
// Placeholder `{nome}` sostituiti dai parametri passati.

export type Language = "it" | "en";

const STORAGE_KEY = "dualbond-language";

const EN: Record<string, string> = {
  // Barra superiore
  "Internet multi-link per Windows": "Multi-link Internet for Windows",
  "Monitor attivo": "Monitor active",
  "Connessione al motore…": "Connecting to engine…",
  "Diagnostica": "Diagnostics",
  "Impostazioni": "Settings",
  "Lingua": "Language",
  "Italiano": "Italiano",
  "English": "English",
  // App
  "Acceleratore per singoli file": "Single-file accelerator",
  "Somma i link quando il server consente di dividere il download.": "Adds up your links when the server allows splitting the download.",
  "Apri": "Open",
  "Chiudi": "Close",
  "Riepilogo velocità": "Speed summary",
  "Traffico combinato": "Combined traffic",
  "Velocità in tempo reale": "Real-time speed",
  "Live": "Live",
  "Media 10 s": "10 s average",
  "Connessioni vive": "Live connections",
  "Aggregazione": "Aggregation",
  "Singola": "Single",
  "Percorsi Internet": "Internet paths",
  "Verifica…": "Checking…",
  "Rileva ora": "Detect now",
  "Avvio del monitor": "Starting monitor",
  "Sto leggendo le interfacce di rete di Windows.": "Reading Windows network interfaces.",
  "Nessuna connessione viva": "No live connection",
  "Collega Wi-Fi, Ethernet o tethering USB: apparirà automaticamente.": "Plug in Wi-Fi, Ethernet or USB tethering: it will show up automatically.",
  "Rileva di nuovo": "Detect again",
  "connessione memorizzata": "remembered connection",
  "connessioni memorizzate": "remembered connections",
  "Andamento": "Trend",
  "Velocità combinata": "Combined speed",
  "Valori smussati, aggiornamento 4 Hz": "Smoothed values, 4 Hz refresh",
  "Download": "Download",
  "Torna all’ultimo risultato": "Back to last result",
  "1 connessione verificata": "1 connection verified",
  "{n} connessioni verificate": "{n} connections verified",
  "Scansione non riuscita": "Scan failed",
  // SystemBalancePanel
  "Servizio non disponibile": "Service unavailable",
  "Tutto il PC è bilanciato": "The whole PC is balanced",
  "Attivazione in corso…": "Activating…",
  "Intervento richiesto": "Action required",
  "Pronto": "Ready",
  "In attesa di una connessione utilizzabile": "Waiting for a usable connection",
  "{name} è esclusa dal bilanciamento: accendila qui sotto per usarla con DualBond.": "{name} is left out of balancing: switch it on below to use it with DualBond.",
  "Le tue connessioni sono escluse dal bilanciamento: accendile qui sotto per usarle con DualBond.": "Your connections are left out of balancing: switch them on below to use them with DualBond.",
  "Esclusa": "Left out",
  "Il tunnel non è partito.": "The tunnel did not start.",
  "Il servizio non risponde.": "The service is not responding.",
  "Rete": "Network",
  "Modalità sistema": "System mode",
  "Un’unica Internet, dietro tutte le tue connessioni.": "One Internet, behind all your connections.",
  "DualBond distribuisce automaticamente i flussi di Windows tra Ethernet, Wi-Fi e tethering USB.": "DualBond automatically spreads Windows traffic flows across Ethernet, Wi-Fi and USB tethering.",
  "Bilanciamento di sistema": "System balancing",
  "Tachimetri modalità sistema": "System mode gauges",
  "Totale combinato": "Combined total",
  "Traffico ora": "Traffic now",
  "Media": "Average",
  "Link disponibili": "Available links",
  "Flussi vivi": "Live flows",
  "Percorsi disponibili": "Available paths",
  "Lettura del servizio…": "Reading service…",
  "Nessuna connessione Internet stabile disponibile.": "No stable Internet connection available.",
  "media {v}": "avg {v}",
  "Usa {name}": "Use {name}",
  "Bilanciamento automatico per flusso. I singoli download cifrati non vengono spezzati; per quelli usa l’acceleratore file qui sotto.": "Automatic per-flow balancing. Individual encrypted downloads are not split; use the file accelerator below for those.",
  // Tipi di link
  "Tethering USB": "USB tethering",
  "Altra rete": "Other network",
  "Altro": "Other",
  // Gauge
  "Velocità totale {total}, media {avg}, {gain}": "Total speed {total}, average {avg}, {gain}",
  "Nessuna connessione attiva": "No active connection",
  "Nessuna connessione": "No connection",
  "{name} · media {avg}": "{name} · avg {avg}",
  "Offline": "Offline",
  "Media {v}": "Average {v}",
  "Connessione memorizzata": "Remembered connection",
  "Totale": "Total",
  "{v}× guadagno": "{v}× gain",
  // LinkChip / LinkCard
  "Online": "Online",
  "Verifica": "Checking",
  "Instabile": "Unstable",
  "Scollegata": "Disconnected",
  "Non utilizzabile": "Unusable",
  "Non usata": "Not used",
  "A consumo": "Metered",
  "segnale {n} %": "signal {n} %",
  "Esclusa dal download": "Excluded from downloads",
  "Dettagli di {name}": "Details of {name}",
  "Disattiva {name}": "Turn off {name}",
  "Escludi dai download": "Exclude from downloads",
  "Usa nei download": "Use in downloads",
  "Dettaglio {name}": "Details: {name}",
  "Colore {c}": "Colour {c}",
  "Velocità attuale": "Current speed",
  "Picco sessione": "Session peak",
  "Connessioni attive": "Active connections",
  "Errori": "Errors",
  "Traffico totale": "Total traffic",
  "Chiave stabile": "Stable key",
  "Dimentica questo link": "Forget this link",
  // Download
  "Download in corso": "Downloading",
  "In pausa": "Paused",
  "Verifica del file…": "Verifying file…",
  "Download completato": "Download complete",
  "Download non riuscito": "Download failed",
  "Download annullato": "Download cancelled",
  "Progresso {v}": "Progress {v}",
  "ora": "now",
  "media 10 s": "10 s average",
  "Circa": "About",
  "rimanenti": "remaining",
  "Pausa": "Pause",
  "Riprendi": "Resume",
  "Annulla": "Cancel",
  "Apri cartella": "Open folder",
  "Scarica un altro file": "Download another file",
  "{a} di {b}": "{a} of {b}",
  // UrlBar
  "Inserisci un indirizzo HTTP o HTTPS completo.": "Enter a complete HTTP or HTTPS address.",
  "Nuovo download multi-link": "New multi-link download",
  "Incolla l’indirizzo diretto del file": "Paste the direct file address",
  "DualBond distribuirà i segmenti sulle connessioni abilitate.": "DualBond will spread the segments across the enabled connections.",
  "Indirizzo del file": "File address",
  "https://esempio.it/file.zip": "https://example.com/file.zip",
  "Avvio…": "Starting…",
  "Avvia download": "Start download",
  // Grafico
  "1 min": "1 min",
  "5 min": "5 min",
  // SpeedTest / diagnostica
  "La diagnostica non ha risposto entro 20 secondi.": "Diagnostics did not respond within 20 seconds.",
  "Test connessioni": "Connection test",
  "Verifica in corso…": "Checking…",
  "Riprova": "Retry",
  "Ripeti": "Repeat",
  "Nessun link fisico rilevato.": "No physical link detected.",
  // esiti della diagnostica che arrivano dal backend
  "OK (binding corretto)": "OK (correct binding)",
  "nessun gateway di default": "no default gateway",
  "nessun IP locale della stessa famiglia": "no local IP of the same family",
  "Nessuna scheda di rete ha una route verso Internet.": "No network adapter has a route to the Internet.",
  // Settings
  "Chiudi impostazioni": "Close settings",
  "Sezioni impostazioni": "Settings sections",
  "Connessioni": "Connections",
  "Aspetto": "Appearance",
  "Caricamento delle impostazioni…": "Loading settings…",
  "Tutte le connessioni": "All connections",
  "Le preferenze valgono sia per la modalità sistema sia per l’acceleratore file.": "Preferences apply to both system mode and the file accelerator.",
  "Aggiorna": "Refresh",
  "Nessuna connessione è stata rilevata. Le nuove reti compariranno e saranno abilitate automaticamente.": "No connection detected. New networks will appear and be enabled automatically.",
  "Nome di {name}": "Name of {name}",
  "online": "online",
  "offline": "offline",
  "Colore di {name}": "Colour of {name}",
  "Scegli {c}": "Pick {c}",
  "Dimentica {name}": "Forget {name}",
  "Acceleratore file": "File accelerator",
  "Queste opzioni si applicano soltanto ai download avviati dalla sezione dedicata.": "These options only apply to downloads started from the dedicated section.",
  "Cartella di destinazione": "Destination folder",
  "Cartella Download di Windows": "Windows Downloads folder",
  "Scegli…": "Browse…",
  "Scegli la cartella dei download": "Choose the download folder",
  "Se il campo è vuoto, DualBond usa la cartella Download di Windows.": "If empty, DualBond uses the Windows Downloads folder.",
  "Connessioni parallele per rete": "Parallel connections per network",
  "Automatiche (consigliato)": "Automatic (recommended)",
  "Più connessioni non significano sempre maggiore velocità; la modalità automatica ne usa quattro.": "More connections do not always mean more speed; automatic mode uses four.",
  "L’acceleratore file resta utile quando un singolo download può essere suddiviso in segmenti. La modalità sistema, invece, distribuisce i flussi completi senza spezzare una singola connessione cifrata.": "The file accelerator is useful when a single download can be split into segments. System mode instead distributes whole flows without splitting a single encrypted connection.",
  "Tema e colori vengono applicati subito e restano memorizzati su questo PC.": "Theme and colours apply immediately and are remembered on this PC.",
  "Tema": "Theme",
  "Sistema": "System",
  "Chiaro": "Light",
  "Scuro": "Dark",
  "Colore principale": "Accent colour",
  "Diagnostica reti": "Network diagnostics",
  "Verifica che ogni interfaccia possa uscire con il proprio indirizzo e percorso.": "Checks that every interface can reach the Internet with its own address and path.",
  "Avvia verifica": "Run check",
  "percorso verificato": "path verified",
  "Avvia la verifica per controllare tutte le connessioni disponibili.": "Run the check to test all available connections.",
  "Salvataggio…": "Saving…",
  "Le modifiche vengono salvate automaticamente.": "Changes are saved automatically.",
  // Accent presets
  "Cyan e Viola": "Cyan and Violet",
  "Verde acido e Magenta": "Acid Green and Magenta",
  "Blu elettrico e Arancio": "Electric Blue and Orange",
  "Rosa shock e Ciano": "Shock Pink and Cyan",
  "Giallo acido e Viola": "Acid Yellow and Violet",
  "Rosso cyber e Blu": "Cyber Red and Blue",
  // Messaggi amichevoli
  "Questo sito non permette di dividere il download. Userò una sola connessione: sarà più lento ma il file arriverà corretto.": "This site does not allow splitting the download. I will use a single connection: slower, but the file will arrive intact.",
  "Il link è scaduto. Copia un link nuovo dal sito: riprenderò da dove eravamo, senza ricominciare.": "The link has expired. Copy a fresh link from the site: I will resume where we left off, without starting over.",
  "Una delle connessioni riceve una versione diversa del file. Continuo con le altre per non rovinare il download.": "One connection is receiving a different version of the file. I will carry on with the others so the download is not corrupted.",
  "Il sito sta limitando i download. Rallento e riprovo automaticamente.": "The site is rate-limiting downloads. I will slow down and retry automatically.",
  "Non riesco a scaricare una parte del file su nessuna connessione disponibile.": "I cannot download part of the file on any available connection.",
  "Il file sul sito è cambiato da quando avevi iniziato. Ricomincio da capo?": "The file on the site has changed since you started. Start over?",
  "Sì / No": "Yes / No",
  "Download annullato.": "Download cancelled.",
  "{name} non è più collegato. Continuo con le altre connessioni, non ho perso niente.": "{name} is no longer connected. I am continuing with the other connections; nothing was lost.",
  "{name} è a consumo. Vuoi usarla comunque per questo download?": "{name} is metered. Use it anyway for this download?",
  "Sì / No / Non chiedere più": "Yes / No / Don't ask again",
  "Due connessioni usano la stessa rete interna e non riesco a tenerle separate. Prova a scollegare un telefono e attivare invece il suo hotspot Wi-Fi. In alternativa userò solo la connessione più veloce.": "Two connections share the same internal network and I cannot keep them apart. Try unplugging a phone and using its Wi-Fi hotspot instead. Otherwise I will use only the fastest connection.",
};

const DICTIONARIES: Record<Language, Record<string, string>> = { it: {}, en: EN };

function detect(): Language {
  try {
    const stored = typeof localStorage !== "undefined" ? localStorage.getItem(STORAGE_KEY) : null;
    if (stored === "it" || stored === "en") return stored;
  } catch {
    // localStorage non disponibile: si ricade sulla lingua del sistema.
  }
  const nav = typeof navigator !== "undefined" ? navigator.language : "it";
  return nav.toLowerCase().startsWith("it") ? "it" : "en";
}

class LanguageStore {
  current = $state<Language>(detect());

  set(language: Language) {
    this.current = language;
    try {
      localStorage.setItem(STORAGE_KEY, language);
    } catch {
      // preferenza non persistita, resta valida per la sessione
    }
    if (typeof document !== "undefined") document.documentElement.lang = language;
  }

  get locale(): string {
    return this.current === "it" ? "it-IT" : "en-US";
  }
}

export const lang = new LanguageStore();
if (typeof document !== "undefined") document.documentElement.lang = lang.current;

/** Traduce `key` (testo italiano) nella lingua corrente. Reattiva: legge
 * `lang.current`, quindi va chiamata dentro template o `$derived`. */
export function t(key: string, params?: Record<string, string | number>): string {
  const text = DICTIONARIES[lang.current][key] ?? key;
  if (!params) return text;
  return text.replace(/\{(\w+)\}/g, (match, name) =>
    name in params ? String(params[name]) : match,
  );
}
