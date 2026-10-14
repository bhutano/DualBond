# Note tecniche — Milestone 1 (discovery, identità stabili, validazione binding)

Documento di lavoro: raccoglie tutto ciò che è stato verificato nei sorgenti
vendored di `windows-sys` (in `.cargo-home/registry/src/index.crates.io-*/windows-sys-0.59.0/`)
prima di scrivere il codice reale della Milestone 1, così non va ri-derivato
da zero alla prossima sessione.

Riferimento spec: `dualbond-specifica-prompt-v3.md` (documento locale, non incluso nel repository)
(Parte 4.1 Discovery, Parte 4.2 Binding, Parte 4.3 Validazione empirica).

## Stato aggiornato (implementazione completata)

Tutti i passi di §10 sono stati eseguiti. Codice scritto e verificato:

- `src-tauri/src/core/{errors,logging}.rs`
- `src-tauri/src/network/{identity,iphlpapi,wlan,wmi,discovery,socket_bind,verify_bind}.rs`
- `src-tauri/src/cli.rs` (comando `doctor`), wire-up in `main.rs`
  (intercetta `argv[1] == "doctor"` prima di costruire l'app Tauri)
- 34 unit test (TDD, tutti green) sulla logica pura: parsing SOCKADDR,
  classificazione IF_TYPE, chiavi stabili usb:/mac:, parsing SSID/GUID,
  combinazione discovery, byte-order IP_UNICAST_IF/IPV6_UNICAST_IF,
  decisione vincitore contatori (§4.3). I wrapper FFI diretti (syscall
  Windows) non sono unit-testati (mock dell'OS sconsigliato) — validati con
  `cargo run -- doctor` reale su questa macchina.

**Validato su hardware reale** (`cargo run --bin dualbond -- doctor`,
macchina con Ethernet + 2× "Connessione alla rete locale (LAN)*" (virtuali,
probabile hypervisor) + Wi-Fi reale):
- Discovery corretto: Ethernet vs Wi-Fi distinti via IfType, chiavi stabili
  `mac:xx:xx:xx:xx:xx:xx` generate correttamente per tutti gli adapter.
- Route di default trovata solo sull'interfaccia Wi-Fi reale (10.x.x.x).
- **Probe di validazione binding (criterio di uscita Milestone 1) superato**:
  socket UDP bindato via `IP_UNICAST_IF` sull'interfaccia Wi-Fi, traffico
  osservato (delta `OutOctets`) esclusivamente su quella interfaccia →
  `bound_correctly: true`.

**Risolto — SSID/segnale Wi-Fi (`ERROR_ACCESS_DENIED` da `WlanQueryInterface`)**:
root cause trovato con systematic-debugging, confermato con `netsh wlan
show interfaces` (stesso `wlanapi.dll`, stesso errore 5) — Windows spiega
esplicitamente: da Windows 10 leggere SSID/BSSID via WLAN API richiede il
permesso **Servizi di localizzazione** attivo (SSID/BSSID sono usabili per
geolocalizzazione). Non è un bug di codice, non è legato alla sessione
interattiva/harness (ipotesi precedente, ora falsificata dall'evidenza:
`netsh` fallisce identico anche da un normale terminale desktop). Il
`GUID` matching tra IpHelper e WLAN non è la causa (verificato via unit
test, produce match corretto per tutte le interfacce Wi-Fi).

Fix applicato: `DualBondError::LocationServicesRequired` (nuova variante
in `core/errors.rs`) intercetta `ERROR_ACCESS_DENIED` specificamente in
`WlanQueryInterface` (`network/wlan.rs::query_current_connection`) e
restituisce un messaggio comprensibile invece del codice numerico grezzo,
condiviso tra CLI e comando Tauri (`doctor::collect_report`).

**Verificato end-to-end** (2026-08-20): utente ha attivato Impostazioni →
Privacy e sicurezza → Posizione → Servizi di posizione (registro:
`HKCU\...\ConsentStore\location` `Value` passato da `Deny` ad `Allow`).
Rilanciando `doctor`, SSID e segnale ora letti correttamente:
`Wi-Fi — ssid: "<SSID-hotspot>", signal_percent: 96`. Dettaglio
interessante scoperto durante il debug: il registro traccia le app
desktop non pacchettizzate per nome file sotto
`ConsentStore\location\NonPackaged\Executables\<exe>`, ma solo come flag
"prompt già mostrato" — non esiste un vero Allow/Deny per singola app
desktop indipendente dal master globale; con il master su `Deny` e
`ShowGlobalPrompts=0`, Windows non registra nemmeno il tentativo per app
mai viste prima (confermato empiricamente: `dualbond.exe` non compariva
in quella chiave nonostante multiple chiamate a `WlanQueryInterface`,
mentre app installate in precedenza — Adobe, NVIDIA — sì, presumibilmente
registrate quando il master era ancora `Allow`).

Le due interfacce Wi-Fi virtuali ("Connessione alla rete locale (LAN)*")
ora falliscono con `WIN32_ERROR 1168` (`ERROR_NOT_FOUND`) invece di
`ACCESS_DENIED` — atteso, sono adapter virtuali senza connessione WLAN
reale associata, nulla da leggere.

**Milestone 1 completa e verificata su tutti i fronti.** Prossimo passo:
Milestone 2 (Parte 11 della spec) — oppure test USB tethering / multi-link
reale (mai ancora testati su hardware, vedi conversazione precedente),
a scelta dell'utente.

---

Stato del codice al momento di scrivere *questa parte* del file (versione
originale, sotto): **non è stato ancora scritto/modificato nessun file
Rust**. Solo ricerca sui sorgenti della crate. (Superato — vedi sopra.)

---

## 1. Perché consultare i sorgenti vendored

`windows-sys` è una crate di binding FFI generata da bindgen: non ha
documentazione discorsiva, bisogna leggere le struct/union/costanti esatte
per non sbagliare layout di memoria (specialmente unioni C, bitfield, e
l'ordine byte di `IP_UNICAST_IF` che la spec segnala come "l'errore più
comune in assoluto").

I sorgenti completi delle dipendenze erano già presenti in
`D:\SignalBuster\DualBond\.cargo-home\registry\src\index.crates.io-1949cf8c6b5b557f\`
(fetch precedente), quindi niente da scaricare: si può `grep`/`Read`
direttamente lì. Versione pinnata in `Cargo.toml`: `windows-sys = "0.59"`.

Percorsi chiave:
- `windows-sys-0.59.0\src\Windows\Win32\NetworkManagement\IpHelper\mod.rs`
- `windows-sys-0.59.0\src\Windows\Win32\NetworkManagement\WiFi\mod.rs`
- `windows-sys-0.59.0\src\Windows\Win32\Networking\WinSock\mod.rs`
- `wmi-0.13.4\src\lib.rs` (crate `wmi`, alto livello, usa COM internamente)

---

## 2. Funzioni FFI verificate (iphlpapi.dll)

```rust
// tutte in windows_sys::Win32::NetworkManagement::IpHelper

fn GetAdaptersAddresses(
    family: u32,                              // AF_UNSPEC = 0, AF_INET = 2, AF_INET6 = 23
    flags: GET_ADAPTERS_ADDRESSES_FLAGS,       // u32; 0 va bene, non serve GAA_FLAG_INCLUDE_GATEWAYS
    reserved: *const c_void,                   // NULL
    adapteraddresses: *mut IP_ADAPTER_ADDRESSES_LH,
    sizepointer: *mut u32,
) -> u32;                                       // WIN32_ERROR: 0 = successo, 111 = ERROR_BUFFER_OVERFLOW (rialloca e riprova)

fn GetIfEntry2(row: *mut MIB_IF_ROW2) -> WIN32_ERROR;
// input: row.InterfaceIndex (o InterfaceLuid) già impostato prima della chiamata

fn GetIpForwardTable2(
    family: ADDRESS_FAMILY,                    // u16; AF_UNSPEC/AF_INET/AF_INET6
    table: *mut *mut MIB_IPFORWARD_TABLE2,      // alloca l'API stessa; liberare con FreeMibTable
) -> WIN32_ERROR;

fn FreeMibTable(memory: *const c_void);          // libera il buffer di GetIpForwardTable2 (e simili)
```

Costanti WIN32_ERROR utili (in `Windows::Win32::Foundation`):
`ERROR_SUCCESS = NO_ERROR = 0`, `ERROR_BUFFER_OVERFLOW = 111`,
`ERROR_INSUFFICIENT_BUFFER = 122`, `ERROR_NOT_ENOUGH_MEMORY = 8`,
`ERROR_NOT_FOUND = 1168`.

`IfOperStatusUp: IF_OPER_STATUS = 1` (in `Ndis` module) — usare per filtrare
solo interfacce effettivamente su.

**IF_TYPE non è definito in windows-sys** (mancano `IF_TYPE_ETHERNET_CSMACD`,
`IF_TYPE_IEEE80211`, ecc.). Sono valori IANA `ifType` stabili e vanno
hardcoded a mano nel nostro codice con un commento:
- `6` = Ethernet CSMA/CD (include anche RNDIS/NCM del tethering USB Android!)
- `71` = IEEE 802.11 (Wi-Fi)
- `23` = PPP
- `24` = software loopback (escludere sempre)
- `131` = tunnel (escludere)

> Nota importante: il tethering USB Android appare a Windows come un
> adattatore Ethernet virtuale (IF_TYPE 6), **non** distinguibile dall'
> Ethernet vera tramite `IfType`. La distinzione va fatta con WMI
> (`PNPDeviceID` che inizia con `USB\`), come previsto da §4.1 della spec.

---

## 3. Struct verificate — IpHelper (adapters / route / counters)

### `IP_ADAPTER_ADDRESSES_LH`
Gated da `#[cfg(all(feature = "Win32_NetworkManagement_Ndis", feature = "Win32_Networking_WinSock"))]`
→ **serve aggiungere la feature `Win32_NetworkManagement_Ndis` a `windows-sys`
in `Cargo.toml`** (attualmente non presente nella lista feature).

Campi rilevanti (lista completa in `mod.rs:1115-1152`):
```rust
pub struct IP_ADAPTER_ADDRESSES_LH {
    pub Anonymous1: IP_ADAPTER_ADDRESSES_LH_0,       // union { Length: u32, IfIndex: u32 } via .Anonymous.{Length,IfIndex}
    pub Next: *mut IP_ADAPTER_ADDRESSES_LH,          // linked list, walk con while !p.is_null()
    pub AdapterName: PSTR,                            // ANSI GUID stringa stabile dell'adapter (usare come fallback identity)
    pub FirstUnicastAddress: *mut IP_ADAPTER_UNICAST_ADDRESS_LH,
    pub FirstDnsServerAddress: *mut IP_ADAPTER_DNS_SERVER_ADDRESS_XP,
    pub DnsSuffix: PWSTR,
    pub Description: PWSTR,                           // localizzato, NON usare per identità (lo dice la spec)
    pub FriendlyName: PWSTR,                           // nome mostrato all'utente come default
    pub PhysicalAddress: [u8; 8],                      // MAC, primi 6 byte
    pub PhysicalAddressLength: u32,
    pub Mtu: u32,
    pub IfType: u32,
    pub OperStatus: IF_OPER_STATUS,                    // confrontare con IfOperStatusUp
    pub Luid: NET_LUID_LH,
    pub ConnectionType: NET_IF_CONNECTION_TYPE,
    // ... altri campi ignorabili per Milestone 1
}
```
Per leggere `IfIndex`: `(*p).Anonymous1.Anonymous.IfIndex` (union `IP_ADAPTER_ADDRESSES_LH_0`
→ campo `Anonymous: IP_ADAPTER_ADDRESSES_LH_0_0 { Length: u32, IfIndex: u32 }`).

### `IP_ADAPTER_UNICAST_ADDRESS_LH`
```rust
pub struct IP_ADAPTER_UNICAST_ADDRESS_LH {
    pub Anonymous: IP_ADAPTER_UNICAST_ADDRESS_LH_0,   // union { Length, Flags } — non serve per noi
    pub Next: *mut IP_ADAPTER_UNICAST_ADDRESS_LH,
    pub Address: SOCKET_ADDRESS,                       // <-- IP dell'interfaccia (vedi §4 sotto)
    // PrefixOrigin, SuffixOrigin, DadState, lifetimes, OnLinkPrefixLength
}
```

### `MIB_IF_ROW2` (contatori per-interfaccia, §4.3)
Nessun `Default`; va creato con `mem::zeroed()` e impostato solo
`InterfaceIndex` prima di chiamare `GetIfEntry2`. Campi utili:
```rust
pub struct MIB_IF_ROW2 {
    pub InterfaceLuid: NET_LUID_LH,
    pub InterfaceIndex: u32,      // <-- impostare questo prima della chiamata
    // ...
    pub InOctets: u64,
    pub OutOctets: u64,           // <-- i due contatori che servono per §4.3
    // altri contatori (errori, discard...) ignorabili per ora
}
```

### `MIB_IPFORWARD_ROW2` / `MIB_IPFORWARD_TABLE2` (route/gateway, §4.1)
```rust
pub struct MIB_IPFORWARD_ROW2 {
    pub InterfaceLuid: NET_LUID_LH,
    pub InterfaceIndex: u32,
    pub DestinationPrefix: IP_ADDRESS_PREFIX,   // { Prefix: SOCKADDR_INET, PrefixLength: u8 }
    pub NextHop: SOCKADDR_INET,                  // <-- il gateway
    // Metric, Protocol, Loopback, Age, Origin...
}
pub struct MIB_IPFORWARD_TABLE2 {
    pub NumEntries: u32,
    pub Table: [MIB_IPFORWARD_ROW2; 1],          // trucco C: array flessibile.
    // Accesso alla riga i: base_ptr.byte_add(offset_of!(Table) + i * size_of::<MIB_IPFORWARD_ROW2>())
    // oppure più semplice: std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize)
    // (funziona perché Table è comunque il primo campo dopo NumEntries, nessun padding extra da considerare
    //  a parte l'allineamento naturale della struct — verificare con un piccolo test se necessario)
}
```
Route di default = riga con `DestinationPrefix.PrefixLength == 0` e
`InterfaceIndex` che combacia con l'adapter cercato. `NextHop` è
l'`IP_ADDRESS_PREFIX`/`SOCKADDR_INET` da convertire con la logica del punto 4.

Liberare la tabella con `FreeMibTable(table as *const c_void)`.

---

## 4. Conversione `SOCKET_ADDRESS` / `SOCKADDR*` → `std::net::IpAddr`

```rust
// windows_sys::Win32::Networking::WinSock
pub struct SOCKET_ADDRESS { pub lpSockaddr: *mut SOCKADDR, pub iSockaddrLength: i32 }
pub struct SOCKADDR { pub sa_family: ADDRESS_FAMILY /* u16 */, pub sa_data: [i8; 14] }
pub struct SOCKADDR_IN  { pub sin_family: ADDRESS_FAMILY, pub sin_port: u16, pub sin_addr: IN_ADDR, pub sin_zero: [i8;8] }
pub struct SOCKADDR_IN6 { pub sin6_family: ADDRESS_FAMILY, pub sin6_port: u16, pub sin6_flowinfo: u32, pub sin6_addr: IN6_ADDR, pub Anonymous: SOCKADDR_IN6_0 }
pub union  SOCKADDR_INET { pub Ipv4: SOCKADDR_IN, pub Ipv6: SOCKADDR_IN6, pub si_family: ADDRESS_FAMILY }

pub struct IN_ADDR  { pub S_un: IN_ADDR_0 }
pub union  IN_ADDR_0 { pub S_un_b: IN_ADDR_0_0 /* {s_b1..s_b4: u8} */, pub S_un_w: .., pub S_addr: u32 }
pub struct IN6_ADDR { pub u: IN6_ADDR_0 }
pub union  IN6_ADDR_0 { pub Byte: [u8; 16], pub Word: [u16; 8] }

pub const AF_INET: ADDRESS_FAMILY = 2;
pub const AF_INET6: ADDRESS_FAMILY = 23;
pub const AF_UNSPEC: ADDRESS_FAMILY = 0;
```

Conversione robusta (evita confusione di endianness): usare sempre i campi
byte-a-byte, mai `S_addr: u32` grezzo.

```rust
unsafe fn ip_from_sockaddr(p: *const SOCKADDR) -> Option<IpAddr> {
    if p.is_null() { return None; }
    match (*p).sa_family {
        AF_INET => {
            let sin = &*(p as *const SOCKADDR_IN);
            let b = sin.sin_addr.S_un.S_un_b; // { s_b1, s_b2, s_b3, s_b4 }
            Some(IpAddr::V4(Ipv4Addr::new(b.s_b1, b.s_b2, b.s_b3, b.s_b4)))
        }
        AF_INET6 => {
            let sin6 = &*(p as *const SOCKADDR_IN6);
            Some(IpAddr::V6(Ipv6Addr::from(sin6.sin6_addr.u.Byte)))
        }
        _ => None,
    }
}
```

Per `SOCKADDR_INET` (usato in `NextHop`/`DestinationPrefix.Prefix`): leggere
`si_family` e poi `.Ipv4`/`.Ipv6` invece di castare un puntatore a
`SOCKADDR`, ma la logica byte-per-byte è identica.

---

## 5. `IP_UNICAST_IF` / `IPV6_UNICAST_IF` — binding dell'interfaccia in uscita

Già definiti in windows-sys, **valori confermati uguali alla spec**:
```rust
pub const IPPROTO_IP: i32 = 0;
pub const IPPROTO_IPV6: i32 = 41;
pub const IP_UNICAST_IF: i32 = 31;     // usare con IPPROTO_IP, ifindex in NETWORK byte order (to_be_bytes())
pub const IPV6_UNICAST_IF: i32 = 31;   // usare con IPPROTO_IPV6, ifindex in HOST byte order (to_ne_bytes()/to_le_bytes())

fn setsockopt(s: SOCKET /* usize */, level: i32, optname: i32, optval: PCSTR /* *const u8 */, optlen: i32) -> i32;
```
`SOCKET` in windows-sys è `usize`. Da un `socket2::Socket` si ottiene il
raw socket Windows con `.as_raw_socket()` (`RawSocket = u64`), da castare a
`usize` per la FFI. Questo confirma quanto già scritto nella spec (§4.2):
attenzione critica alla differenza di byte order tra IPv4 e IPv6.

---

## 6. WLAN API (wlanapi.dll) — SSID e segnale Wi-Fi

```rust
// windows_sys::Win32::NetworkManagement::WiFi
fn WlanOpenHandle(dwclientversion: u32, reserved: *const c_void, pdwnegotiatedversion: *mut u32, phclienthandle: *mut HANDLE) -> u32;
fn WlanCloseHandle(hclienthandle: HANDLE, reserved: *const c_void) -> u32;
fn WlanEnumInterfaces(hclienthandle: HANDLE, reserved: *const c_void, ppinterfacelist: *mut *mut WLAN_INTERFACE_INFO_LIST) -> u32;
fn WlanQueryInterface(
    hclienthandle: HANDLE,
    pinterfaceguid: *const GUID,
    opcode: WLAN_INTF_OPCODE,               // usare wlan_intf_opcode_current_connection = 7
    reserved: *const c_void,
    pdwdatasize: *mut u32,
    ppdata: *mut *mut c_void,               // -> cast a *mut WLAN_CONNECTION_ATTRIBUTES
    pwlanopcodevaluetype: *mut WLAN_OPCODE_VALUE_TYPE,
) -> u32;
fn WlanFreeMemory(pmemory: *const c_void);  // liberare sia la interface list che i dati di query
```

Struct:
```rust
pub struct WLAN_INTERFACE_INFO_LIST { pub dwNumberOfItems: u32, pub dwIndex: u32, pub InterfaceInfo: [WLAN_INTERFACE_INFO; 1] }
pub struct WLAN_INTERFACE_INFO { pub InterfaceGuid: GUID, pub strInterfaceDescription: [u16; 256], pub isState: WLAN_INTERFACE_STATE }

pub struct WLAN_CONNECTION_ATTRIBUTES {
    pub isState: WLAN_INTERFACE_STATE,
    pub wlanConnectionMode: WLAN_CONNECTION_MODE,
    pub strProfileName: [u16; 256],
    pub wlanAssociationAttributes: WLAN_ASSOCIATION_ATTRIBUTES,   // <-- qui dentro SSID + segnale
    pub wlanSecurityAttributes: WLAN_SECURITY_ATTRIBUTES,
}
pub struct WLAN_ASSOCIATION_ATTRIBUTES {
    pub dot11Ssid: DOT11_SSID,              // { uSSIDLength: u32, ucSSID: [u8; 32] } — NON è UTF-8 garantito, ma quasi sempre sì
    pub dot11BssType: DOT11_BSS_TYPE,
    pub dot11Bssid: [u8; 6],
    pub dot11PhyType: DOT11_PHY_TYPE,
    pub uDot11PhyIndex: u32,
    pub wlanSignalQuality: u32,              // 0-100, esattamente il "signalPercent" richiesto da §6.4/contratto IPC
    pub ulRxRate: u32,                        // kbps
    pub ulTxRate: u32,                        // kbps
}
```

Come collegare l'interfaccia WLAN (`GUID`) all'`IfIndex` di `GetAdaptersAddresses`:
il campo `AdapterName` di `IP_ADAPTER_ADDRESSES_LH` è la stringa GUID
dell'adapter (es. `"{4D36E972-...}"`), che **corrisponde** al GUID
restituito da `WlanEnumInterfaces` per le interfacce Wi-Fi. Serve un parsing
stringa→GUID e confronto, oppure confrontare per uguaglianza di bytes dopo
aver parsato la stringa ANSI con `windows_sys::core::GUID::from(...)` (verificare
in fase di implementazione — piccolo dettaglio da testare con `cargo check`
+ test manuale su una macchina con Wi-Fi).

`wlan_intf_opcode_current_connection: WLAN_INTF_OPCODE = 7`.

---

## 7. WMI — identità stabile USB (`PNPDeviceID`)

Crate `wmi = "0.13"` (già in `Cargo.toml`), usa COM internamente (non serve
aggiungere feature a `windows-sys` per questo).

Uso base:
```rust
use wmi::{COMLibrary, WMIConnection};
use serde::Deserialize;

let com_con = COMLibrary::new()?;          // inizializza COM sul thread corrente
let wmi_con = WMIConnection::new(com_con)?;

#[derive(Deserialize, Debug)]
#[serde(rename = "Win32_NetworkAdapter")]
#[serde(rename_all = "PascalCase")]
struct NetworkAdapter {
    net_connection_id: Option<String>,   // nome amichevole
    pnp_device_id: Option<String>,       // "USB\VID_18D1&PID_4EE7\<serial>" oppure "PCI\..." per Ethernet reale
    interface_index: Option<u32>,        // <-- da incrociare con IfIndex di GetAdaptersAddresses
    guid: Option<String>,
}

let adapters: Vec<NetworkAdapter> = wmi_con.query()?;
```

Parsing `PNPDeviceID` per USB (§3.2 della spec):
```
"USB\VID_18D1&PID_4EE7\0123456789ABCDEF"
      ^^^^^^^ ^^^^^^^^  ^^^^^^^^^^^^^^^^
      VID     PID       serial
```
→ chiave stabile: `format!("usb:{VID}:{PID}:{serial}")` (spec §3.2).

**Nota**: `COMLibrary::new()` inizializza COM come multi-thread o
apartment-thread a seconda della configurazione; siccome Tauri/webview2 già
usa COM sul thread UI, la chiamata WMI andrà fatta su un thread dedicato
(es. `tokio::task::spawn_blocking`) per evitare conflitti di threading model
con WebView2. Da verificare in fase di implementazione.

---

## 8. Feature flag da aggiungere a `Cargo.toml`

Attuale (`src-tauri/Cargo.toml`):
```toml
[dependencies.windows-sys]
version = "0.59"
features = [
  "Win32_Foundation",
  "Win32_NetworkManagement_IpHelper",
  "Win32_NetworkManagement_WiFi",
  "Win32_Networking_WinSock",
  "Win32_Storage_FileSystem",
  "Win32_System_IO",
  "Win32_System_Power",
  "Win32_System_Threading",
]
```
**Manca**: `"Win32_NetworkManagement_Ndis"` — senza questa feature,
`IP_ADAPTER_ADDRESSES_LH` e `MIB_IF_ROW2` non compilano (sono gated dietro
`#[cfg(feature = "Win32_NetworkManagement_Ndis")]` per i campi che usano
`IF_OPER_STATUS`, `NET_LUID_LH`, `NET_IF_CONNECTION_TYPE`, ecc.).

Azione da fare appena si riprende: aggiungere la riga alla lista features
prima di scrivere `network/iphlpapi.rs`.

---

## 9. Decisioni di design prese (da confermare con l'utente prima di scrivere codice)

1. **Validazione empirica (§4.3) senza contattare Internet**: dato il
   vincolo "Vietato usare servizi tipo 'what is my IP'... misurazione
   puramente locale", il trasferimento di prova userà **datagrammi UDP
   verso il gateway del link stesso** (non un host esterno): si apre un
   socket UDP bindato con `IP_UNICAST_IF` + IP locale, si inviano ~5s di
   pacchetti verso `gateway_ip:33434` (porta arbitraria, nessuna risposta
   attesa), e si confrontano i delta di `OutOctets` tra l'interfaccia
   attesa e tutte le altre. Questo intercetta esattamente il caso di
   collisione di subnet descritto in §0.3-A (ARP che risolve sull'interfaccia
   sbagliata), senza violare il vincolo "nessun host esterno oltre al
   server di download e ai resolver DNS".
2. **Hot-plug (`NotifyIpInterfaceChange`) rinviato alla Milestone 5** come
   da tabella Parte 11: la Milestone 1 fa solo discovery "a scatto" (invocata
   da `dualbond doctor` o dal comando Tauri di diagnostica), non un
   listener continuo.
3. **Persistenza `link_identity` rinviata**: lo schema SQL è definito in
   §3.2 ma SQLite arriva solo in Milestone 3 (Parte 11). Per la Milestone 1
   si genera la chiave stabile e il nome auto-rilevato ad ogni esecuzione,
   senza ancora salvare rinomina/colore/enabled scelti dall'utente (non
   c'è ancora una UI per farlo, arriva in Milestone 6/7).
4. **Struttura moduli** (da Parte 5 della spec), sottoinsieme rilevante per
   Milestone 1:
   ```
   src-tauri/src/
     core/  errors.rs  logging.rs
     network/
       iphlpapi.rs     # GetAdaptersAddresses, GetIfEntry2, GetIpForwardTable2
       wlan.rs         # SSID + segnale via WLAN API
       discovery.rs    # combina iphlpapi + wmi + wlan -> Vec<LinkCandidate>
       identity.rs     # chiave stabile (usb:/wifi:/mac:)
       socket_bind.rs  # crea socket con IP_UNICAST_IF + bind locale
       verify_bind.rs  # test ~5s + confronto contatori (§4.3)
     cli.rs            # `dualbond doctor`
     main.rs           # bootstrap Tauri + intercetta argv per CLI
   ```

---

## 10. Prossimi passi concreti

1. Aggiungere `"Win32_NetworkManagement_Ndis"` alle feature di `windows-sys`
   in `src-tauri/Cargo.toml`.
2. Scrivere `network/iphlpapi.rs` (wrapper sicuri sopra le 4 funzioni FFI
   del punto 2, con parsing secondo i punti 3-4).
3. Scrivere `network/wlan.rs` (punto 6).
4. Scrivere `network/discovery.rs` + `network/identity.rs` (combina tutto,
   distingue USB/Wi-Fi/Ethernet, calcola chiave stabile) — richiede anche
   la query WMI del punto 7.
5. Scrivere `network/socket_bind.rs` + `network/verify_bind.rs` (punto 5 +
   decisione 9.1).
6. Scrivere `cli.rs` con il comando `doctor` che stampa l'elenco link e il
   risultato della validazione (criterio di uscita Milestone 1).
7. Wire-up in `main.rs`: intercettare `std::env::args()` prima di costruire
   l'app Tauri; se il comando CLI è gestito, uscire senza aprire la finestra.
8. Compilare e iterare con `cargo check` (toolchain rust globale già
   presente e funzionante: rustc/cargo 1.96.0) finché non c'è più nessun
   errore, poi testare `cargo run -- doctor` su questa macchina reale con
   almeno due interfacce (Wi-Fi + Ethernet) per verificare che l'isolamento
   venga effettivamente dimostrato dai contatori.
