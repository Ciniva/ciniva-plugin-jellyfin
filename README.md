# Ciniva plugin: Jellyfin bridge

A Jellyfin-compatible API in front of [Ciniva](https://github.com/Ciniva/ciniva), so Jellyfin apps (Android TV, webOS,
Tizen, Roku), Swiftfin, Infuse and Findroid can browse, play (direct or through Ciniva's GPU transcoding) and sync
progress. Users sign in with their Ciniva accounts; the bridge keeps no state of its own.

## Image

`ghcr.io/ciniva/ciniva-plugin-jellyfin` (built by GitHub Actions on every push to `main`; tags `latest`,
`sha-<short>`, and `X.Y.Z` for `vX.Y.Z` tags).

The image includes the official Jellyfin web client (jellyfin-web 10.11.11, GPL-2.0, copied from the pinned
`jellyfin/jellyfin` image) at `/web/`, which Jellyfin Desktop and Jellyfin Media Player load after connecting. Set
`WEB_DIR` to serve a different build.

## Configuration

| Variable | Default | |
|---|---|---|
| `CINIVA_URL` | – | How the bridge reaches Ciniva |
| `CINIVA_PLUGIN_SECRET` | – | Ciniva's service key (or `CINIVA_PLUGIN_SECRET_FILE`), used for images and plugin registration |
| `PLUGIN_URL` | – | How Ciniva reaches the bridge |
| `PUBLIC_URL` | – | Address shown to clients (e.g. `http://server:8096`) |
| `PORT` | `8096` | |
| `SERVER_NAME` | `Ciniva` | |
| `JELLYFIN_VERSION` | `10.11.11` | Version reported to clients |

Publish port 8096 to your LAN. TV apps generally don't trust private certificate authorities, so give them the plain
`http://` address.
