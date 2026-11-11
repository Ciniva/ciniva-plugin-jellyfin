//! Live TV from Ciniva's IPTV plugin: channels as Jellyfin `TvChannel` items, played as live HLS relayed from Ciniva.

use crate::{
    app::{body_json, field, field_bool, field_i64, json, not_found, ApiError, ApiResult, Auth, Client, Shared, Q},
    ciniva::{LiveChannel, IPTV},
    ids::{Id, Kind},
    items::{self, Ctx},
    library::{self, ctx},
    playback::{self, is_tizen, new_play_session, proxy_response, rewrite_playlist, transcoding_profile},
    system::{iso_at, now_secs},
};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const MAX_LOGO_BYTES: usize = 2 * 1024 * 1024;
const LOGO_CACHE: usize = 400;

pub fn channel_hex(raw: &str) -> Option<String> {
    Id::decode(raw).and_then(|i| i.channel_hex())
}

/// Changes when the logo URL does, so clients refetch.
pub fn logo_tag(url: &str) -> String {
    hex::encode(&Sha256::digest(url.as_bytes())[..8])
}

pub fn channel_item(cx: &Ctx, c: &LiveChannel) -> Option<Value> {
    let id = Id::channel(&c.id)?;
    let number = c.number.map(|n| n.to_string());
    let mut v = json!({
        "Name": c.name,
        "ServerId": cx.server_id,
        "Id": id,
        "Etag": id,
        "Type": "TvChannel",
        "ChannelType": "TV",
        "MediaType": "Video",
        "IsFolder": false,
        "LocationType": "Remote",
        "PlayAccess": "Full",
        "SortName": items::sort_name(&c.name),
        "ChannelNumber": number,
        "Number": number,
        "Genres": c.groups,
        "ImageTags": {},
        "BackdropImageTags": [],
        "ImageBlurHashes": {},
        "ParentId": Id::view(Kind::LiveTvView),
        "UserData": user_data(&id, c.favourite),
    });
    if let Some(logo) = c.logo.as_deref().filter(|l| l.starts_with("http")) {
        let tag = logo_tag(logo);
        v["ImageTags"] = json!({ "Primary": tag });
        v["ChannelPrimaryImageTag"] = json!(tag);
        v["PrimaryImageAspectRatio"] = json!(1.0);
    }
    Some(v)
}

pub fn user_data(id: &str, favourite: bool) -> Value {
    let mut u = items::user_data(id, 0, 0, false);
    u["IsFavorite"] = json!(favourite);
    u
}

/// The plugin's `ui/channels` query for a Jellyfin channel query.
pub fn plugin_query(q: &Q, paged: bool) -> String {
    let mut out: Vec<String> = vec![];
    if q.bool("isfavorite") == Some(true) {
        out.push("favourites=1".into());
    }
    if q.bool("enablefavoritesorting") == Some(true) {
        out.push("favourites_first=1".into());
    }
    if q.list("sortby").iter().any(|s| s.eq_ignore_ascii_case("sortname") || s.eq_ignore_ascii_case("name")) {
        out.push("sort=name".into());
    }
    if let Some(t) = q.get("searchterm") {
        out.push(format!("q={}", encode(t)));
    }
    let ids: Vec<String> = q.list("ids").iter().chain(q.list("channelids").iter()).filter_map(|i| channel_hex(i)).collect();
    if !ids.is_empty() {
        out.push(format!("ids={}", ids.join(",")));
    }
    if paged {
        if let Some(s) = q.int("startindex").filter(|s| *s > 0) {
            out.push(format!("start={s}"));
        }
        if let Some(l) = q.int("limit").filter(|l| *l >= 0) {
            out.push(format!("limit={l}"));
        }
    }
    out.join("&")
}

fn encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

async fn remember_logos(st: &Shared, channels: &[LiveChannel]) {
    let mut logos = st.logos.lock().await;
    for c in channels {
        if let Some(l) = &c.logo {
            logos.insert(c.id.clone(), l.clone());
        }
    }
}

/// One page of channels as a Jellyfin query result. An offline plugin reads as no channels.
pub async fn channel_result(st: &Shared, auth: &Auth, q: &Q) -> ApiResult {
    let start = q.int("startindex").unwrap_or(0).max(0) as usize;
    let list = match st.ciniva.channels(&auth.token, &plugin_query(q, true)).await {
        Ok(l) => l,
        Err(e) => {
            tracing::info!("no channels from the IPTV plugin: {}", e.1);
            return Ok(library::empty_result());
        }
    };
    remember_logos(st, &list.channels).await;
    let cx = ctx(st, auth);
    let items: Vec<Value> = list.channels.iter().filter_map(|c| channel_item(&cx, c)).collect();
    let total = list.total.unwrap_or(start + items.len());
    Ok(library::query_result(items, total, start))
}

