# Ghal Bol — product backlog

Wish list only. Design: [DESIGN.md](DESIGN.md), [AGENTS.md](../AGENTS.md).

- [X] HTTPS invite links open Ghal Bol on Android (not only the browser)
- [ ] Voice calls: ringtone and better audio / call UX
- [ ] Editable contact display name
- [ ] Video calls: reliability, quality, bug fixes
- [ ] User status text (e.g. WhatsApp-style)
- [ ] Profile picture
- [ ] Typing indicator (“typing…”)
- [ ] Chat background and themes (app presets + custom)
- [ ] Send photos and videos in chat
- [ ] Message reactions and message action menu (e.g. WhatsApp-style)
- [ ] Allow Group Chats.
- [ ] Voice and Video calls after working fine, should be encrypted p2p e2e.
- [ ] Show if the user is online, when he was last online.
- [ ] Allow blocking contacts, currently it's broken and there is no way the user can block anyone.
- [ ] Enable voice messages similar to whats app way.

## Current Todo 
Phase 1 — Better voice calls (phone / WhatsApp feel)
Improve the call experience and audio quality: clearer sound, stable connection, proper ring/back tones, sensible mic/speaker behavior, fast connect, clean in-call UI. Today signaling runs in Rust over the encrypted DM stream; audio/video media is WebRTC in Flutter (call_webrtc.dart) with basic STUN only — that’s where most quality/UX work lives (codecs, echo/noise, device routing, TURN for hard NATs, ring UX, etc.). This matches the open item in docs/TODO.md: “Voice calls: ringtone and better audio / call UX”.

Phase 2 — Encrypt call media like messages
After Phase 1 works well, lock down the actual voice path so someone on the network only sees encrypted traffic, not usable audio. Signaling is already signed/encrypted on the DM stream (ghal_bol_call_v1). Call audio today is standard WebRTC (SRTP) — peer-to-peer, but not the same stack as text DMs (X25519 sealed ghal_bol_msg_v1). Phase 2 means designing true E2E for media (keys tied to contact keys, no server decode), likely in Rust + WebRTC hooks or a chosen E2E media scheme — bigger step, done only after calls are solid.

Order: quality and UX first, then media encryption — same as your TODO line 16.

