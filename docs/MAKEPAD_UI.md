# Makepad 2 UI

`ghalbol_app` is the only **Ghal Bol** (chat) UI under the **GhalBol** brand. It is Makepad 2: Splash `script_mod!`, `Script` / `ScriptHook`, and `AppMain::script_mod`. Face and shell lessons follow the same Makepad truths shipped in the Kumkaj sibling workspace (idle/active Overlay selection, intent→face, 720 breakpoint chrome) — see [`.cursor/rules/makepad-layout-faces.mdc`](../.cursor/rules/makepad-layout-faces.mdc). Each product keeps its own copy of faces; there is no shared UI crate.

The dependency is pinned in `apps/ghalbol/Cargo.toml` to a Makepad git revision (`makepad-widgets` 2.0.0; `scripts/build_android_app.sh` installs `cargo-makepad` from the same revision).

Screens call [`ghalbol_core::host`](../ghalbol_core/src/host.rs) in-process. Ack policy, outbox, and transcript merge stay in `ghalbol_core`.

Where Makepad 2 still lacks a widget or platform affordance (for example a native navigation rail), the shell fills the gap with Splash layout primitives (`SolidView`, `ScrollXView`, buttons) so product behaviour stays complete.

## What the shell does

Debug builds use the `com.ghalbol.debug` data directory.

**App icon / launcher:** Brand art is `docs/assets/app_icon.png`. Sized copies live under `apps/ghalbol/resources/` (`icon_{32,64,128,256,512,1024}.png`, `icon.ico`) and Android `resources/android/res/mipmap-*/ic_launcher.png` plus splash `drawable/launch_logo.png`. The custom Android manifest sets `android:icon` / `android:roundIcon`; launch theme uses `makepad_launch_background`. Desktop window icons come from `MAKEPAD_APP_ICON_*` in `.cargo/config.toml`. Linux `.desktop` install writes `~/.local/share/icons/hicolor/128x128/apps/com.ghalbol*.png`.

