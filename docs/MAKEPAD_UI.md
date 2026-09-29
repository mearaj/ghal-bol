# Makepad 2 UI

`ghal_bol_app` is the only Ghal Bol UI. It is Makepad 2: Splash `script_mod!`, `Script` / `ScriptHook`, and `AppMain::script_mod`.

The dependency is pinned in `ghal_bol_app/Cargo.toml` to Makepad `dev` (`makepad-widgets` 2.0.0).

Screens call [`ghal_bol_core::host`](../ghal_bol_core/src/host.rs) in-process. Ack policy, outbox, and transcript merge stay in `ghal_bol_core`.

Where Makepad 2 still lacks a widget or platform affordance (for example a native navigation rail), the shell fills the gap with Splash layout primitives (`SolidView`, `ScrollXView`, buttons) so product behaviour stays complete.

## What the shell does

Debug builds use the `com.ghalbol.debug` data directory.

- Create, import a private key, import an encrypted keystore backup, unlock, or delete the device identity.
- After a newly generated identity, export an encrypted backup before chat opens.
- Start native connect and register saved contacts.
- Hub chrome: **≥720px** desktop = side rail + list + room; **&lt;720px** = bottom tabs + stacked list/room. Until the first resize, the shell assumes the design desktop width so the rail is not hidden. Room stays hidden until a contact is opened. Long lists, transcripts, sheets, Identity/More, unlock, and call UI **scroll** (see Scrolling below); toolbars scroll horizontally when actions overflow.
- **Back / forward** (mouse side-buttons, Android system / gesture back via `Event::BackPressed`, Escape, Alt+Left / Alt+Right, on-screen Back): one shared `navigate_back` / `navigate_forward`. Back unwinds **visible** depth in order — call → any sheet (Invite, Language, Password, …) → UI lock → open room → hub tab → quit — and skips no-op history entries. Opening a tab/sheet pins the UI underneath on the stack then pushes. Forward restores the next hub history entry. Incoming rings decline on back; dialing/connected ignore back. At hub/identity root, mouse/Android back quits (Escape / Alt+Left do not).
- Chat list with unread counts and availability text, open a chat, sent and received bubbles. Delivery ticks come from the transcript: `○` pending, `✓` sent, `✓✓` delivered, blue `✓✓` read.
- Share an invitation (link and QR). **Invite** shows a real QR image and the https link with **Copy link**. **Scan QR** reads a code from the camera. **QR picture** (More) reads a code from a PNG or JPEG. Paste an invitation, or add a contact by public key.
- Password and private-key fields use Makepad `is_password` masking plus a **Show** / **Hide** control (`toggle_is_password`). Makepad does not ship a built-in eye glyph on `TextInput`, so the shell fills that gap beside each secret field.
- Voice call and video call. While a call is ringing, dialing, or connected, the window switches to the call screen: status, local and remote pictures, accept, mute, speaker, and end. An incoming call plays a tone. An outgoing call plays a short ringback until it connects. **Speaker** routes the Android earpiece or loudspeaker; on the desktop it turns call playback on or off. **Add** marks an unknown contact as known. **Block** blocks the open chat. Sending a message also marks that contact known.
- Roster rows show unread counts and the contact’s availability text.
- Show the private key, export a keystore backup, change the app password.
- Delivery storage, in-app log, contacts, blocked contacts. About saves availability: Available, Busy, Away, In a call, Clear, or custom text (64 characters).
- Voice note: tap **Voice note** to record, tap again to send (up to 2 minutes). Tap a voice bubble to play it.
- Choose a file to send. Tap a file bubble to open it.
- Older messages load another page of the transcript.
- **Language** — worldwide catalogue (India Eighth Schedule + state languages, South/East/Southeast Asia, Middle East, Europe, Africa, Americas) with native names and English glosses, grouped by region. Bundled Noto script fonts (`resources/fonts`) plus Makepad LXGW WenKai cover Devanagari, Bengali, Tamil, Telugu, Gujarati, Kannada, Malayalam, Gurmukhi, Odia, Arabic, Hebrew, Thai, Sinhala, Myanmar, Khmer, Lao, Tibetan, Ethiopic, Thaana, Georgian, Armenian, Meetei Mayek, Ol Chiki, and CJK. Chrome strings exist for a starter set; other languages select correctly and fall back to English until translated. Preference is saved.

