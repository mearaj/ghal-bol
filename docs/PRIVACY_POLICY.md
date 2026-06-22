# Ghal Bol — Privacy Policy

**Last updated:** June 2026  
**App:** Ghal Bol (`com.ghalbol`)  
**Publisher:** Ghal Bol / [ghalbol.com](https://ghalbol.com)

This Privacy Policy explains how the Ghal Bol mobile and desktop applications (“the app”) and the optional **coordination service** we operate handle information. Ghal Bol is a **peer-to-peer (P2P) messenger**: chat and call content is designed to travel **directly between users’ devices**, not to be stored as message history on our servers.

---

## Summary

| Topic | What we do |
|--------|------------|
| Account | **No** phone number, email, or cloud account required |
| Messages | **Not** collected or stored on Ghal Bol servers; sent **P2P** when peers are online |
| Identity | **Local** cryptographic key on your device; public key used for invites and discovery |
| Coordination server | Stores **only** presence and connection endpoints (see below) |
| Ads / analytics | **No** advertising SDKs; **no** third-party analytics in the core app |
| Sale of data | **We do not sell** your personal information |

---

## Who this applies to

This policy applies to anyone who installs or uses Ghal Bol and, where relevant, to data processed by the **default production coordination server** at `https://coord.ghalbol.com`. Advanced users may point the app at a **different coordination server** (including one they run themselves); that operator’s practices would then apply to coordination data sent to that server.

---

## What Ghal Bol is (and is not)

Ghal Bol provides **encrypted 1:1 text chat**, **delivery/read receipts**, and **voice/video calls** between people who connect via **QR code or invite link**. It is **not** a traditional cloud messenger: there is **no central chat archive** operated by us, and **offline message delivery to the cloud** is not provided. Messages are synchronized when **both peers are online** and connected over the network.

---

## Information stored on your device

The app keeps data **locally** under your control, including:

- **Cryptographic identity** — a secp256k1 key pair. The private key is stored in an encrypted **keystore** (`keystore_v1.json`, protected with your **app password** using Argon2id and ChaCha20-Poly1305).
- **Contacts** — names, public keys, chat previews, unread counts, and trust/block flags (`contacts_v1.json`).
- **Chat transcripts** — message text, timestamps, and delivery state on **your device only** (`chat_transcript_v1.json`).
- **Preferences** — including the coordination server URL you use.
- **In-app diagnostic log** (optional, for troubleshooting) — kept in app memory and exportable **only if you choose** to share it (e.g. via the system share sheet).

Your **app password** is used to unlock the keystore on the device. It is **not** transmitted to Ghal Bol servers.

You may **export** an encrypted keystore backup or (after re-entering your password) view/copy your private key. You may **delete** the local identity from the app or remove all app data by **uninstalling**.

---

## Peer-to-peer communication

When you send a message or place a call:

1. Content is protected for transport between peers using **cryptographic keys derived from your identities** (message envelopes are signed/sealed in the `ghal_bol` protocol; connections use **libp2p** with encrypted transports such as Noise over TCP/QUIC).
2. Payloads are intended to flow **directly between devices** when the network allows.
3. **We do not receive message bodies, attachments, or call audio/video** on the coordination server.

**Delivery and read receipts** are decided by the **recipient’s device** and exchanged over the same P2P channel; the sender’s app updates status only when those signals are received.

**LAN discovery:** On local networks, the app may use **mDNS** to find configured contacts without using the coordination server.

**NAT traversal:** If a direct connection is not possible, the app may use **libp2p relay circuits** on **Ghal Bol coordination servers** (co-located with `coord.ghalbol.com` or other configured coord hosts). Traffic remains encrypted at the transport/protocol layers. The app does **not** use public libp2p bootstrap peers or Kademlia DHT for peer discovery.

---

## Coordination server (presence only)

To help peers find each other on the internet, the app registers with a **coordination server** (default: `https://coord.ghalbol.com`). This service is **Tier 1 discovery only** — it does **not** store chats or call content.

When your app is online and registered, the server may store:

| Data | Purpose |
|------|---------|
| **Public key** (66-character hex identity) | Identify your peer for lookup |
| **Reachable endpoints** (e.g. IP addresses, ports, libp2p multiaddrs, transport capabilities) | Allow other peers to dial you |
| **Optional IPv4 / IPv6 hints** | Assist connection setup |
| **Last heartbeat timestamp** | Know whether you are currently reachable |

Registration uses a **challenge–response**: you sign a server nonce with your private key so only the holder of that identity can register endpoints for that public key.

**Retention:** Presence records are **ephemeral**. Entries expire after heartbeats stop (short TTL; stale peers are purged). There is **no** message archive on the server.

**HTTPS:** Production coordination uses TLS. The app may be configured to use other URLs (including local development servers).

---

## Voice and video calls

Calls use **native encrypted media** over the **same libp2p peer connection** as chat (`/ghal-bol/call/1.0.0` for voice, `/ghal-bol/call-video/1.0.0` for video). **Call signaling** (invite, accept, hangup, video on/off) is exchanged over the **same P2P messaging channel**, not through our coordination server.

Media is intended to flow **peer-to-peer** (direct LAN or relayed via our coordination relay when NAT requires it). We do not operate a media server that decrypts call content.

---

## Invite links and QR codes

Invites (e.g. `https://ghalbol.com/connect/<public_key_hex>` or `ghalbol://connect/...`) contain your **public key** and optional display alias. They do **not** embed your private key. Anyone you share an invite with can add you as a contact and attempt to connect while you are online.

---

## Android background processing

On Android, networking runs in a separate **foreground service** (`:p2p` process) so P2P can continue when the app is in the background. This shows a **persistent notification** while the service is active. The service type is **`remoteMessaging`** (keeping a network path open to receive peer traffic), not cloud backup or file sync.

---

## Permissions

The app requests OS permissions only for features you use:

| Permission | Why |
|------------|-----|
| **Internet / network state** | P2P connections and coordination lookups |
| **Wi‑Fi multicast** (Android) | Local (LAN) peer discovery via mDNS |
| **Camera** | Scan invitation QR codes |
| **Microphone** | Voice and video calls |
| **Notifications** (Android) | Optional alerts when enabled |
| **Foreground service** (Android) | Background P2P process described above |

You can deny permissions; related features may not work (e.g. no QR scan without camera).

---

## Third-party services

| Service | Role | Data involved |
|---------|------|----------------|
| **Ghal Bol coordination server** | Presence and endpoint lookup | Public key, endpoints, heartbeats (see above) |
| **Ghal Bol relay** (co-located with coord, when needed) | NAT traversal for encrypted P2P (chat + calls) | Encrypted transit only; not message storage by Ghal Bol |
| **QR scanner plugin** (`mobile_scanner`) | Decode QR on device | Processing is **on-device**; may use platform camera/ML APIs per device vendor |

We do **not** integrate advertising networks or third-party analytics SDKs in the core application code paths described in our open-source tree.

**Google Play:** Distribution through Google Play is subject to [Google’s policies](https://policies.google.com/) in addition to this document.

---

## What we do not do

- We do **not** require a phone number, email address, or social login.
- We do **not** store your chat transcripts or call recordings on Ghal Bol coordination servers.
- We do **not** sell your personal information.
- We do **not** use your messages to train machine-learning models.

---

## Children

Ghal Bol is **not directed at children under 13**. We do not knowingly collect personal information from children. If you believe a child has provided information through the app, contact us and we will take appropriate steps.

---

## Security

We use industry-standard cryptography for identity and message sealing on the wire. No system is perfectly secure; you are responsible for your **app password**, **device security**, and **who you share invites or key backups with**. Importing a private key from another system (e.g. a cryptocurrency wallet) is discouraged because it can put unrelated funds at risk.

---

## Your choices

- **Use or change coordination server** in app/environment configuration.
- **Export or delete** local identity and data as described above.
- **Block contacts** locally (`is_blocked`); this affects your device’s behavior, not a global account system.
- **Uninstall** the app to remove local data from your device (coordination presence will expire on its own).

Because there is no central chat account, **“delete my messages from the cloud”** does not apply — we do not host them. Peers may retain their own copy of conversations on their devices.

---

## International users

If you use Ghal Bol from outside the country where our coordination infrastructure is hosted, your **public key and network endpoints** will be processed where that server runs (and on peers’ devices worldwide). P2P traffic routes according to the internet path between devices.

---

## Changes to this policy

We may update this Privacy Policy from time to time. The **“Last updated”** date at the top will change when we do. Continued use of the app after an update means you accept the revised policy. Material changes may also be noted in release notes or on [ghalbol.com](https://ghalbol.com).

---

## Contact

Questions or privacy requests:

- **Email:** privacy@ghalbol.com  
- **Website:** https://ghalbol.com  

Replace the email above with your live support address if it differs on the website.

---

## Play Store / repository hosting

For **Google Play Console**, provide a **public HTTPS URL** to this document, for example:

- `https://github.com/<your-org>/<your-repo>/blob/main/docs/PRIVACY_POLICY.md` (GitHub renders markdown), or  
- `https://ghalbol.com/privacy` if you mirror this file on your website.

Keep the URL stable across releases.
