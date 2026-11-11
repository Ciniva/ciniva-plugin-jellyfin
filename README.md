# Ciniva plugin: Jellyfin bridge

A Jellyfin-compatible API in front of [Ciniva](https://github.com/Ciniva/ciniva), so Jellyfin apps (Android TV, webOS,
Tizen, Roku), Swiftfin, Infuse and Findroid can browse, play (direct or through Ciniva's GPU transcoding) and sync
progress. Users sign in with their Ciniva accounts; the bridge keeps no state of its own.

## Live TV

When Ciniva's [IPTV plugin](https://github.com/Ciniva/ciniva-plugin-iptv) is online, a **Live TV** view appears with
its channels (the groups the administrator chose, each user's favourites first when the app asks). There is no
programme guide yet, so guides show channels without programmes. A channel plays as live HLS: the bridge starts
Ciniva's live session with the codecs the app's DeviceProfile lists (copying the channel's video when it can,
MPEG-TS segments on Tizen) and relays the rolling playlist and segments. Channel logos are fetched from the playlist's
URLs by the bridge, so apps never contact those sites. The bridge needs internet access for logos.

## Image

`ghcr.io/ciniva/ciniva-plugin-jellyfin` (published by GitHub Actions for `vX.Y.Z` tags matching `Cargo.toml` as
`X.Y.Z`, `latest` and `sha-<short>`; run the workflow by hand to publish any branch as `sha-<short>`).

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
