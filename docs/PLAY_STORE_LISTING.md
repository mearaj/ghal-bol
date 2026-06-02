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

## Release notes (v1.0.0)

Initial release: P2P encrypted chat, QR invites, voice/video calls, production coordination at coord.ghalbol.com.
