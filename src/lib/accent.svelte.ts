// Accento (coppia di colori) scelto dall'utente tra una lista di preset
// "cyber" (Impostazioni → Aspetto). Applicato scrivendo --accent-raw/
// --accent2-raw sull'elemento radice: app.css deriva --accent/--accent-2
// da questi (via color-mix in chiaro), quindi un solo valore per preset
// basta per restare leggibile in entrambi i temi.

export type AccentPreset = {
  id: string;
  name: string;
  accent: string;
  accent2: string;
};

export const ACCENT_PRESETS: AccentPreset[] = [
  { id: "cyan-viola", name: "Cyan / Viola", accent: "#00E8FF", accent2: "#B026FF" },
  { id: "verde-magenta", name: "Verde acido / Magenta", accent: "#39FF14", accent2: "#FF00E5" },
  { id: "blu-arancio", name: "Blu elettrico / Arancio", accent: "#00D9FF", accent2: "#FF6B00" },
  { id: "rosa-ciano", name: "Rosa shock / Ciano", accent: "#FF2E9A", accent2: "#00F0FF" },
  { id: "giallo-viola", name: "Giallo acido / Viola", accent: "#E8FF00", accent2: "#8B00FF" },
  { id: "rosso-blu", name: "Rosso cyber / Blu", accent: "#FF073A", accent2: "#0D7EFF" },
];

const STORAGE_KEY = "dualbond-accent";
const DEFAULT_PRESET_ID = ACCENT_PRESETS[0].id;

function findPreset(id: string): AccentPreset {
  return ACCENT_PRESETS.find((p) => p.id === id) ?? ACCENT_PRESETS[0];
}

function readStored(): string {
  if (typeof localStorage === "undefined") return DEFAULT_PRESET_ID;
  return localStorage.getItem(STORAGE_KEY) ?? DEFAULT_PRESET_ID;
}

function apply(preset: AccentPreset) {
  if (typeof document === "undefined") return;
  document.documentElement.style.setProperty("--accent-raw", preset.accent);
  document.documentElement.style.setProperty("--accent2-raw", preset.accent2);
}

class AccentStore {
  presetId = $state<string>(readStored());

  constructor() {
    $effect.root(() => {
      $effect(() => {
        const preset = findPreset(this.presetId);
        apply(preset);
        if (typeof localStorage !== "undefined") {
          localStorage.setItem(STORAGE_KEY, preset.id);
        }
      });
    });
  }

  set(id: string) {
    this.presetId = id;
  }
}

export const accent = new AccentStore();
