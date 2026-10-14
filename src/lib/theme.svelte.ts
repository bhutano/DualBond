// Tema chiaro/scuro/di sistema (§6.6 della spec, pannello Aspetto).
// "Di sistema" = nessun attributo su <html>, l'app.css segue
// prefers-color-scheme da sola.

export type ThemePreference = "system" | "light" | "dark";

const STORAGE_KEY = "dualbond-theme";

class ThemeStore {
  preference = $state<ThemePreference>(readStored());

  constructor() {
    $effect.root(() => {
      $effect(() => {
        apply(this.preference);
        if (typeof localStorage !== "undefined") {
          localStorage.setItem(STORAGE_KEY, this.preference);
        }
      });
    });
  }

  set(pref: ThemePreference) {
    this.preference = pref;
  }
}

function readStored(): ThemePreference {
  if (typeof localStorage === "undefined") return "system";
  const stored = localStorage.getItem(STORAGE_KEY);
  return stored === "light" || stored === "dark" ? stored : "system";
}

function apply(pref: ThemePreference) {
  if (typeof document === "undefined") return;
  if (pref === "system") {
    delete document.documentElement.dataset.theme;
  } else {
    document.documentElement.dataset.theme = pref;
  }
}

export const theme = new ThemeStore();
