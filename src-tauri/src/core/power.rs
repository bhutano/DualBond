//! Impedisce la sospensione del sistema durante un download attivo
//! (§4.9 della spec): `SetThreadExecutionState(ES_SYSTEM_REQUIRED |
//! ES_CONTINUOUS)` mentre scarica, rilasciato a download completato/in
//! pausa — altrimenti la sospensione interromperebbe USB e Wi-Fi a metà
//! trasferimento.

use windows_sys::Win32::System::Power::{
    SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED,
};

/// RAII: finché l'istanza vive, il sistema non si sospende da solo. Il
/// `Drop` ripristina il comportamento normale — va tenuta viva per tutta
/// la durata del download attivo (non prima, non dopo), mai dimenticata
/// in giro: un download completato che la tiene ancora viva
/// impedirebbe la sospensione per sempre.
pub struct StayAwakeGuard;

impl StayAwakeGuard {
    pub fn acquire() -> Self {
        unsafe {
            SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED);
        }
        Self
    }
}

impl Drop for StayAwakeGuard {
    fn drop(&mut self) {
        unsafe {
            SetThreadExecutionState(ES_CONTINUOUS);
        }
    }
}