- Create, import a private key, import an encrypted keystore backup, unlock, or delete the device identity.
- After a newly generated identity, export an encrypted backup before chat opens. **Desktop** uses a save/open file picker. **Android** writes the backup to Downloads (plus an app-files copy); import opens the system document picker, and cancelling it offers pasting the keystore JSON instead.
- Start native connect and register saved contacts.
- Hub chrome: **≥720px** desktop = side rail + list/room (or peer pane); **&lt;720px** = scrollable bottom peer chips + stacked list/room. Until the first resize, the shell assumes the design desktop width so the rail is not hidden. Room stays hidden until a contact is opened. Long lists, transcripts, sheets, peer panes, unlock, and call UI **scroll** (see Scrolling below).
- **Hub peers (no More hub):** **Chats · Contacts · Invite · Settings**, plus **Lock** chrome. Peers are roots (`set_hub_tab`); sheets drill down with Back / hub history. Selection is Overlay **idle + active** faces (`RailTab`/`RailTabOn`, `NavChip`/`NavChipOn`) painted from `hub_tab` via `paint_nav_tabs` — Makepad `Button` has no selected bit. Nav WebP marks live under `apps/ghalbol/resources/nav/`.
- **Hub list AppBar** (Chats): title **Ghal Bol** on its own row, then a full-width **`ActionBar` / `HScroll`** of labeled shortcuts (New chat, Invite, Join, Scan, Delivery, Lock on narrow). Do **not** put the title beside the scroller in one `flow: Right` row — on Android that leftover-width layout disables horizontal pan. The strip must pan when actions overflow. Wide layout also keeps **Lock** on the side rail.
- **Back / forward** (mouse side-buttons, Android system / gesture back via `Event::BackPressed`, Escape, Alt+Left / Alt+Right, on-screen Back): one shared `navigate_back` / `navigate_forward` over a browser-style hub history stack (`hub_tab`, open room, sheet page). Opening a tab, room, or sheet pins the UI underneath then pushes. Back/forward only walk that stack (skipping no-op duplicates). Guards outside the stack: active call (`call_back_nav`), forced first-time backup (must Save), and UI lock dismiss. Incoming rings decline on back; dialing/connected ignore back. At hub/identity root, mouse/Android back quits (Escape / Alt+Left do not).
- Chat list with unread counts and availability text. Roster rows are rich `RosterRow` views (avatar initial, title, status, preview, Unknown chip, unread badge); **blue wash** for unread, **green wash** for the open room. Tap opens via `roster_list.on_item_tap` (rows have **no** `cursor`/Button — a drag scrolls, a plain tap opens). Empty search shows “Try a different search.” A highlighted roster row is not an open room (`host::set_open_room`).
- Chat room AppBar: **back · title** on one row; **call / video / ⋮** in an **`HScroll`** strip under it. The ⋮ menu covers Voice call / Video call / Share invitation / Join someone / Add contact / Block / Lock. Trust banner (Add / Block) while `is_known: false`. Titles are **`Label`** (not `Selectable` — that steals IME focus).
- Bubbles: Flutter chat chrome — **outgoing** right-aligned (`align.x: 1`) with **left gutter ~48** / right ~8; **inbound** left-aligned with **right gutter ~48** / left ~8. Chip is **`width: Fit{max: ~280}`** (content-sized), never a shared full-width slab. Body + meta row (**You**/peer caption + ticks) inside the chip. Delivery ticks use **FA PUA single glyphs** on WorldFonts (`\uf111` pending, `\uf00c` sent, `\uf560` check-double for delivered/read, `!` failed). Do **not** use Unicode `✓✓` — NotoSans has no ✓ so it falls through to two wide FA checks with a gap. **Failed / mailbox-expired** outbound text, voice, and attachments (with a local file) show `!` and **Tap to resend** (new upload — never extend after expiry). Attachments: Download / Open / Save. Voice bubbles tap to play. In-call **Flip camera** (Android) toggles front/back.
- Share an invitation (link and QR). **Invite** shows a real QR image and the https link with **Copy link**. **Scan QR** opens a sheet with a continuous live camera preview until a code is read or you go Back. **QR picture** (Invite peer) reads a code from a PNG or JPEG (Android: system document picker). Paste an invitation, or add a contact by public key.
- Password and private-key fields use Makepad `is_password` masking plus a **Show** / **Hide** control (`toggle_is_password`). Makepad does not ship a built-in eye glyph on `TextInput`, so the shell fills that gap beside each secret field.
- Voice call and video call. Dial / accept / end open the call screen **immediately** (not on a later poll). Voice chrome is peer avatar + name + status + labeled **Accept / Mute / Speaker / End** in an `HScroll` — **no** empty local/remote video panes. Video tiles appear only while `host::call_shows_video` is true. **Accept / Decline** appear only on an **incoming** ring — never on the caller’s dial (`Calling…`). An incoming call plays a tone. An outgoing call plays a short ringback until it connects. **Speaker** routes the Android earpiece or loudspeaker; on the desktop it turns call playback on or off. **Add** marks an unknown contact as known. **Block** blocks the open chat. Sending a message also marks that contact known.
- Show the private key, export a keystore backup, change the app password. **Copy** (invite link, public key) uses the desktop clipboard tools on Linux, else Makepad's clipboard.
- New-message notifications come from `ghalbol_core` (`message_notify.rs`) whenever inbound mail bumps unread: one per chat, body "New message" / "Voice message" / "File" (never the text), replaced by newer mail and removed when that chat opens. Linux uses desktop notifications; Android posts to the app's notification channel.
- Delivery storage, in-app log, contacts, blocked contacts. About saves availability: Available, Busy, Away, In a call, Clear, or custom text (64 characters).
- Voice note: mic opens a Flutter-style bar (Discard · red dot · elapsed · Send); Discard cancels; Send / second mic tap uploads (up to 2 minutes; “Recording too short” under ~400 ms). Tap a voice bubble to play it.
- Choose a file to send (desktop picker; Android system document picker — `host::send_picked_attachment` copies the `content://` file into app storage first). Attachment bubbles: Download / Open / Save.
- Older messages: **Older messages** chip, plus auto-load when the transcript is scrolled to the top. Stick-to-bottom follows new lines only when already near the end (or on own send / room open). Poll refreshes the open transcript only when `host::transcript_revision` moves.
- **Language** — worldwide catalogue (India Eighth Schedule + state languages, South/East/Southeast Asia, Middle East, Europe, Africa, Americas) with native names and English glosses, grouped by region. Bundled Noto script fonts (`resources/fonts`) plus Makepad LXGW WenKai cover Devanagari, Bengali, Tamil, Telugu, Gujarati, Kannada, Malayalam, Gurmukhi, Odia, Arabic, Hebrew, Thai, Sinhala, Myanmar, Khmer, Lao, Tibetan, Ethiopic, Thaana, Georgian, Armenian, Meetei Mayek, Ol Chiki, and CJK. Chrome strings exist for a starter set; other languages select correctly and fall back to English until translated. Preference is saved.

## Android chrome (safe area + keyboard)