pub async fn channels(State(st): State<Shared>, auth: Auth, q: Q) -> ApiResult {
    channel_result(&st, &auth, &q).await
}

pub async fn fetch(st: &Shared, token: &str, hex: &str) -> ApiResult<LiveChannel> {
    let list = st.ciniva.channels(token, &format!("ids={hex}")).await?;
    remember_logos(st, &list.channels).await;
    list.channels.into_iter().find(|c| c.id == hex).ok_or_else(not_found)
}

pub async fn channel(State(st): State<Shared>, auth: Auth, Path(id): Path<String>) -> ApiResult {
    let hex = channel_hex(&id).ok_or_else(not_found)?;
    let c = fetch(&st, &auth.token, &hex).await?;
    Ok(json(channel_item(&ctx(&st, &auth), &c).ok_or_else(not_found)?))
}

pub async fn available(st: &Shared, token: &str) -> bool {
    st.ciniva.plugin_online(token, IPTV).await
}

pub fn view(cx: &Ctx) -> Value {
    let id = Id::view(Kind::LiveTvView);
    json!({
        "Name": "Live TV",
        "ServerId": cx.server_id,
        "Id": id,
        "Etag": id,
        "Type": "UserView",
        "CollectionType": "livetv",
        "IsFolder": true,
        "MediaType": "Unknown",
        "SortName": "live tv",
        "LocationType": "Remote",
        "ParentId": Id::view(Kind::Root),
        "DisplayPreferencesId": id,
        "ImageTags": {},
        "BackdropImageTags": [],
        "ImageBlurHashes": {},
        "UserData": items::user_data(&id, 0, 0, false),
    })
}

pub async fn info(State(st): State<Shared>, auth: Auth) -> ApiResult {
    let user = st.ciniva.me(&auth.token).await?;
    let online = available(&st, &auth.token).await;
    Ok(json(json!({
        "Services": [{
            "Name": "Ciniva",
            "HomePageUrl": "",
            "Status": if online { "Ok" } else { "Unavailable" },
            "StatusMessage": "",
            "Version": env!("CARGO_PKG_VERSION"),
            "HasUpdateAvailable": false,
            "IsVisible": true,
            "Tuners": [],
        }],
        "IsEnabled": online,
        "EnabledUsers": [Id::user(user.id)],
    })))
}

pub async fn guide_info() -> Response {
    let now = now_secs();
    json(json!({ "StartDate": iso_at(now - now % 3600), "EndDate": iso_at(now - now % 3600 + 14 * 86_400) }))
}

pub async fn set_favourite(st: &Shared, token: &str, hex: &str, on: bool) -> ApiResult<()> {
    st.ciniva.send(Some(token), Method::POST, &format!("/plugins/{IPTV}/ui/favorite"), &json!({ "id": hex, "on": on })).await?;
    Ok(())
}

/// Channel logos are external URLs (often imgur). Only URLs the plugin lists for a channel are fetched.
pub async fn image(st: &Shared, client: &Client, hex: &str, method: &Method) -> ApiResult {
    let known = st.logos.lock().await.get(hex).cloned();
    let url = match known {
        Some(u) => u,
        None => {
            let token = st.ciniva.secret().map(str::to_string).or_else(|| client.token.clone()).ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "authorization required".into()))?;
            fetch(st, &token, hex).await?.logo.ok_or_else(not_found)?
        }
    };
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(not_found());
    }
    let cached = st.logo_cache.lock().await.get(&url).cloned();
    let (ctype, bytes) = match cached {
        Some(c) => c,
        None => {
            let fetched = download(st, &url).await?;
            let mut cache = st.logo_cache.lock().await;
            if cache.len() >= LOGO_CACHE {
                cache.clear();
            }
            cache.insert(url, fetched.clone());
            fetched
        }
    };
    let body = if *method == Method::HEAD { Bytes::new() } else { bytes };
    Ok((StatusCode::OK, [(header::CONTENT_TYPE, ctype), (header::CACHE_CONTROL, "public, max-age=86400".to_string())], body).into_response())
}