## Layout tokens

Shared Splash styles in `ghal_bol_app/src/main.rs`:

- **Quiet / Go / Stop** — rounded buttons with roomy inner padding (`Inset{left/right: 20–22, top/bottom: 14}`) and 10px corner radius. Never put label text flush against the pill edge.
- **Field** — rounded text inputs with the same `Inset` padding. Carets use an explicit dark `draw_cursor.color` (Makepad’s theme default is white, which vanishes on white fields). Selection uses a teal band.
- **RailTab** — full-width side-rail / bottom-nav control. Rail is **≥176px** (220px when the window is wide). `label_walk` Fill so labels wrap; **`label_align: Center`** so tab text is centered in the face (layout `align` alone does not center glyphs when the walk is Fill).
- **MenuRow** — full-width list actions with vertical margin between rows; `label_walk` Fill so long labels wrap.
- **Bubble** — chat chips with a max width, wrap, and padding.
- Parent rows use `spacing: 10`–`14` so buttons and cards do not sit flush.
- Action rows that can overflow use `flow: Right{wrap: true}` (Identity buttons) or a full-width scrolling strip (`ActionBar` / `HScroll` with `ScrollBars` + `use_vertical_finger_scroll`). Never leave Fit-width chips left-aligned in the bottom nav — those tabs stay equal `Fill` width.

Makepad 2 requires **`padding: Inset{…}` / `margin: Inset{…}`**. A plain `{left: …}` object does not apply and leaves labels flush to the border.

### Text overflow (never silent clip)

Makepad’s default `text_overflow` is **Clip** (hard cut, no “…”) — that is what produced “Identit”. The shell never relies on that default for constrained labels:

| Situation | Behaviour |
|-----------|-----------|
| Side rail / bottom tabs / menu rows / roster / bubbles | `label_walk` Fill + wrap; ellipsis after `max_lines` |
| Chat / roster titles | `Label` Fill + `max_lines: 1` + `text_overflow: Ellipsis` |
| Toolbars (roster actions, chat actions, composer, algo, call, bottom tabs) | Full-width **`HScroll`**; mouse wheel pans horizontally via `use_vertical_finger_scroll` |
| Fit-width action buttons in a wrap row | grow with text; parent wraps to the next line |

Hard clipping without scroll or ellipsis is a bug. Do not put a title and an action strip in the same `flow: Right` row on narrow widths — stack the title above a full-width `HScroll`.

## Scrolling (fill Makepad gaps)

Makepad 2 does not give free scrolling. The shell fills that gap:

- **`VScroll` (`ScrollYView`)** — roster list, chat transcript, Identity/More bodies, sheet body (languages, log, long forms), unlock screen, call screen. Content never truncates off-screen.
- **`HScroll` (`ScrollXView`)** — every action strip that can overflow. Uses `ScrollBars{…}` (not a bare `{…}` object) and `scroll_bar_x.use_vertical_finger_scroll: true` so a normal mouse wheel reaches off-screen actions. Drag-scroll works on every platform.
- **Nesting rule:** fixed chrome (title bars, search, composer, sheet Back + primary action) stays **outside** the vertical scroller. Horizontal toolbars are **siblings**, not children, of vertical scrollers so list scroll does not fight toolbar pan.

## Run

```bash
cargo run -p ghal_bol_app
./scripts/package_linux_release.sh
./scripts/build_android_app.sh
```

Linux login start is installed on launch (`~/.config/autostart`). An incoming call plays a short tone. Android uses the same crate via cargo-makepad; see [PRODUCTION_RELEASE.md](PRODUCTION_RELEASE.md).

Linux packages needed to link Makepad are listed in the Makepad README (`libx11`, Wayland, EGL/GL, ALSA).
