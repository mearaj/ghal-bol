# Warning: AI agent should avoid making changes to this file.

## Current issues to resolve
The app should have full eye on the network everytime, so it knows
what active network it's using, whether internet is active or not.
When app registers itself to the coord/relay server then it doesn't 
requires to re register again and again. Only in the situation where
it's global reachable network address changes, or it's last
registration failed, or when it is reached a threshold of 15 mins after
which it is required to register itself again, or if some logic finds it
necessary to register.
Now your job is to fix/implement without causing any regression, 
without breaking any working functionality and understanding the 
impact of removing/changing the code w.r.t previous code.



# Now
Registering again and again on coord server is not a good 
practise... we need the connection to be strong, reliable, steady 
and that doesn't mean flooding or polluting the network... 
the app should see if the internet network is changed i.e. it's global
public reachability address is changed, if it's not and it's already 
registered on coord server then there should be no need to register.
This should be reliable and accurate. There should never be the situation
where app thinks it's already registered and it's valid address is
reflecting on coord/relay server and it actually is not.
But for this to work we need to keep full eye on the internet network and
also whenever the background service is restarted, this check should


# Next
Our app still takes time to discover the 
peer, connect to it. The connection is never stable, stable 
for few or more seconds and unstable (no p2p messages reach
or ack reached) for many minutes. Not at all reliable. 
We aren't sure if our messages will reach, the feeling of confidence
isn't there to the user like whats app even though both the 
peers are always online. Switching from LAN to WAN or vice 
versa takes long to time to regain connectivity. 
We don't have change password system. 
The ui shouldn't try to pull all the messages at once, 
infact as user scrolls it should load messages making sure scroll
doesn't reach a point where messages are still there but miscalculation
of scroll area and behaviour which causes prevention of messages pulling
 because no trigger occurs. We aren't allowing sharing of 
 attachments, docs, photos, etc yet. We don't have status like WhatsApp.


# Story, anything in the docs that violates this story should be overriden and this story should be preferred over it

After user login for the first time then background service (ghal_bol) should
start running. It should watch the network continuously, should know the status 
of the internet, should be quick to figure out it's global reachable address and also it's 
LAN address and as soon as it's global reachable address is found it should regularly
register itself at the coord server. WAN should always work if internet is active for
both the peers and if coord server is reachable. Now if any peer is found on LAN then only 
for that peer LAN should be used and in case if LAN is lost then again it should repeat the
regular process of WAN and this switch shouldn't impact user experience; the user shouldn't see any
weird behavior. If the coord server is unreachable, the app must **not** fall back to libp2p **for WAN peer
discovery** (no Kademlia, no public bootstrap peer directory) — coord/relay is
required for WAN lookup. **libp2p stays** for transport: relay circuits, NAT hole-punch
(DCUtR), mDNS on LAN, Noise streams, ping, AutoNAT. LAN ability must not be impacted when
coord HTTP is down, and the app must keep retrying all configured coord servers on a regular
interval. The ultimate goal is strong, reliable, and smooth interaction between peers. We already
have the coord/relay server(s) and libp2p, which should be more than enough for smooth interaction
over WAN/LAN.

**UI session (single source of truth — agents must not break these):**
- **`ghal_bol` owns** ack policy, outbox, delivery/read ticks, contacts, transcript, dial, coord/relay. **`ghal_bol_server` does not know UI state.**
- **`ghal_bol_ui` reports only** app visible + open room via **`GhalBolUiSession`** → native `p2p_sync_ui_session`. No separate foreground/read/visible RPCs from Dart.
- Poll/`peer_connected`/`isStreamReady` in Flutter are **display hints only** — never gate sends or acks in Dart.

**Connectivity rules (agents must not break these):**
- Keep P2P active in every scenario where both peers can reach each other.
- When a peer leaves LAN, fall back to WAN via coord/relay immediately (additive dial; do not wait on slow timers).
- When both peers are on LAN (mDNS), shift to the direct LAN path immediately; WAN stays as backup.
- Register on **all** configured coord URLs; lookup peers in order and stop on first success; on disconnect with active internet, repeat lookup from the first server.
- **WAN peer addresses are always live** — `GET /v1/peers/{public_key}` on coord (never reuse stale cached **contact** dial addrs). LAN peers via mDNS. Coord relay coordinates (`GET /v1/relay`) may be disk-cached for boot only — **invalidate when relay TCP fails or after dev server restart** (bore port changes every run).
- **Network switch (Wi‑Fi ↔ mobile ↔ other)** must be prepared in `:p2p` before the user sends: re-register on coord with new endpoints, reset lookup backoff, urgent live coord lookup, reopen chat streams so delivery/read acks and outbox resume without user action.
- **Messages and acks must never silently stall** because a libp2p connection looks up but the `/ghal-bol/msg/1.0.0` stream died — reopen the stream and retry queued `ack_received` / `ack_read` and pending outbox lines.

**Call signaling (UI must match wire — do not touch native media quality/latency):**
- Outbound **ringing** only after `invite` is on the DM stream (`call_signal_sent`), not when FFI returns.
- Drop queued/stale `invite` frames when the call ended or age > 45s — no ghost rings minutes later.
- Caller shows **Answered** as soon as `accept` arrives; media connect is a separate phase.
- Hangup/reject purges pending signals for that `call_id` on both native queue and poll buffer.
- **Android:** `:p2p` posts the full-screen incoming-call notification when an `invite` arrives on the wire (UI process may be killed). Tapping it must restore ringing UI via `p2p_call_status` + poll.

The current biggest issue is that agents fail to understand this and break at least one functionality.
We don't want to use bootstrap peers and Kademlia to **find peers over WAN** via libp2p as we
did before — coord/relay is the WAN directory now. libp2p is still used wherever transport needs
it (relay reservation, hole-punch, mDNS, streams); we avoid libp2p's peer-directory behaviour
that floods the network.
Second change: instead of a single coord/relay server, the app accepts an **array** of coord
servers via `GHAL_BOL_COORD_URLS` in
`env/.env.development` and `env/.env.production` — no hardcoded URLs in code. For now each
list has one entry; more can be added later.
Register on **all** configured coord servers. When looking up a peer, try each server in order; if lookup succeeds on any server,
stop — no need to query the rest. If the connection between peers drops while internet is still
active, repeat lookup from the first server.