async fn download(st: &Shared, url: &str) -> ApiResult<(String, Bytes)> {
    let bad = |m: String| ApiError(StatusCode::BAD_GATEWAY, m);
    let resp = st.web.get(url).send().await.map_err(|e| bad(format!("logo: {e}")))?;
    if !resp.status().is_success() {
        return Err(ApiError(StatusCode::NOT_FOUND, format!("logo answered {}", resp.status())));
    }
    let ctype = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    if !ctype.starts_with("image/") || resp.content_length().is_some_and(|l| l as usize > MAX_LOGO_BYTES) {
        return Err(ApiError(StatusCode::NOT_FOUND, "logo is not a small image".into()));
    }
    let bytes = resp.bytes().await.map_err(|e| bad(format!("logo: {e}")))?;
    if bytes.len() > MAX_LOGO_BYTES {
        return Err(ApiError(StatusCode::NOT_FOUND, "logo too large".into()));
    }
    Ok((ctype, bytes))
}

/// A live MediaSource whose TranscodingUrl leads to the bridge's live HLS relay.
pub fn media_source(item_id: &str, name: &str, url: String, container: &str) -> Value {
    json!({
        "Protocol": "Http",
        "Id": item_id,
        "Type": "Default",
        "Container": container,
        "Name": name,
        "IsRemote": true,
        "ETag": item_id,
        "ReadAtNativeFramerate": false,
        "IgnoreDts": false,
        "IgnoreIndex": false,
        "GenPtsInput": false,
        "SupportsTranscoding": true,
        "SupportsDirectStream": false,
        "SupportsDirectPlay": false,
        "IsInfiniteStream": true,
        "UseMostCompatibleTranscodingProfile": false,
        "RequiresOpening": false,
        "RequiresClosing": false,
        "RequiresLooping": false,
        "SupportsProbing": false,
        "MediaStreams": [
            { "Type": "Video", "Codec": "h264", "Index": 0, "IsDefault": true, "IsExternal": false, "IsInterlaced": false, "IsTextSubtitleStream": false, "SupportsExternalStream": false },
            { "Type": "Audio", "Codec": "aac", "Index": 1, "Channels": 2, "IsDefault": true, "IsExternal": false, "IsInterlaced": false, "IsTextSubtitleStream": false, "SupportsExternalStream": false },
        ],
        "MediaAttachments": [],
        "Formats": [],
        "RequiredHttpHeaders": {},
        "TranscodingUrl": url,
        "TranscodingSubProtocol": "hls",
        "TranscodingContainer": container,
        "DefaultAudioStreamIndex": 1,
        "DefaultSubtitleStreamIndex": -1,
        "HasSegments": false,
    })
}

/// The bridge URL a client plays a channel from, carrying what its DeviceProfile can decode.
pub fn transcoding_url(item_id: &str, token: &str, play_session: &str, profile: Option<&Value>, max_bitrate: Option<i64>, tizen: bool) -> (String, String) {
    let t = profile.map(transcoding_profile).unwrap_or(playback::Transcode { video_codecs: vec![], audio_codecs: vec![], container: "ts".into(), max_audio_channels: None });
    let video: Vec<&str> = t.video_codecs.iter().map(String::as_str).filter(|c| ["h264", "hevc", "av1"].contains(c)).collect();
    let mut url = format!(
        "/videos/{item_id}/master.m3u8?PlaySessionId={play_session}&api_key={token}&VideoCodec={}&AudioCodec={}&TranscodingContainer={}",
        if video.is_empty() { "h264".to_string() } else { video.join(",") },
        if t.audio_codecs.is_empty() { "aac".to_string() } else { t.audio_codecs.join(",") },
        t.container
    );
    if let Some(b) = max_bitrate.filter(|b| *b > 0) {
        url.push_str(&format!("&MaxStreamingBitrate={b}"));
    }
    if let Some(c) = t.max_audio_channels {
        url.push_str(&format!("&MaxAudioChannels={c}"));
    }
    // Tizen freezes on HEVC in fMP4, so it gets MPEG-TS, as its HLS profile asks.
    if tizen {
        url.push_str(&format!("&SegmentContainer={}", t.container));
    }
    (url, t.container)
}

