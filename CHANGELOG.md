# Changelog

## 1.0.2 — WebView2 rendering

- Pause WebView2 rendering while DualBond is minimized or hidden in the notification area, and resume rendering when the window is restored.

## 1.0.1 — service reliability

- **The system service restarts by itself** after a crash or an unexpected stop (Windows recovery actions: retry after 5 s, 15 s, 60 s). Before, a crash left it stopped until the next reboot, and the app showed no connections.
- **Clear notice when the service is stopped**, with a *Start service* button (asks for administrator rights), instead of an empty connection list.
- Panics in the service are now written to `service.log` with a backtrace; if the process still dies without a trace, Windows keeps a local minidump in `%ProgramData%\DualBond\crashdumps` (at most 3, never uploaded).
- `service.log` is rotated at 10 MB (one backup), and per-connection messages moved to debug level: before, it grew without limit (80+ MB).

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
