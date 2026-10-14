# Third-party notices

DualBond includes or dynamically loads the following components in its Windows installer.

## Wintun 0.14.1

Copyright © WireGuard LLC. The Wintun *source code* is GPL-2.0, but DualBond does not use the source: it redistributes the **unmodified, official, signed prebuilt `wintun.dll`**, which is governed by the separate *Prebuilt Binaries License* (full text in [`third_party/wintun-prebuilt-binaries-license.txt`](third_party/wintun-prebuilt-binaries-license.txt)). In short, that license allows use for lawful purposes and redistribution together with other software that uses the DLL only through the public `wintun.h` API (which DualBond does, loading it at runtime); it forbids modifying or reverse-engineering the DLL, reselling or sublicensing the DLL itself, and using the WireGuard/Wintun names to endorse derived products. **The DualBond project license does not apply to `src-tauri/resources/wintun.dll`.** Wintun™ is a trademark of Jason A. Donenfeld. Source and license: <https://www.wintun.net/>.

Bundled amd64 DLL SHA-256: `E5DA8447DC2C320EDC0FC52FA01885C103DE8C118481F683643CACC3220DAFCE`.

The official `wintun-0.14.1.zip` archive was verified before extraction with SHA-256 `07C256185D6EE3652E09FA55C0B673E2624B565E02C4B9091C79CA7D2F24EF51`; the DLL has a valid Microsoft Authenticode signature.

## ipstack 1.0.1

Copyright © its contributors. Licensed under Apache-2.0. Source: <https://github.com/narrowlink/ipstack>.

## tun 0.8.14

Copyright © its contributors. Licensed under the WTFPL. Source: <https://github.com/meh/rust-tun>.

## windows-service 0.8.1

Copyright © its contributors. Licensed under MIT or Apache-2.0. Source: <https://github.com/mullvad/windows-service-rs>.

## Other dependencies

All other Rust crates (567 in `Cargo.lock`) and npm packages (Svelte, Vite, Tauri API/CLI, uPlot, TypeScript) are under permissive licenses: MIT, Apache-2.0, BSD-2/3-Clause, ISC, Zlib, Unlicense/CC0, Unicode-3.0, WTFPL, or a choice of those. No GPL/AGPL/SSPL component is linked into DualBond. Five crates (`cssparser` and related, from the Servo project) are MPL-2.0 (file-level copyleft); they are used unmodified, and their source is available from crates.io.
