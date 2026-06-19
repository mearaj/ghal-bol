# Play Store listing — draft

Use in [Google Play Console](https://play.google.com/console) for `com.ghalbol`.

## App name

**Ghal Bol**

## Short description (≤80 chars)

P2P encrypted chat & voice/video calls. No phone number. You own your identity.

## Full description

Ghal Bol is realtime peer-to-peer messaging built for direct communication between online peers.

**What you get**
- Encrypted 1:1 chat with delivery and read receipts
- Voice/Video calls over a direct peer connection
- Connect via QR code or invite link — no phone number or cloud account
- Your identity and chat history stay on your device

**How it works**
- Each install creates a local cryptographic identity you control
- When both peers are online, messages sync directly between devices
- A lightweight coordination service helps peers find each other — it never stores your messages

**Who it’s for**
- People who want private, direct messaging without a central chat cloud
- Developers and early adopters comfortable with peer-to-peer networking

Ghal Bol is actively developed. Feedback welcome at ghalbol.com.

## Category

**Communication**

## Tags / keywords (internal notes)

P2P, encrypted chat, private messenger, QR invite, voice/video call, decentralized

## Content rating

Expect **Everyone** or **Teen** depending on questionnaire answers. No user-generated public content feeds; 1:1 chat only.

## Data safety (Play Console form — guidance)

| Question | Answer |
|----------|--------|
| Collects data? | Yes — limited technical data |
| Personal info | Public key / device endpoints sent to coordination server for presence |
| Messages | Not collected by developer servers; P2P between users |
| Encrypted in transit | Yes (peer encryption + HTTPS to coord) |
| Users can request deletion | Coord presence expires automatically; local data deleted on uninstall |
| Data shared with third parties | No sale; optional Google Play ML Kit for scanner |

Link privacy policy URL: `https://ghalbol.com/privacy` (publish [PRIVACY_POLICY.md](PRIVACY_POLICY.md) there first).

## Contact details

- Website: `https://ghalbol.com`
- Email: support@ghalbol.com (use your real support address)

## Release notes (v1.0.1 build 8)

- Native encrypted voice and video calls over direct P2P (no WebRTC)
- Improved LAN and WAN connectivity, Wi‑Fi handover, and coord/relay stability
- Delivery and read receipt fixes; hub unread sync on room leave
- Foreground service declarations for camera and microphone (Play Store explainers in `videos/`)
- Network status helper; assorted P2P and call UI fixes

Previous: v1.0.0 — initial release (P2P encrypted chat, QR invites, coordination at coord.ghalbol.com).

## Foreground service permissions (Play Console)

Path: **App content → Foreground service permissions** (or the declaration shown when uploading a build that uses `FOREGROUND_SERVICE_*`).

Ghal Bol runs libp2p in a separate Android process (`:p2p`, `GhalBolP2pService`). The same foreground service may combine types when the user has granted the matching runtime permissions:

| Permission | Play task checkbox | When used |
|------------|-------------------|-----------|
| `FOREGROUND_SERVICE_REMOTE_MESSAGING` | Transferring text messages from one device to another | After sign-in — keeps P2P chat networking active (notification: “Listening for messages”) |
| `FOREGROUND_SERVICE_MICROPHONE` | Background audio input | During an **active voice call** — live mic capture in the P2P networking process |
| `FOREGROUND_SERVICE_CAMERA` | Background camera streaming | During an **active video call** when the user enables video — live camera capture in the P2P networking process |

Explainer videos (~30 s, 1080×1920, multi-slide narrative) live in [`videos/`](../videos/). Regenerate with [`videos/build_playstore_fgs_explainers.sh`](../videos/build_playstore_fgs_explainers.sh) (runs `build_playstore_fgs_explainers.py`). After pushing to `main`, use GitHub URLs in Play Console.

### `FOREGROUND_SERVICE_REMOTE_MESSAGING`

**Video link:**

`https://github.com/mearaj/ghal-bol/blob/main/videos/FOREGROUND_SERVICE_REMOTE_MESSAGING_PLAYSTORE_EXPLANATION.mp4`

**Describe permission use:**

```
Ghal Bol provides encrypted peer-to-peer direct text messaging between users who have connected via QR or invite link.

When the user unlocks the app, Ghal Bol starts a foreground service (Android process :p2p) that keeps the device’s messaging network connection active. The service displays a persistent, low-priority notification titled “Ghal Bol” with the text “Listening for messages” so the user knows networking is active in the background.

This permission is used only for that remote-messaging task: maintaining connectivity so the app can receive inbound text messages, send outbound messages, and handle message delivery signals while the user is signed in—even if the main chat UI is not on screen.

The service must start as soon as the user is authenticated because messages can arrive at any time over the open peer connection; Android restricts or stops long-running background network work without a foreground service, which would disconnect peers and delay or lose messages.

The work cannot be paused or restarted on a schedule without tearing down active P2P sessions: stopping the service drops connections, and the user would not receive messages until they reopen the app and networking is established again. The service ends when the user signs out or the app stops the P2P session.
```

### `FOREGROUND_SERVICE_MICROPHONE`

**Video link:**

`https://github.com/mearaj/ghal-bol/blob/main/videos/FOREGROUND_SERVICE_MICROPHONE_PLAYSTORE_EXPLANATION.mp4`

**Describe permission use:**

```
Ghal Bol is a peer-to-peer encrypted messenger. Voice calls are a core feature—not an optional add-on.

When you start or accept a voice call, Ghal Bol must capture audio from your microphone and stream it in real time to your contact over your direct P2P connection. Audio is encrypted end-to-end; we do not record calls or store them on a server.

On Android, capture runs in Ghal Bol’s networking process (alongside encrypted chat) while the call is active. FOREGROUND_SERVICE_MICROPHONE is required so that process can perform continuous background audio input for the duration of the call. You see the in-call UI and Android’s microphone indicator; the ongoing foreground-service notification remains visible.

The microphone is used only during calls you initiate or accept—never for always-on listening outside a call. Capture must run without interruption while you are speaking; pausing the foreground task would drop audio to your contact. The microphone foreground type ends when you hang up, or when you sign out and the P2P session stops.
```

### `FOREGROUND_SERVICE_CAMERA`

**Video link:**

`https://github.com/mearaj/ghal-bol/blob/main/videos/FOREGROUND_SERVICE_CAMERA_PLAYSTORE_EXPLANATION.mp4`

**Describe permission use:**

```
Ghal Bol is a peer-to-peer encrypted messenger. Video calls are a core feature—not an optional add-on.

When you enable video during a call, Ghal Bol must capture your camera and stream live video to your contact over your direct P2P connection. Video is encrypted end-to-end and is not uploaded to our servers for storage or replay.

On Android, capture runs in Ghal Bol’s networking process while the call and video are active. FOREGROUND_SERVICE_CAMERA is required so that process can perform continuous background camera streaming for the call. You remain on the in-call screen with local and remote video; Android shows the camera privacy indicator.

The camera is used only during video calls you initiate or accept, and only while video is turned on—not for background surveillance. Streaming must continue without interruption while video is enabled; stopping the foreground task would freeze video for your contact. Camera capture and the camera foreground type end when you turn video off, hang up, or sign out.
```