- **Safe area:** root screens (`identity_view`, `hub_view`, `sheet_view`, `call_view`) pad with `mod.widgets.SAFE_INSET_PAD_{TOP,BOTTOM,LEFT,RIGHT}` so content clears the status bar / gesture nav. Makepad updates those heap values on `WindowGeomChange`.
- **Keyboard:** Messages must stay **above** the IME, never under it. `Window` body sets `keyboard_resize: true` so the IME **reflows** the hub (`chat_log` shrinks; `chat_bar` + composer stay visible above the keyboard). Do **not** use the default `KeyboardView` **pan** (that shoved the top bar and messages off-screen). Android **must** use `windowSoftInputMode="adjustNothing"` with that pairing — `adjustResize` plus `keyboard_resize` double-shrinks the surface and makes the IME dance on scroll. On IME show, pin `chat_log` scroll to the bottom so the latest messages remain in the shrunk viewport. IME appears only when a field is focused; on soft-keyboard **DidHide**, release composer key focus so a focused `TextInput` does not re-request the IME. **Do not** hide the keyboard on send.

## Palette (from `docs/assets/app_icon.png`)

Cool communication shell — **not** WhatsApp teal. Sampled from the interlocking bubble mark:

| Token | Hex | Use |
|-------|-----|-----|
| Surface / clear | `#EEF2F8` | Window clear, bars, rail |
| Hub wash | `#E4EAF3` | Hub background |
| Panel | `#FFFFFF` | Cards / panes |
| Ink | `#0B1220` | Body text (always dark on light faces) |
| Muted | `#5A6B7D` / `#7A8B9C` | Secondary copy |
| Border | `#C5D0DE` | Quiet outlines |
| Primary | `#0183FB` | Submit / `Go` / selection band |
| Primary hover/down | `#0170D8` / `#015CB5` | |
| Chrome navy | `#10236D` | Active peer faces, brand wordmark |
| Soft online | `#01C98A` | Sparingly (presence), not CTAs |
| Danger | `#C62828` | `Stop` |

**Contrast law:** light faces → dark ink in every animator state; chrome/primary → white ink always. Never flip ink lightness mid-hover.

## Layout tokens

Shared Splash styles in `apps/ghalbol/src/main.rs`. **Same job → same look** (intent→face):

| Intent | Face |
|--------|------|
| Quiet alt / secondary | **Quiet** / **StackSecondaryBtn** |
| Submit (one per form) | **Go** / **StackPrimaryBtn** |
| Destroy | **Stop** |
| Navigate elsewhere | **HubDestBtn** / **MenuRow** — never filled primary |
| Multi-word CTA stack | **ActionStack** (`flow: Down`) |
| Select peer (rail) | **RailTab** + **RailTabOn** Overlay |
| Select peer (phone) | **NavChip** + **NavChipOn** Overlay |
| Toolbar glyph | **IconBtn** / **IconGo** |
| Compact ActionBar chip | **BarChip** |

- Always `padding` **and** `label_walk.margin` so labels are never flush on the face edge. Full-width CTAs wrap + center — **no** `max_lines:1` + Ellipsis on translated buttons.
- **Never** put a Fit CTA beside Fill fields in one `Right` row (Makepad has no flex-shrink — letters stack vertically). Fields in the row; CTAs in **ActionStack** under them.
- **Never** `flow: Right{wrap: true}` with Fill children (hang / turtle spam).
- **IconBtn / IconGo** — 44×44 Font Awesome toolbar controls. WorldFonts falls through to `makepad_widgets` FA; those fonts must be listed in `app_main!(… font_assets: …)` or Android shows boxes. **Never** `set_text` translated labels onto these glyphs.
- **Field** — rounded inputs; caret `#0B1220`; selection band uses primary `#0183FB` alpha.
- **Selectable** — read-only multiline `TextInput` for copyable body copy on Settings/sheets. **Do not** use Selectable in the chat log or as the room/hub title (it takes key focus and steals the IME). Chat bodies and titles use **`Label`** / **`TextBubble`**.
- **Rail** ≥176px (220px when wide); left pad clears **NavMark** WebP overlays (`resources/nav/*.webp`).
- **RosterRow** — avatar + title/status/preview + Unknown chip + unread badge. No hit-capturing children.
- **TextBubble** — body Label + meta (caption + tick). **Bubble** (`Quiet`) only when the row must open/play media on tap.
- Parent rows use `spacing: 10`–`14`. Hub list AppBar: title **stacked above** a full-width **`ActionBar`/`HScroll`**. Chat AppBar: title row, then **`HScroll`** call/video/⋮. Bottom peers are Fit chips inside a horizontal scroller (not equal Fill).