pub async fn playback_info(st: &Shared, auth: &Auth, item_id: &str, hex: &str, q: &Q, body: &Bytes) -> ApiResult {
    let body = body_json(body);
    let c = fetch(st, &auth.token, hex).await?;
    let max_bitrate = field_i64(&body, "MaxStreamingBitrate").or_else(|| q.int("maxstreamingbitrate"));
    if field_bool(&body, "EnableTranscoding").or_else(|| q.bool("enabletranscoding")) == Some(false) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "live channels play only through HLS".into()));
    }
    let play_session = new_play_session();
    let (url, container) = transcoding_url(item_id, &auth.token, &play_session, field(&body, "DeviceProfile"), max_bitrate, is_tizen(&auth.client));
    Ok(json(json!({ "MediaSources": [media_source(item_id, &c.name, url, &container)], "PlaySessionId": play_session })))
}

/// Ciniva's live request body for the query a TranscodingUrl carries.
pub fn live_request(q: &Q) -> Value {
    let video: Vec<String> = q.list("videocodec").iter().map(|s| s.to_ascii_lowercase()).filter(|c| ["h264", "hevc", "av1"].contains(&c.as_str())).collect();
    let audio: Vec<String> = q.list("audiocodec").iter().map(|s| s.to_ascii_lowercase()).collect();
    json!({
        "max_height": q.int("maxheight").unwrap_or(1080).clamp(240, 2160),
        "bitrate_kbps": q.int("maxstreamingbitrate").map(|b| b / 1000).unwrap_or(8000).clamp(1000, 20_000),
        "video_copy": if video.is_empty() { None } else { Some(video.join(",")) },
        "audio_copy": if audio.is_empty() { "aac".to_string() } else { audio.join(",") },
        "audio_channels": q.int("maxaudiochannels").unwrap_or(2).clamp(1, 8),
        "container": q.get("segmentcontainer").map(str::to_ascii_lowercase),
    })
}

async fn start(st: &Shared, token: &str, hex: &str, request: &Value) -> ApiResult<String> {
    let r: Value = st
        .ciniva
        .send(Some(token), Method::POST, &format!("/plugins/{IPTV}/live/{hex}"), request)
        .await?
        .json()
        .await
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e.to_string()))?;
    let sid = r["session"].as_str().ok_or_else(|| ApiError(StatusCode::BAD_GATEWAY, "Ciniva did not start the channel".into()))?.to_string();
    tracing::info!(channel = hex, session = %sid, title = %r["title"].as_str().unwrap_or(""), plan = %r["plan"], "live channel");
    st.live.lock().await.insert(format!("{hex}|{request}"), sid.clone());
    Ok(sid)
}

/// The Ciniva live session for this channel and request: a known one, or a new one. Ciniva shares sessions per
/// channel and plan, so concurrent starts end up on the same one.
async fn session(st: &Shared, token: &str, hex: &str, request: &Value) -> ApiResult<String> {
    if let Some(sid) = st.live.lock().await.get(&format!("{hex}|{request}")).cloned() {
        return Ok(sid);
    }
    start(st, token, hex, request).await
}

pub async fn master(st: &Shared, auth: &Auth, hex: &str, q: &Q, raw_query: &str) -> ApiResult {
    let request = live_request(q);
    session(st, &auth.token, hex, &request).await?;
    let bandwidth = request["bitrate_kbps"].as_i64().unwrap_or(8000) * 1000 + 192_000;
    Ok(playback::m3u8(format!("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH={bandwidth}\nlive.m3u8?{raw_query}\n")))
}

/// The rolling live playlist; a session that ended (nobody watched for 45 s) is started again.
pub async fn playlist(st: &Shared, auth: &Auth, hex: &str, q: &Q) -> ApiResult {
    let request = live_request(q);
    let mut sid = session(st, &auth.token, hex, &request).await?;
    let resp = match st.ciniva.raw(&auth.token, Method::GET, &format!("/live/{sid}/index.m3u8"), &HeaderMap::new()).await {
        Err(e) if e.0 == StatusCode::NOT_FOUND => {
            st.live.lock().await.remove(&format!("{hex}|{request}"));
            sid = start(st, &auth.token, hex, &request).await?;
            st.ciniva.raw(&auth.token, Method::GET, &format!("/live/{sid}/index.m3u8"), &HeaderMap::new()).await?
        }
        r => r?,
    };
    let text = resp.text().await.map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(playback::m3u8(rewrite_playlist(&text, &format!("live/{sid}/"), &auth.token)))
}

