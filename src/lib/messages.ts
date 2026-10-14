import { t } from "./i18n.svelte";

// Errore tecnico -> messaggio in linguaggio comune (§6.7 della spec).
// Mai un codice HTTP grezzo come messaggio principale: quello va nel log
// e nel dettaglio espandibile, non qui.

export type FriendlyMessage = {
  text: string;
  action?: string;
};

const PATTERNS: { test: RegExp; message: FriendlyMessage }[] = [
  {
    test: /atteso 206.*ricevuto 200|risorsa probabilmente cambiata/i,
    message: {
      text: "Questo sito non permette di dividere il download. Userò una sola connessione: sarà più lento ma il file arriverà corretto.",
    },
  },
  {
    test: /\b403\b/,
    message: {
      text: "Il link è scaduto. Copia un link nuovo dal sito: riprenderò da dove eravamo, senza ricominciare.",
    },
  },
  {
    test: /etag divergente/i,
    message: {
      text: "Una delle connessioni riceve una versione diversa del file. Continuo con le altre per non rovinare il download.",
    },
  },
  {
    test: /\b429\b|\b503\b|too many requests|service unavailable/i,
    message: {
      text: "Il sito sta limitando i download. Rallento e riprovo automaticamente.",
    },
  },
  {
    test: /irraggiungibile dopo troppi tentativi/i,
    message: {
      text: "Non riesco a scaricare una parte del file su nessuna connessione disponibile.",
      action: "Riprova",
    },
  },
  {
    test: /il file sul sito è cambiato|risorsa cambiata/i,
    message: {
      text: "Il file sul sito è cambiato da quando avevi iniziato. Ricomincio da capo?",
      action: "Sì / No",
    },
  },
  {
    test: /annullato dall'utente/i,
    message: {
      text: "Download annullato.",
    },
  },
];

/** Traduce un messaggio d'errore tecnico (dal backend Rust, italiano
 * anch'esso) nel testo da mostrare come messaggio principale. Se nessun
 * pattern combacia, il testo tecnico stesso resta come fallback — meglio
 * mostrare qualcosa di vero che nascondere l'errore, ma il dettaglio
 * tecnico va comunque preferito nel log espandibile, non qui. */
function translate(message: FriendlyMessage): FriendlyMessage {
  return { text: t(message.text), action: message.action ? t(message.action) : undefined };
}

export function friendlyMessage(technicalError: string): FriendlyMessage {
  for (const { test, message } of PATTERNS) {
    if (test.test(technicalError)) return translate(message);
  }
  return { text: technicalError };
}

/** Link scomparso a metà download (§6.7, §4.8) — non deriva da un
 * pattern sul messaggio d'errore (quello resta un Transport generico),
 * va costruito esplicitamente da chi osserva lo stato `offline` di un
 * link che prima era `online`. */
export function linkDisappearedMessage(linkName: string): FriendlyMessage {
  return {
    text: t("{name} non è più collegato. Continuo con le altre connessioni, non ho perso niente.", { name: linkName }),
  };
}

export function meteredLinkPromptMessage(linkName: string): FriendlyMessage {
  return {
    text: t("{name} è a consumo. Vuoi usarla comunque per questo download?", { name: linkName }),
    action: t("Sì / No / Non chiedere più"),
  };
}

export function subnetCollisionMessage(): FriendlyMessage {
  return {
    text: t(
      "Due connessioni usano la stessa rete interna e non riesco a tenerle separate. Prova a scollegare un telefono e attivare invece il suo hotspot Wi-Fi. In alternativa userò solo la connessione più veloce.",
    ),
    action: t("Riprova"),
  };
}