Makepad 2 requires **`padding: Inset{…}` / `margin: Inset{…}`**. A plain `{left: …}` object does not apply and leaves labels flush to the border.

### Text overflow (never silent clip)

Makepad’s default `text_overflow` is **Clip** (hard cut, no “…”) — that is what produced “Identit”. The shell never relies on that default for constrained labels:

| Situation | Behaviour |
|-----------|-----------|
| Side rail / bottom tabs / menu rows / roster / bubbles | `label_walk` Fill + wrap; ellipsis after `max_lines` |
| Chat / roster titles | `Label` Fill + `max_lines: 1` + `text_overflow: Ellipsis` |
| Hub list AppBar | Title outside scroller + **`ActionBar`/`HScroll`** labeled actions |
| Chat action strip / algo / call controls | Full-width **`HScroll`**; mouse wheel pans via `use_vertical_finger_scroll` |
| Chat composer | Messenger-dense: **`spacing`/`padding` ~4**, mic/attach/send **40×40**, field padding **12×8**, **`height: Fit{max: FitBound.Abs(100)}`** so long drafts scroll instead of eating the transcript. **`View` `flow: Right`** with `Field` **`width: Fill`** — not `HScroll` with a Fixed-width field. Multiline: **`is_multiline: true`** + wrap. Desktop: **`submit_on_enter: true`**. Android: **do not** set `return_key_type: Send`. Composer **`IconGo` / `IconBtn`**: **`grab_key_focus: false`**. Wire send on **`pressed`**, not **`clicked`**. |
| Fit-width action buttons in a wrap row | grow with text; parent wraps to the next line |

Hard clipping without scroll or ellipsis is a bug. Do not put a title and an action strip in the same `flow: Right` row on narrow widths — stack the title above a full-width `HScroll`.

## Scrolling (fill Makepad gaps)

Makepad 2 does not give free scrolling. The shell fills that gap:

- **`VScroll` (`ScrollYView`)** — roster list, chat transcript, Contacts/Invite/Settings peer bodies, sheet body (languages, log, long forms), unlock screen, call screen. Content never truncates off-screen.
- **`HScroll` / `ActionBar` (`ScrollXView`)** — action strips that can overflow (hub actions, chat call/video/⋮, algo chips, call controls). **Height ~96px with top/bottom padding ~20** so Android has a clear finger band above/below the chips to pan — a Fit-to-chip strip, or a strip the same height as full `Quiet` buttons, leaves nowhere to drag (every touch hits a button). Hub labeled actions use compact **`BarChip`** (not full `Quiet`; do **not** name the class `Chip` — that clashes with `RosterRow.side.chip` Unknown badge). Chips/`IconBtn` stay `height: Fit` (or 44) and `align.y: 0.5` inside the taller strip. Hub title **"Ghal Bol"** stays a short static label (tight padding, no HScroll) — vertical space goes to the ActionBar, not the title. Uses `ScrollBars{…}` and `scroll_bar_x.use_vertical_finger_scroll: true` for mouse wheel. Never nest the strip beside a title in one `Right` row.
- **Nesting rule:** fixed chrome (title bars, search, composer, sheet Back + primary action) stays **outside** the vertical scroller. Horizontal toolbars are **siblings**, not children, of vertical scrollers so list scroll does not fight toolbar pan.
- **IME:** see **Android chrome** above — reflow, do not pan.

## Run

```bash
cargo run -p ghalbol_app
./scripts/package_linux_release.sh
./scripts/build_android_app.sh --release   # device UX / navigation
./scripts/build_android_app.sh             # debug (unoptimized codegen — not a size excuse)
```

**Android navigation:** tab / screen switches must redraw **only the active root** and skip no-op `apply_shell_layout` work. Multi-second stalls from redrawing every pane (or blaming APK size) are bugs. Prefer `--release` for device builds; keep `pass.clear_color` / theme window background at `#F7F7F7` so GL init does not flash black.

**Desktop caption chrome:** Makepad’s default dark theme paints min/max/close with `color_label_inner` (near-white) on a transparent `color_app_caption_bar`. With a light `#F7F7F7` clear that makes the glyphs invisible under Wayland custom chrome. `main_window` overrides the caption bar to `#F7F7F7` and the DesktopButton glyphs to `#111B21` (close keeps the red hover).

Linux login start is installed on launch (`~/.config/autostart`). An incoming call plays a short tone. Android uses the same crate via cargo-makepad; see [PRODUCTION_RELEASE.md](PRODUCTION_RELEASE.md).

Linux packages needed to link Makepad are listed in the Makepad README (`libx11`, Wayland, EGL/GL, ALSA).