pub async fn live_playlist(State(st): State<Shared>, auth: Auth, Path(id): Path<String>, q: Q) -> ApiResult {
    let hex = channel_hex(&id).ok_or_else(not_found)?;
    playlist(&st, &auth, &hex, &q).await
}

pub async fn segment(State(st): State<Shared>, auth: Auth, Path((_id, sid, seg)): Path<(String, String, String)>) -> ApiResult {
    let resp = st.ciniva.raw(&auth.token, Method::GET, &format!("/live/{sid}/{seg}"), &HeaderMap::new()).await?;
    Ok(proxy_response(resp, None))
}

pub async fn empty_guide() -> Response {
    library::empty_result()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(logo: Option<&str>, favourite: bool) -> LiveChannel {
        LiveChannel {
            id: "f9758be9ce7e941a".into(),
            number: Some(7),
            name: "Imedi TV".into(),
            logo: logo.map(String::from),
            groups: vec!["Georgia".into()],
            favourite,
            ..Default::default()
        }
    }

    #[test]
    fn tv_channel_item_shape() {
        let cx = Ctx { server_id: "srv", token: "t" };
        let v = channel_item(&cx, &channel(Some("https://i.imgur.com/a.png"), true)).unwrap();
        assert_eq!(v["Type"], "TvChannel");
        assert_eq!(v["MediaType"], "Video");
        assert_eq!(v["Id"], Id::channel("f9758be9ce7e941a").unwrap());
        assert_eq!(v["ChannelNumber"], "7");
        assert_eq!(v["UserData"]["IsFavorite"], true);
        assert_eq!(v["ImageTags"]["Primary"], logo_tag("https://i.imgur.com/a.png"));
        assert!(v.get("RunTimeTicks").is_none());

        let v = channel_item(&cx, &channel(None, false)).unwrap();
        assert_eq!(v["ImageTags"], json!({}));
        assert_eq!(v["UserData"]["IsFavorite"], false);
        assert!(channel_item(&cx, &LiveChannel { id: "nothex".into(), ..Default::default() }).is_none());
    }

    #[test]
    fn plugin_queries() {
        let q = Q::parse(Some("StartIndex=100&Limit=50&IsFavorite=true&EnableFavoriteSorting=true&SortBy=SortName"));
        assert_eq!(plugin_query(&q, true), "favourites=1&favourites_first=1&sort=name&start=100&limit=50");
        assert_eq!(plugin_query(&q, false), "favourites=1&favourites_first=1&sort=name");
        let id = Id::channel("f9758be9ce7e941a").unwrap();
        assert_eq!(plugin_query(&Q::parse(Some(&format!("Ids={id},{}&SearchTerm=a b", Id::movie(1)))), true), "q=a%20b&ids=f9758be9ce7e941a");
    }

    #[test]
    fn live_source_and_request() {
        let tizen = json!({ "TranscodingProfiles": [{ "Type": "Video", "Protocol": "hls", "Container": "ts", "VideoCodec": "h264,hevc", "AudioCodec": "aac,ac3", "MaxAudioChannels": "6" }] });
        let (url, container) = transcoding_url("ID", "TOK", "PS", Some(&tizen), Some(20_000_000), true);
        assert_eq!(container, "ts");
        assert!(url.starts_with("/videos/ID/master.m3u8?PlaySessionId=PS&api_key=TOK&VideoCodec=h264,hevc&AudioCodec=aac,ac3"));
        assert!(url.ends_with("&MaxStreamingBitrate=20000000&MaxAudioChannels=6&SegmentContainer=ts"));

        let r = live_request(&Q::parse(url.split_once('?').map(|x| x.1)));
        assert_eq!(r["video_copy"], "h264,hevc");
        assert_eq!(r["audio_copy"], "aac,ac3");
        assert_eq!(r["container"], "ts");
        assert_eq!(r["audio_channels"], 6);
        assert_eq!(r["bitrate_kbps"], 20_000);

        let (url, _) = transcoding_url("ID", "TOK", "PS", None, None, false);
        let r = live_request(&Q::parse(url.split_once('?').map(|x| x.1)));
        assert_eq!(r["video_copy"], "h264");
        assert!(r["container"].is_null());

        let src = media_source("ID", "Imedi", url, "ts");
        assert_eq!(src["IsInfiniteStream"], true);
        assert_eq!(src["SupportsDirectPlay"], false);
        assert_eq!(src["TranscodingSubProtocol"], "hls");
        assert!(src.get("RunTimeTicks").is_none());
    }
}
