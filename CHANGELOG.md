# Changelog

## 1.0.0 — first public release

DualBond adds all your Internet connections together on Windows (Wi-Fi, Ethernet, phones on USB tethering).

- **System mode**: per-flow load balancing of the whole PC across every usable connection, with one master switch.
- **File accelerator**: one download split into chunks, each fetched over a different connection, with byte-exact resume and verification.
- **Converging gauges**: one small dial per connection, flowing into the big total dial with animated lines; the whole dashboard fits on one screen.
- Live measurement from the moment the app opens: every connection in use shows its real traffic, even before balancing is switched on or when a link is left out.
- Keeps Wi-Fi and Ethernet connected together: the installer turns off Windows' "minimize simultaneous connections" policy (restored on uninstall).
- Fail-open: if no connection is usable, the tunnel steps aside and Windows keeps browsing normally.
- Gauge scale grows with your connections (up to 1 Tbps); speeds above 1,000 Mbps are shown in Gbps.
- Real Internet detection per link, hot-plug, metered links, diagnostics.
- English and Italian interface, switchable from the top bar.
- Windows installer (NSIS) and one-line PowerShell install script with SHA-256 verification.
- Website in `docs/` (GitHub Pages).
- Source-available under the **PolyForm Shield License 1.0.0**: free to use and modify, including at work; selling it or offering a competing product requires a commercial license (`COMMERCIAL.md`). Contributions via CLA (`CLA.md`).
