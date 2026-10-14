// Mappatura non lineare valore -> angolo del gauge (§6.3 della spec).
// Le velocità realistiche coprono due ordini di grandezza (5↔500 Mbps):
// una scala lineare renderebbe illeggibili i link lenti, quindi log.

/** Scaletta dei fondoscala (§6.3) — deve combaciare con
 * `metrics::sampler::SCALE_STEPS` lato Rust: è il backend a decidere
 * (con isteresi) quale fondoscala è attivo, il frontend lo riceve già
 * in `StatsSnapshot.scaleMax` e usa questa lista solo per le tacche. */
export const SCALE_STEPS = [
  10, 25, 50, 100, 250, 500, 1000, 2500, 5000, 10000, 25000, 50000, 100000, 250000, 500000, 1000000,
] as const;

/** Geometria dell'arco (§6.3): 270°, da 135° a 405° (0°/360° = destra,
 * in senso orario), apertura in basso. */
export const ARC_START_DEG = 135;
export const ARC_END_DEG = 405;
export const ARC_SWEEP_DEG = ARC_END_DEG - ARC_START_DEG;

/** `t = log(1+v) / log(1+scaleMax)`, clampato a [0,1] — un valore oltre
 * il fondoscala corrente (transitorio, prima che l'isteresi lo alzi) non
 * deve mandare l'arco fuori scala. */
export function valueToFraction(value: number, scaleMax: number): number {
  if (scaleMax <= 0) return 0;
  const t = Math.log(1 + Math.max(0, value)) / Math.log(1 + scaleMax);
  return Math.min(1, Math.max(0, t));
}

export function fractionToAngleDeg(fraction: number): number {
  return ARC_START_DEG + fraction * ARC_SWEEP_DEG;
}

export function valueToAngleDeg(value: number, scaleMax: number): number {
  return fractionToAngleDeg(valueToFraction(value, scaleMax));
}

/** Tacche etichettate ai valori della scaletta interni al fondoscala
 * corrente (§6.3: "Tacche etichettate ai valori della scaletta interni
 * al fondoscala corrente"). */
export function tickValues(scaleMax: number): number[] {
  return SCALE_STEPS.filter((s) => s <= scaleMax);
}

/** Punto cartesiano su un cerchio di raggio `r` centrato nell'origine,
 * per un angolo in gradi nella stessa convenzione di `ARC_START_DEG`
 * (0° = destra, in senso orario — coordinate schermo, Y verso il basso). */
export function polarToCartesian(radius: number, angleDeg: number): { x: number; y: number } {
  const rad = (angleDeg * Math.PI) / 180;
  return { x: radius * Math.cos(rad), y: radius * Math.sin(rad) };
}
