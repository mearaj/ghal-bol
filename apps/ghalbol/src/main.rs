//! Makepad 2 desktop shell. Product logic stays in `ghalbol_core::host`.
//!
//! Splash uses `script_mod!` (Makepad 2).

// Production Windows exe opens without a console window (`build.rs` sets the cfg).
#![cfg_attr(all(windows, ghalbol_hide_console), windows_subsystem = "windows")]

pub use makepad_widgets;

mod i18n;
mod phrases;

use makepad_widgets::*;

use ghalbol_core::host::{self, ChatLine, RosterEntry, UnlockedSession};

// Android/APK only ships fonts listed here (plus the International set).
// WorldFonts `self:resources/fonts/…` members must match these logical paths
// or glyphs render as boxes / missing icons.
app_main!(
    App,
    font_assets: [
        "makepad_widgets/resources/fa-solid-900.ttf",
        "ghalbol_app/resources/fonts/NotoSans-Regular.ttf",
        "ghalbol_app/resources/fonts/NotoSansDevanagari-Regular.ttf",
        "ghalbol_app/resources/fonts/NotoSansBengali-Regular.ttf",
        "ghalbol_app/resources/fonts/NotoSansTamil-Regular.ttf",
        "ghalbol_app/resources/fonts/NotoSansTelugu-Regular.ttf",
        "ghalbol_app/resources/fonts/NotoSansGujarati-Regular.ttf",
        "ghalbol_app/resources/fonts/NotoSansKannada-Regular.ttf",
        "ghalbol_app/resources/fonts/NotoSansMalayalam-Regular.ttf",
        "ghalbol_app/resources/fonts/NotoSansGurmukhi-Regular.ttf",
        "ghalbol_app/resources/fonts/NotoSansOriya-Regular.ttf",
        "ghalbol_app/resources/fonts/NotoSansArabic-Regular.ttf",
        "ghalbol_app/resources/fonts/NotoSansHebrew-Regular.ttf",
        "ghalbol_app/resources/fonts/NotoSansThai-Regular.ttf",
        "ghalbol_app/resources/fonts/NotoSansSinhala-Regular.ttf",
        "ghalbol_app/resources/fonts/NotoSansMyanmar-Regular.ttf",
        "ghalbol_app/resources/fonts/NotoSansKhmer-Regular.ttf",
        "ghalbol_app/resources/fonts/NotoSansLao-Regular.ttf",
        "ghalbol_app/resources/fonts/NotoSansTibetan-Regular.ttf",
        "ghalbol_app/resources/fonts/NotoSansEthiopic-Regular.ttf",
        "ghalbol_app/resources/fonts/NotoSansThaana-Regular.ttf",
        "ghalbol_app/resources/fonts/NotoSansGeorgian-Regular.ttf",
        "ghalbol_app/resources/fonts/NotoSansArmenian-Regular.ttf",
        "ghalbol_app/resources/fonts/NotoSansMeeteiMayek-Regular.ttf",
        "ghalbol_app/resources/fonts/NotoSansOlChiki-Regular.ttf",
    ]
);

script_mod! {
    use mod.prelude.widgets.*

    // Multi-script UI fonts: Makepad falls through FontFamily members when a
    // glyph is missing. Cover Indian scheduled scripts + major world scripts.
    // CJK (zh/ja/ko) comes from Makepad's LXGW WenKai; emoji from Noto Color.
    let WorldFonts = FontFamily{
        latin := FontMember{res: crate_resource("self:resources/fonts/NotoSans-Regular.ttf") asc: -0.1 desc: 0.0}
        devanagari := FontMember{res: crate_resource("self:resources/fonts/NotoSansDevanagari-Regular.ttf") asc: 0.0 desc: 0.0}
        bengali := FontMember{res: crate_resource("self:resources/fonts/NotoSansBengali-Regular.ttf") asc: 0.0 desc: 0.0}
        tamil := FontMember{res: crate_resource("self:resources/fonts/NotoSansTamil-Regular.ttf") asc: 0.0 desc: 0.0}
        telugu := FontMember{res: crate_resource("self:resources/fonts/NotoSansTelugu-Regular.ttf") asc: 0.0 desc: 0.0}
        gujarati := FontMember{res: crate_resource("self:resources/fonts/NotoSansGujarati-Regular.ttf") asc: 0.0 desc: 0.0}
        kannada := FontMember{res: crate_resource("self:resources/fonts/NotoSansKannada-Regular.ttf") asc: 0.0 desc: 0.0}
        malayalam := FontMember{res: crate_resource("self:resources/fonts/NotoSansMalayalam-Regular.ttf") asc: 0.0 desc: 0.0}
        gurmukhi := FontMember{res: crate_resource("self:resources/fonts/NotoSansGurmukhi-Regular.ttf") asc: 0.0 desc: 0.0}
        odia := FontMember{res: crate_resource("self:resources/fonts/NotoSansOriya-Regular.ttf") asc: 0.0 desc: 0.0}
        arabic := FontMember{res: crate_resource("self:resources/fonts/NotoSansArabic-Regular.ttf") asc: 0.0 desc: 0.0}
        hebrew := FontMember{res: crate_resource("self:resources/fonts/NotoSansHebrew-Regular.ttf") asc: 0.0 desc: 0.0}
        thai := FontMember{res: crate_resource("self:resources/fonts/NotoSansThai-Regular.ttf") asc: 0.0 desc: 0.0}
        sinhala := FontMember{res: crate_resource("self:resources/fonts/NotoSansSinhala-Regular.ttf") asc: 0.0 desc: 0.0}
        myanmar := FontMember{res: crate_resource("self:resources/fonts/NotoSansMyanmar-Regular.ttf") asc: 0.0 desc: 0.0}
        khmer := FontMember{res: crate_resource("self:resources/fonts/NotoSansKhmer-Regular.ttf") asc: 0.0 desc: 0.0}
        lao := FontMember{res: crate_resource("self:resources/fonts/NotoSansLao-Regular.ttf") asc: 0.0 desc: 0.0}
        tibetan := FontMember{res: crate_resource("self:resources/fonts/NotoSansTibetan-Regular.ttf") asc: 0.0 desc: 0.0}
        ethiopic := FontMember{res: crate_resource("self:resources/fonts/NotoSansEthiopic-Regular.ttf") asc: 0.0 desc: 0.0}
        thaana := FontMember{res: crate_resource("self:resources/fonts/NotoSansThaana-Regular.ttf") asc: 0.0 desc: 0.0}
        georgian := FontMember{res: crate_resource("self:resources/fonts/NotoSansGeorgian-Regular.ttf") asc: 0.0 desc: 0.0}
        armenian := FontMember{res: crate_resource("self:resources/fonts/NotoSansArmenian-Regular.ttf") asc: 0.0 desc: 0.0}
        meetei := FontMember{res: crate_resource("self:resources/fonts/NotoSansMeeteiMayek-Regular.ttf") asc: 0.0 desc: 0.0}
        olchiki := FontMember{res: crate_resource("self:resources/fonts/NotoSansOlChiki-Regular.ttf") asc: 0.0 desc: 0.0}
        chinese := FontMember{res: crate_resource("makepad_widgets:resources/LXGWWenKaiRegular.ttf") asc: 0.0 desc: 0.0}
        // Packaged via International + font_assets (not self: copy — that was
        // missing from the APK and made every toolbar glyph a box).
        icons := FontMember{res: crate_resource("makepad_widgets:resources/fa-solid-900.ttf") asc: 0.0 desc: 0.0}
        emoji := FontMember{res: crate_resource("makepad_widgets:resources/NotoColorEmoji.ttf") asc: 0.0 desc: 0.0}
    }
    let WorldFontsBold = FontFamily{
        latin := FontMember{res: crate_resource("self:resources/fonts/NotoSans-Regular.ttf") asc: -0.1 desc: 0.0}
        devanagari := FontMember{res: crate_resource("self:resources/fonts/NotoSansDevanagari-Regular.ttf") asc: 0.0 desc: 0.0}
        bengali := FontMember{res: crate_resource("self:resources/fonts/NotoSansBengali-Regular.ttf") asc: 0.0 desc: 0.0}
        tamil := FontMember{res: crate_resource("self:resources/fonts/NotoSansTamil-Regular.ttf") asc: 0.0 desc: 0.0}
        telugu := FontMember{res: crate_resource("self:resources/fonts/NotoSansTelugu-Regular.ttf") asc: 0.0 desc: 0.0}
        gujarati := FontMember{res: crate_resource("self:resources/fonts/NotoSansGujarati-Regular.ttf") asc: 0.0 desc: 0.0}
        kannada := FontMember{res: crate_resource("self:resources/fonts/NotoSansKannada-Regular.ttf") asc: 0.0 desc: 0.0}
        malayalam := FontMember{res: crate_resource("self:resources/fonts/NotoSansMalayalam-Regular.ttf") asc: 0.0 desc: 0.0}
        gurmukhi := FontMember{res: crate_resource("self:resources/fonts/NotoSansGurmukhi-Regular.ttf") asc: 0.0 desc: 0.0}
        odia := FontMember{res: crate_resource("self:resources/fonts/NotoSansOriya-Regular.ttf") asc: 0.0 desc: 0.0}
        arabic := FontMember{res: crate_resource("self:resources/fonts/NotoSansArabic-Regular.ttf") asc: 0.0 desc: 0.0}
        hebrew := FontMember{res: crate_resource("self:resources/fonts/NotoSansHebrew-Regular.ttf") asc: 0.0 desc: 0.0}
        thai := FontMember{res: crate_resource("self:resources/fonts/NotoSansThai-Regular.ttf") asc: 0.0 desc: 0.0}
        sinhala := FontMember{res: crate_resource("self:resources/fonts/NotoSansSinhala-Regular.ttf") asc: 0.0 desc: 0.0}
        myanmar := FontMember{res: crate_resource("self:resources/fonts/NotoSansMyanmar-Regular.ttf") asc: 0.0 desc: 0.0}
        khmer := FontMember{res: crate_resource("self:resources/fonts/NotoSansKhmer-Regular.ttf") asc: 0.0 desc: 0.0}
        lao := FontMember{res: crate_resource("self:resources/fonts/NotoSansLao-Regular.ttf") asc: 0.0 desc: 0.0}
        tibetan := FontMember{res: crate_resource("self:resources/fonts/NotoSansTibetan-Regular.ttf") asc: 0.0 desc: 0.0}
        ethiopic := FontMember{res: crate_resource("self:resources/fonts/NotoSansEthiopic-Regular.ttf") asc: 0.0 desc: 0.0}
        thaana := FontMember{res: crate_resource("self:resources/fonts/NotoSansThaana-Regular.ttf") asc: 0.0 desc: 0.0}
        georgian := FontMember{res: crate_resource("self:resources/fonts/NotoSansGeorgian-Regular.ttf") asc: 0.0 desc: 0.0}
        armenian := FontMember{res: crate_resource("self:resources/fonts/NotoSansArmenian-Regular.ttf") asc: 0.0 desc: 0.0}
        meetei := FontMember{res: crate_resource("self:resources/fonts/NotoSansMeeteiMayek-Regular.ttf") asc: 0.0 desc: 0.0}
        olchiki := FontMember{res: crate_resource("self:resources/fonts/NotoSansOlChiki-Regular.ttf") asc: 0.0 desc: 0.0}
        chinese := FontMember{res: crate_resource("makepad_widgets:resources/LXGWWenKaiBold.ttf") asc: 0.0 desc: 0.0}
        icons := FontMember{res: crate_resource("makepad_widgets:resources/fa-solid-900.ttf") asc: 0.0 desc: 0.0}
        emoji := FontMember{res: crate_resource("makepad_widgets:resources/NotoColorEmoji.ttf") asc: 0.0 desc: 0.0}
    }
    mod.theme.font_regular.font_family = WorldFonts
    mod.theme.font_label.font_family = WorldFonts
    mod.theme.font_bold.font_family = WorldFontsBold
    mod.theme.font_italic.font_family = WorldFonts
    mod.theme.font_bold_italic.font_family = WorldFontsBold
    // Widget classes copy the theme face when they are defined, which is
    // before this script runs. Point the classes themselves at WorldFonts
    // so every label, button, and field falls through to the script fonts.
    mod.widgets.Label.draw_text.text_style.font_family = WorldFonts
    mod.widgets.Button.draw_text.text_style.font_family = WorldFonts
    mod.widgets.TextInput.draw_text.text_style.font_family = WorldFonts

    let state = mod.state

    // Intent → face (same job → same look). Quiet/Go/Stop keep legacy names as
    // aliases for secondary / submit-primary / destroy so call sites stay stable.
    // Never promote a navigate/hub jump to filled primary — use HubDestBtn / MenuRow.
    // Palette: cool shell + electric blue CTAs + navy chrome (from app_icon.png).
    let Quiet = Button{
        width: Fit
        height: Fit
        padding: Inset{left: 20. right: 20. top: 14. bottom: 14.}
        margin: Inset{left: 0. right: 0. top: 0. bottom: 0.}
        align: Center
        label_walk: Walk{width: Fit, height: Fit, margin: Inset{left: 2. right: 2. top: 0. bottom: 0.}}
        grab_key_focus: false
        reset_hover_on_click: true
        draw_bg.border_size: 1.0
        draw_bg.border_radius: 10.0
        draw_bg.color: #FFFFFF
        draw_bg.color_hover: #DCE4F0
        draw_bg.color_down: #C5D0DE
        draw_bg.color_focus: #FFFFFF
        draw_bg.border_color: #C5D0DE
        draw_bg.border_color_hover: #C5D0DE
        draw_bg.border_color_down: #C5D0DE
        draw_bg.border_color_focus: #0183FB
        draw_text.color: #0B1220
        draw_text.color_hover: #0B1220
        draw_text.color_down: #0B1220
        draw_text.color_focus: #0B1220
        draw_text.text_style.font_size: 13
    }
    let Go = Button{
        width: Fit
        height: Fit
        padding: Inset{left: 22. right: 22. top: 14. bottom: 14.}
        margin: Inset{left: 0. right: 0. top: 0. bottom: 0.}
        align: Center
        label_walk: Walk{width: Fit, height: Fit, margin: Inset{left: 2. right: 2. top: 0. bottom: 0.}}
        grab_key_focus: false
        reset_hover_on_click: true
        draw_bg.border_size: 1.0
        draw_bg.border_radius: 10.0
        draw_bg.color: #0183FB
        draw_bg.color_hover: #0170D8
        draw_bg.color_down: #015CB5
        draw_bg.color_focus: #0183FB
        draw_bg.border_color: #015CB5
        draw_bg.border_color_hover: #015CB5
        draw_bg.border_color_down: #015CB5
        draw_bg.border_color_focus: #015CB5
        draw_text.color: #FFFFFF
        draw_text.color_hover: #FFFFFF
        draw_text.color_down: #FFFFFF
        draw_text.color_focus: #FFFFFF
        draw_text.text_style.font_size: 13
    }
    let Stop = Button{
        width: Fit
        height: Fit
        padding: Inset{left: 20. right: 20. top: 14. bottom: 14.}
        margin: Inset{left: 0. right: 0. top: 0. bottom: 0.}
        align: Center
        label_walk: Walk{width: Fit, height: Fit, margin: Inset{left: 2. right: 2. top: 0. bottom: 0.}}
        grab_key_focus: false
        reset_hover_on_click: true
        draw_bg.border_size: 1.0
        draw_bg.border_radius: 10.0
        draw_bg.color: #FFFFFF
        draw_bg.color_hover: #FDECEA
        draw_bg.color_down: #F8D7D4
        draw_bg.color_focus: #FFFFFF
        draw_bg.border_color: #E8B4B0
        draw_bg.border_color_hover: #E8B4B0
        draw_bg.border_color_down: #E8B4B0
        draw_bg.border_color_focus: #C62828
        draw_text.color: #C62828
        draw_text.color_hover: #C62828
        draw_text.color_down: #C62828
        draw_text.color_focus: #C62828
        draw_text.text_style.font_size: 13
    }
    // Full-width multi-word CTAs — wrap + center; never Chip/ellipsis.
    let StackPrimaryBtn = Go{
        width: Fill
        height: Fit
        flow: Right{wrap: true}
        align: Align{x: 0.5 y: 0.5}
        label_walk: Walk{width: Fit, height: Fit, margin: Inset{left: 4. right: 4. top: 0. bottom: 0.}}
        draw_text.max_lines: 4
        draw_text.text_overflow: Clip
    }
    let StackSecondaryBtn = Quiet{
        width: Fill
        height: Fit
        flow: Right{wrap: true}
        align: Align{x: 0.5 y: 0.5}
        label_walk: Walk{width: Fit, height: Fit, margin: Inset{left: 4. right: 4. top: 0. bottom: 0.}}
        draw_text.max_lines: 4
        draw_text.text_overflow: Clip
    }
    let ActionStack = View{
        width: Fill
        height: Fit
        flow: Down
        spacing: 10
    }
    // Navigate elsewhere (hub destination) — never filled primary.
    let HubDestBtn = Quiet{
        width: Fill
        height: Fit
        padding: Inset{left: 20. right: 20. top: 16. bottom: 16.}
        margin: Inset{left: 0. right: 0. top: 4. bottom: 4.}
        align: Align{x: 0 y: 0.5}
        label_walk: Walk{width: Fill, height: Fit, margin: Inset{left: 2. right: 2. top: 0. bottom: 0.}}
        draw_text.max_lines: 3
        draw_text.text_overflow: Ellipsis
        draw_text.text_style.font_size: 14
    }
    let Field = TextInput{
        width: Fill
        height: Fit
        padding: Inset{left: 16. right: 16. top: 14. bottom: 14.}
        margin: Inset{left: 0. right: 0. top: 0. bottom: 0.}
        draw_bg.border_size: 1.0
        draw_bg.border_radius: 22.0
        draw_bg.color: #FFFFFF
        draw_bg.color_hover: #FFFFFF
        draw_bg.color_focus: #FFFFFF
        draw_bg.color_down: #FFFFFF
        draw_bg.color_empty: #FFFFFF
        draw_bg.border_color: #C5D0DE
        draw_bg.border_color_hover: #8A9BB0
        draw_bg.border_color_focus: #0183FB
        draw_bg.border_color_down: #0183FB
        draw_bg.border_color_empty: #C5D0DE
        draw_text.color: #0B1220
        draw_text.color_hover: #0B1220
        draw_text.color_focus: #0B1220
        draw_text.color_down: #0B1220
        draw_text.color_empty: #7A8B9C
        draw_text.color_empty_hover: #7A8B9C
        draw_text.color_empty_focus: #7A8B9C
        draw_text.text_style.font_size: 15
        // Theme default caret is white — invisible on our white fields.
        draw_cursor.color: #0B1220
        // Selection is drawn *over* glyphs — keep alpha so ink stays readable.
        draw_selection.color: #0183FB40
        draw_selection.color_hover: #0183FB55
        draw_selection.color_focus: #0183FB55
        draw_selection.color_down: #0183FB66
        blink_speed: 0.5
    }
    // Square Font Awesome toolbar control (WorldFonts falls through to FA).
    // grab_key_focus false — tapping next to the composer must not steal IME
    // focus (Android otherwise dismisses the keyboard mid-tap and drops the click).
    let IconBtn = Quiet{
        width: 44
        height: 44
        padding: Inset{left: 0. right: 0. top: 0. bottom: 0.}
        margin: Inset{left: 0. right: 0. top: 0. bottom: 0.}
        align: Center
        grab_key_focus: false
        draw_bg.border_radius: 22.0
        draw_text.text_style.font_size: 18
        draw_text.text_style.font_family: WorldFonts
    }
    let IconGo = Go{
        width: 44
        height: 44
        padding: Inset{left: 0. right: 0. top: 0. bottom: 0.}
        margin: Inset{left: 0. right: 0. top: 0. bottom: 0.}
        align: Center
        grab_key_focus: false
        draw_bg.border_radius: 22.0
        draw_text.text_style.font_size: 18
        draw_text.text_style.font_family: WorldFonts
    }
    // Read-only selectable body text — Makepad Label is not selectable.
    // Looks like a Label; drag to select / copy. Override draw_text.color as needed.
    // empty_text must be "" — theme default is "Your text here".
    let Selectable = TextInput{
        width: Fill
        height: Fit
        is_read_only: true
        is_multiline: true
        empty_text: ""
        flow: Right{wrap: true}
        padding: Inset{left: 0. right: 0. top: 0. bottom: 0.}
        margin: Inset{left: 0. right: 0. top: 0. bottom: 0.}
        draw_bg.border_size: 0.0
        draw_bg.border_radius: 0.0
        draw_bg.color: #00000000
        draw_bg.color_hover: #00000000
        draw_bg.color_focus: #00000000
        draw_bg.color_down: #00000000
        draw_bg.color_empty: #00000000
        draw_bg.border_color: #00000000
        draw_bg.border_color_hover: #00000000
        draw_bg.border_color_focus: #00000000
        draw_bg.border_color_down: #00000000
        draw_bg.border_color_empty: #00000000
        draw_text.color: #0B1220
        draw_text.color_hover: #0B1220
        draw_text.color_focus: #0B1220
        draw_text.color_down: #0B1220
        draw_text.color_empty: #0B1220
        draw_text.color_empty_hover: #0B1220
        draw_text.color_empty_focus: #0B1220
        draw_text.text_style.font_size: 14
        draw_cursor.color: #00000000
        // Selection is drawn *over* glyphs — keep alpha so ink stays readable.
        draw_selection.color: #0183FB40
        draw_selection.color_hover: #0183FB55
        draw_selection.color_focus: #0183FB55
        draw_selection.color_down: #0183FB66
        blink_speed: 0.0
    }
    // Chat text bubble — Label in a rounded box. Deliberately NOT a TextInput:
    // a read-only TextInput takes key focus on the finger-down that starts a
    // scroll, which yanked focus (and the soft keyboard) off the composer and
    // left stray selections on bubbles. Use Bubble (button) for media rows.
    // Flutter: chip is content-sized (max ~280), not a full-width slab. Row
    // padding below provides the opposite-side gutter (out=left, in=right).
    // Labels only — never Selectable/TextInput (those steal IME focus from the composer).
    let TextBubble = RoundedView{
        width: Fit{max: FitBound.Abs(280)}
        height: Fit
        flow: Down
        spacing: 4
        padding: Inset{left: 12. right: 12. top: 8. bottom: 6.}
        margin: Inset{left: 0. right: 0. top: 2. bottom: 2.}
        draw_bg.border_size: 0.0
        draw_bg.border_radius: 12.0
        draw_bg.color: #FFFFFF
        body := Label{
            width: Fit{max: FitBound.Abs(256)}
            text: ""
            draw_text.color: #0B1220
            draw_text.text_style.font_size: 15
            draw_text.max_lines: 48
            draw_text.text_overflow: Ellipsis
        }
        meta := View{
            width: Fit
            height: Fit
            flow: Right
            spacing: 4
            align: Align{y: 0.5}
            caption := Label{
                width: Fit
                text: ""
                draw_text.color: #7A8B9C
                draw_text.text_style.font_size: 11
                draw_text.max_lines: 1
                draw_text.text_overflow: Ellipsis
            }
            // Ticks use FA PUA: check + check-double are single glyphs. NotoSans
            // has no ✓, so "✓✓" fell through to two wide FA checks with a gap.
            tick := Label{
                width: Fit
                text: ""
                draw_text.color: #7A8B9C
                draw_text.text_style.font_size: 12
                draw_text.text_style.font_family: WorldFonts
            }
        }
    }
    let Panel = SolidView{
        show_bg: true
        draw_bg.color: #FFFFFF
        width: Fill
        height: Fill
    }
    let Bar = SolidView{
        show_bg: true
        draw_bg.color: #EEF2F8
        width: Fill
        height: Fit
        flow: Right
        spacing: 10
        padding: Inset{left: 14. right: 14. top: 12. bottom: 12.}
        align: Align{y: 0.5}
    }
    // Side-rail peer — idle face. Selection is Overlay idle+active (Makepad Button
    // has no selected bit). Idle must NOT reuse active fill on hover/focus.
    let RailTab = Button{
        width: Fill
        height: Fit
        padding: Inset{left: 40. right: 12. top: 12. bottom: 12.}
        align: Align{x: 0.0 y: 0.5}
        flow: Right{wrap: true}
        grab_key_focus: false
        reset_hover_on_click: true
        label_walk: Walk{width: Fill, height: Fit, margin: Inset{left: 0. right: 4.}}
        label_align: Align{x: 0.0 y: 0.5}
        icon_walk: Walk{width: 0. height: 0.}
        draw_bg.border_size: 0.0
        draw_bg.border_radius: 12.0
        draw_bg.color: #00000000
        draw_bg.color_hover: #DCE4F0
        draw_bg.color_down: #C5D0DE
        draw_bg.color_focus: #00000000
        draw_bg.border_color: #00000000
        draw_text.color: #5A6B7D
        draw_text.color_hover: #0B1220
        draw_text.color_down: #0B1220
        draw_text.color_focus: #5A6B7D
        draw_text.max_lines: 2
        draw_text.text_overflow: Ellipsis
        draw_text.text_style.font_size: 13
    }
    let RailTabOn = Button{
        width: Fill
        height: Fit
        padding: Inset{left: 40. right: 12. top: 12. bottom: 12.}
        align: Align{x: 0.0 y: 0.5}
        flow: Right{wrap: true}
        grab_key_focus: false
        reset_hover_on_click: true
        label_walk: Walk{width: Fill, height: Fit, margin: Inset{left: 0. right: 4.}}
        label_align: Align{x: 0.0 y: 0.5}
        icon_walk: Walk{width: 0. height: 0.}
        draw_bg.border_size: 0.0
        draw_bg.border_radius: 12.0
        draw_bg.color: #10236D
        draw_bg.color_hover: #10236D
        draw_bg.color_down: #0C1C55
        draw_bg.color_focus: #10236D
        draw_bg.border_color: #10236D
        draw_text.color: #FFFFFF
        draw_text.color_hover: #FFFFFF
        draw_text.color_down: #FFFFFF
        draw_text.color_focus: #FFFFFF
        draw_text.max_lines: 2
        draw_text.text_overflow: Ellipsis
        draw_text.text_style.font_size: 13
    }
    // Phone bottom peers (<720) — scrollable chips; same idle/active Overlay rule.
    let NavChip = Button{
        width: Fit{min: FitBound.Abs(88.)}
        height: 56
        padding: Inset{left: 36. right: 12. top: 8. bottom: 8.}
        align: Align{x: 0.0 y: 0.5}
        grab_key_focus: false
        reset_hover_on_click: true
        label_walk: Walk{width: Fit, height: Fit, margin: Inset{left: 0. right: 2.}}
        icon_walk: Walk{width: 0. height: 0.}
        draw_bg.border_size: 0.0
        draw_bg.border_radius: 12.0
        draw_bg.color: #00000000
        draw_bg.color_hover: #DCE4F0
        draw_bg.color_down: #C5D0DE
        draw_bg.color_focus: #00000000
        draw_bg.border_color: #00000000
        draw_text.color: #5A6B7D
        draw_text.color_hover: #0B1220
        draw_text.color_down: #0B1220
        draw_text.color_focus: #5A6B7D
        draw_text.text_style.font_size: 12
    }
    let NavChipOn = Button{
        width: Fit{min: FitBound.Abs(88.)}
        height: 56
        padding: Inset{left: 36. right: 12. top: 8. bottom: 8.}
        align: Align{x: 0.0 y: 0.5}
        grab_key_focus: false
        reset_hover_on_click: true
        label_walk: Walk{width: Fit, height: Fit, margin: Inset{left: 0. right: 2.}}
        icon_walk: Walk{width: 0. height: 0.}
        draw_bg.border_size: 0.0
        draw_bg.border_radius: 12.0
        draw_bg.color: #10236D
        draw_bg.color_hover: #10236D
        draw_bg.color_down: #0C1C55
        draw_bg.color_focus: #10236D
        draw_bg.border_color: #10236D
        draw_text.color: #FFFFFF
        draw_text.color_hover: #FFFFFF
        draw_text.color_down: #FFFFFF
        draw_text.color_focus: #FFFFFF
        draw_text.text_style.font_size: 12
    }
    let NavMark = Image{
        width: 22
        height: 22
        margin: Inset{left: 10. right: 0. top: 0. bottom: 0.}
    }
    // Hub roster row (Flutter `chat_hub_screen` chrome): avatar initial, title,
    // availability, preview, right-side "Unknown" chip + unread count badge.
    // No cursor / Button children — `roster_list.on_item_tap` owns the press so a
    // vertical drag scrolls the list and a plain tap opens the room (native feel).
    let RosterRow = RoundedView{
        width: Fill
        height: Fit
        flow: Right
        spacing: 12
        padding: Inset{left: 12. right: 12. top: 10. bottom: 10.}
        align: Align{y: 0.5}
        draw_bg.border_radius: 10.0
        draw_bg.border_size: 0.0
        draw_bg.color: #FFFFFF
        avatar := RoundedView{
            width: 44
            height: 44
            align: Center
            draw_bg.border_radius: 22.0
            draw_bg.border_size: 0.0
            draw_bg.color: #D6E8FF
            initial := Label{
                text: "?"
                draw_text.color: #0183FB
                draw_text.text_style.font_size: 17
            }
        }
        body := View{
            width: Fill
            height: Fit
            flow: Down
            spacing: 2
            title := Label{
                width: Fill
                text: ""
                draw_text.color: #0B1220
                draw_text.text_style.font_size: 15
                draw_text.max_lines: 1
                draw_text.text_overflow: Ellipsis
            }
            status := Label{
                width: Fill
                text: ""
                draw_text.color: #0183FB
                draw_text.text_style.font_size: 12
                draw_text.max_lines: 1
                draw_text.text_overflow: Ellipsis
            }
            preview := Label{
                width: Fill
                text: ""
                draw_text.color: #7A8B9C
                draw_text.text_style.font_size: 13
                draw_text.max_lines: 1
                draw_text.text_overflow: Ellipsis
            }
        }
        side := View{
            width: Fit
            height: Fit
            flow: Down
            spacing: 6
            align: Align{x: 1.0}
            chip := RoundedView{
                width: Fit
                height: Fit
                padding: Inset{left: 8. right: 8. top: 3. bottom: 3.}
                draw_bg.border_radius: 8.0
                draw_bg.border_size: 1.0
                draw_bg.border_color: #F0C27B
                draw_bg.color: #FFF3DC
                Label{
                    text: "Unknown"
                    draw_text.color: #8A5A00
                    draw_text.text_style.font_size: 11
                }
            }
            badge := RoundedView{
                width: Fit
                height: Fit
                padding: Inset{left: 7. right: 7. top: 2. bottom: 2.}
                draw_bg.border_radius: 10.0
                draw_bg.border_size: 0.0
                draw_bg.color: #0183FB
                count := Label{
                    text: ""
                    draw_text.color: #FFFFFF
                    draw_text.text_style.font_size: 11
                }
            }
        }
    }
    let MenuRow = Quiet{
        width: Fill
        height: Fit
        padding: Inset{left: 20. right: 20. top: 16. bottom: 16.}
        margin: Inset{left: 0. right: 0. top: 4. bottom: 4.}
        align: Align{x: 0 y: 0.5}
        label_walk: Walk{width: Fill, height: Fit}
        draw_text.max_lines: 3
        draw_text.text_overflow: Ellipsis
        draw_text.text_style.font_size: 14
    }
    let Bubble = Quiet{
        width: Fit{max: FitBound.Abs(280)}
        height: Fit
        // Tapping a media row must not pull key focus (and the IME) off the composer.
        grab_key_focus: false
        padding: Inset{left: 16. right: 16. top: 12. bottom: 12.}
        margin: Inset{left: 0. right: 0. top: 2. bottom: 2.}
        align: Align{x: 0 y: 0.5}
        label_walk: Walk{width: Fit, height: Fit}
        draw_text.max_lines: 40
        draw_text.text_overflow: Ellipsis
        draw_bg.border_radius: 12.0
        draw_text.text_style.font_size: 14
    }
    // Vertical body scroller — sibling of fixed chrome (bars/composer), never wraps them.
    let VScroll = ScrollYView{
        width: Fill
        height: Fill
        flow: Down
        scroll_bars: ScrollBars{
            show_scroll_x: false
            show_scroll_y: true
            scroll_bar_y.drag_scrolling: true
        }
    }
    // Horizontal toolbar scroller. Strip must be clearly taller than the chips
    // so Android has empty band above/below to pan. See MAKEPAD_UI.md § Scrolling.
    let HScroll = ScrollXView{
        width: Fill
        height: 96
        flow: Right
        spacing: 10
        padding: Inset{left: 10. right: 10. top: 20. bottom: 20.}
        align: Align{y: 0.5}
        scroll_bars: ScrollBars{
            show_scroll_x: true
            show_scroll_y: false
            scroll_bar_x.drag_scrolling: true
            scroll_bar_x.use_vertical_finger_scroll: true
        }
    }
    // Hub action strip — tall finger band; title above stays compact (no HScroll).
    let ActionBar = ScrollXView{
        show_bg: true
        draw_bg.color: #EEF2F8
        width: Fill
        height: 96
        flow: Right
        spacing: 10
        padding: Inset{left: 12. right: 12. top: 20. bottom: 20.}
        align: Align{y: 0.5}
        scroll_bars: ScrollBars{
            show_scroll_x: true
            show_scroll_y: false
            scroll_bar_x.drag_scrolling: true
            scroll_bar_x.use_vertical_finger_scroll: true
        }
    }
    // Compact toolbar chip — shorter than ActionBar/HScroll so gutters remain
    // tappable for horizontal pan (full Quiet padding fills the strip).
    // Named BarChip (not Chip) so it never clashes with RosterRow.side.chip
    // (the Unknown badge) in Splash property paths.
    let BarChip = Quiet{
        height: Fit
        padding: Inset{left: 12. right: 12. top: 6. bottom: 6.}
        draw_text.text_style.font_size: 12
        grab_key_focus: false
    }

    startup() do #(App::script_component(vm)){
        ui: Root{
            on_startup: ||{
                ui.identity_copy.render()
            }
            main_window := Window{
                window.inner_size: vec2(1180, 760)
                window.title: "Ghal Bol"
                // Match Android launch chrome — avoid a long black GL clear before first paint.
                pass +: {clear_color: #EEF2F8}
                // Makepad dark theme draws caption glyphs with color_label_inner (near-white)
                // on a transparent caption bar. Our clear is #EEF2F8, so min/max/close vanish
                // under Wayland custom chrome. Force light bar + dark glyphs.
                caption_bar +: {
                    draw_bg.color: #EEF2F8
                    caption_label +: {
                        label +: {draw_text +: {color: #0B1220}}
                    }
                    windows_buttons +: {
                        min +: {
                            draw_bg +: {
                                color: #0B1220
                                color_hover: #0B1220
                                color_down: #0B1220
                                bg_color_hover: #DCE4F0
                                bg_color_down: #C5D0DE
                            }
                        }
                        max +: {
                            draw_bg +: {
                                color: #0B1220
                                color_hover: #0B1220
                                color_down: #0B1220
                                bg_color_hover: #DCE4F0
                                bg_color_down: #C5D0DE
                            }
                        }
                        close +: {
                            draw_bg +: {
                                color: #0B1220
                                color_hover: #FFFFFF
                                color_down: #FFFFFF
                                bg_color_hover: #E81123
                                bg_color_down: #F1707A
                            }
                        }
                    }
                }
                // Own the IME inset here. Android uses adjustNothing so the OS
                // does not also shrink the surface — that double-count hid the
                // transcript and made the keyboard dance on scroll.
                body +: {
                    keyboard_resize: true
                    identity_view := Panel{
                        draw_bg.color: #EEF2F8
                        width: Fill
                        height: Fill
                        flow: Down
                        spacing: 0
                        padding: Inset{
                            top: (mod.widgets.SAFE_INSET_PAD_TOP)
                            bottom: (mod.widgets.SAFE_INSET_PAD_BOTTOM)
                            left: (mod.widgets.SAFE_INSET_PAD_LEFT)
                            right: (mod.widgets.SAFE_INSET_PAD_RIGHT)
                        }
                        unlock_scroll := VScroll{
                            spacing: 16
                            padding: Inset{left: 20. right: 20. top: 28. bottom: 28.}
                            align: Center
                            // One column: title + form share width so unlock chrome stays centered.
                            // apply_shell_layout clamps identity_column to ≤360.
                            identity_column := View{
                                width: Fill
                                height: Fit
                                flow: Down
                                spacing: 16
                                align: Center
                            identity_copy := View{
                                width: Fill
                                height: Fit
                                flow: Down
                                spacing: 10
                                align: Center
                                on_render: ||{
                                    // Fit + parent align:Center so the brand sits on the same axis as the form.
                                    Selectable{
                                        width: Fit
                                        text: "Ghal Bol"
                                        draw_text.color: #0B1220
                                        draw_text.text_style.font_size: 30
                                    }
                                    // Fill width + label_align:Center — layout align alone leaves wrapped ink left.
                                    hint_label := Selectable{
                                        width: Fill
                                        height: Fit
                                        label_align: Center
                                        text: state.hint
                                        draw_text.color: #5A6B7D
                                        draw_text.max_lines: 6
                                    }
                                    if state.status != ""
                                        status_label := Selectable{
                                            width: Fill
                                            height: Fit
                                            label_align: Center
                                            text: state.status
                                            draw_text.color: #7A8B9C
                                            draw_text.max_lines: 4
                                        }
                                    if state.algo_line != ""
                                        algo_label := Selectable{
                                            width: Fit
                                            height: Fit
                                            text: state.algo_line
                                            draw_text.color: #0183FB
                                        }
                                }
                            }
                            identity_form := View{
                                width: Fill
                                height: Fit
                                flow: Down
                                spacing: 14
                                align: Center
                            password_row := View{
                                width: Fill
                                height: Fit
                                flow: Right
                                spacing: 8
                                align: Align{y: 0.5}
                                password_input := Field{
                                    empty_text: "Password"
                                    is_password: true
                                    width: Fill
                                    height: Fit
                                }
                                password_eye := Quiet{ text: "Show" }
                            }
                            unlock_button := Go{ text: "Continue" width: Fill }
                            mode_button := Quiet{ text: "Import an existing key" width: Fill }
                            secret_row := View{
                                width: Fill
                                height: Fit
                                flow: Right
                                spacing: 8
                                align: Align{y: 0.5}
                                secret_input := Field{
                                    empty_text: "Private key hex"
                                    is_password: true
                                    width: Fill
                                    height: Fit
                                }
                                secret_eye := Quiet{ text: "Show" }
                            }
                            // Full-width HScroll — never stack these as Fill-width Down rows
                            // (that removes horizontal pan on Android).
                            algo_row := HScroll{
                                width: Fill
                                spacing: 8
                                algo_secp := Quiet{
                                    text: "secp256k1  [on]"
                                    width: Fit
                                    draw_bg.color: #D6E8FF
                                    draw_bg.color_hover: #BFD9FF
                                    draw_bg.color_down: #A8CBFF
                                    draw_bg.color_focus: #D6E8FF
                                    on_click: ||{
                                        mod.state.algo = "secp256k1"
                                        mod.state.algo_line = "Algorithm: secp256k1"
                                        mod.state.algo_gen = mod.state.algo_gen + 1
                                        ui.algo_secp.set_text("secp256k1  [on]")
                                        ui.algo_ed.set_text("ed25519")
                                        ui.algo_p256.set_text("ecdsa-p256")
                                        ui.algo_secp.draw_bg.color = #D6E8FF
                                        ui.algo_secp.draw_bg.color_hover = #BFD9FF
                                        ui.algo_secp.draw_bg.color_down = #A8CBFF
                                        ui.algo_secp.draw_bg.color_focus = #D6E8FF
                                        ui.algo_ed.draw_bg.color = #FFFFFF
                                        ui.algo_ed.draw_bg.color_hover = #DCE4F0
                                        ui.algo_ed.draw_bg.color_down = #C5D0DE
                                        ui.algo_ed.draw_bg.color_focus = #FFFFFF
                                        ui.algo_p256.draw_bg.color = #FFFFFF
                                        ui.algo_p256.draw_bg.color_hover = #DCE4F0
                                        ui.algo_p256.draw_bg.color_down = #C5D0DE
                                        ui.algo_p256.draw_bg.color_focus = #FFFFFF
                                        ui.identity_copy.render()
                                    }
                                }
                                algo_ed := Quiet{
                                    text: "ed25519"
                                    width: Fit
                                    on_click: ||{
                                        mod.state.algo = "ed25519"
                                        mod.state.algo_line = "Algorithm: ed25519"
                                        mod.state.algo_gen = mod.state.algo_gen + 1
                                        ui.algo_secp.set_text("secp256k1")
                                        ui.algo_ed.set_text("ed25519  [on]")
                                        ui.algo_p256.set_text("ecdsa-p256")
                                        ui.algo_ed.draw_bg.color = #D6E8FF
                                        ui.algo_ed.draw_bg.color_hover = #BFD9FF
                                        ui.algo_ed.draw_bg.color_down = #A8CBFF
                                        ui.algo_ed.draw_bg.color_focus = #D6E8FF
                                        ui.algo_secp.draw_bg.color = #FFFFFF
                                        ui.algo_secp.draw_bg.color_hover = #DCE4F0
                                        ui.algo_secp.draw_bg.color_down = #C5D0DE
                                        ui.algo_secp.draw_bg.color_focus = #FFFFFF
                                        ui.algo_p256.draw_bg.color = #FFFFFF
                                        ui.algo_p256.draw_bg.color_hover = #DCE4F0
                                        ui.algo_p256.draw_bg.color_down = #C5D0DE
                                        ui.algo_p256.draw_bg.color_focus = #FFFFFF
                                        ui.identity_copy.render()
                                    }
                                }
                                algo_p256 := Quiet{
                                    text: "ecdsa-p256"
                                    width: Fit
                                    on_click: ||{
                                        mod.state.algo = "ecdsa-p256"
                                        mod.state.algo_line = "Algorithm: ecdsa-p256"
                                        mod.state.algo_gen = mod.state.algo_gen + 1
                                        ui.algo_secp.set_text("secp256k1")
                                        ui.algo_ed.set_text("ed25519")
                                        ui.algo_p256.set_text("ecdsa-p256  [on]")
                                        ui.algo_p256.draw_bg.color = #D6E8FF
                                        ui.algo_p256.draw_bg.color_hover = #BFD9FF
                                        ui.algo_p256.draw_bg.color_down = #A8CBFF
                                        ui.algo_p256.draw_bg.color_focus = #D6E8FF
                                        ui.algo_secp.draw_bg.color = #FFFFFF
                                        ui.algo_secp.draw_bg.color_hover = #DCE4F0
                                        ui.algo_secp.draw_bg.color_down = #C5D0DE
                                        ui.algo_secp.draw_bg.color_focus = #FFFFFF
                                        ui.algo_ed.draw_bg.color = #FFFFFF
                                        ui.algo_ed.draw_bg.color_hover = #DCE4F0
                                        ui.algo_ed.draw_bg.color_down = #C5D0DE
                                        ui.algo_ed.draw_bg.color_focus = #FFFFFF
                                        ui.identity_copy.render()
                                    }
                                }
                            }
                            delete_button := Stop{ text: "Delete identity on this device" width: Fill }
                            import_backup_button := Quiet{ text: "Import keystore backup" width: Fill }
                            }
                            }
                        }
                    }
                    hub_view := Panel{
                        visible: false
                        draw_bg.color: #E4EAF3
                        flow: Down
                        padding: Inset{
                            top: (mod.widgets.SAFE_INSET_PAD_TOP)
                            bottom: (mod.widgets.SAFE_INSET_PAD_BOTTOM)
                            left: (mod.widgets.SAFE_INSET_PAD_LEFT)
                            right: (mod.widgets.SAFE_INSET_PAD_RIGHT)
                        }
                        hub_body := View{
                            width: Fill
                            height: Fill
                            flow: Right
                        nav_rail := SolidView{
                            show_bg: true
                            draw_bg.color: #EEF2F8
                            width: 176
                            height: Fill
                            flow: Down
                            spacing: 8
                            padding: Inset{top: 16. bottom: 14. left: 12. right: 12.}
                            Selectable{
                                text: "Ghal Bol"
                                draw_text.color: #10236D
                                draw_text.text_style.font_size: 14
                                width: Fill
                            }
                            // Peers are roots (go_peer). Idle+active Overlay — paint from hub_tab.
                            rail_chats_slot := View{
                                width: Fill height: Fit flow: Overlay
                                tab_chats := RailTab{ text: "Chats" }
                                tab_chats_on := RailTabOn{ text: "Chats" visible: false }
                                mark_chats := NavMark{ abs_pos: vec2(10, 13) }
                            }
                            rail_contacts_slot := View{
                                width: Fill height: Fit flow: Overlay
                                tab_contacts := RailTab{ text: "Contacts" }
                                tab_contacts_on := RailTabOn{ text: "Contacts" visible: false }
                                mark_contacts := NavMark{ abs_pos: vec2(10, 13) }
                            }
                            rail_invite_slot := View{
                                width: Fill height: Fit flow: Overlay
                                tab_invite := RailTab{ text: "Invite" }
                                tab_invite_on := RailTabOn{ text: "Invite" visible: false }
                                mark_invite := NavMark{ abs_pos: vec2(10, 13) }
                            }
                            rail_settings_slot := View{
                                width: Fill height: Fit flow: Overlay
                                tab_settings := RailTab{ text: "Settings" }
                                tab_settings_on := RailTabOn{ text: "Settings" visible: false }
                                mark_settings := NavMark{ abs_pos: vec2(10, 13) }
                            }
                            Filler{}
                            rail_lock_slot := View{
                                width: Fill height: Fit flow: Overlay
                                lock_button := RailTab{ text: "Lock" }
                                mark_lock := NavMark{ abs_pos: vec2(10, 13) }
                            }
                        }
                        rail_divider := SolidView{
                            show_bg: true
                            draw_bg.color: #C5D0DE
                            width: 1
                            height: Fill
                        }
                        chats_pane := View{
                            width: Fill
                            height: Fill
                            flow: Right
                            roster_col := SolidView{
                                show_bg: true
                                draw_bg.color: #FFFFFF
                                width: 400
                                height: Fill
                                flow: Down
                                spacing: 0
                                // Stack title ABOVE a full-width ActionBar. Putting title +
                                // actions in one `flow: Right` row on Android gives the
                                // scroller a leftover width and kills horizontal pan — never
                                // again nest HScroll beside the title on narrow chrome.
                                roster_bar := SolidView{
                                    show_bg: true
                                    draw_bg.color: #EEF2F8
                                    width: Fill
                                    height: Fit
                                    flow: Down
                                    spacing: 0
                                    // Title is static — keep it short; vertical space belongs
                                    // to ActionBar (horizontal pan finger band), not the label.
                                    View{
                                        width: Fill
                                        height: Fit
                                        padding: Inset{left: 12. right: 12. top: 4. bottom: 0.}
                                        Label{
                                            text: "Ghal Bol"
                                            draw_text.color: #0B1220
                                            draw_text.text_style.font_size: 16
                                            width: Fill
                                            draw_text.max_lines: 1
                                            draw_text.text_overflow: Ellipsis
                                        }
                                    }
                                    roster_actions := ActionBar{
                                        width: Fill
                                        new_chat_button := BarChip{ text: "New chat" }
                                        invite_button := BarChip{ text: "Invite" }
                                        join_button := BarChip{ text: "Join" }
                                        scan_button := BarChip{ text: "Scan" }
                                        delivery_button := BarChip{ text: "Delivery" }
                                        hub_lock_button := BarChip{ text: "Lock" }
                                    }
                                }
                                SolidView{
                                    show_bg: true
                                    draw_bg.color: #FFFFFF
                                    width: Fill
                                    height: Fit
                                    padding: Inset{left: 16. right: 16. top: 12. bottom: 12.}
                                    search_input := Field{
                                        empty_text: "Search contacts…"
                                        width: Fill
                                    }
                                }
                                roster_list := VScroll{
                                    spacing: 6
                                    padding: Inset{left: 8. right: 8. top: 8. bottom: 12.}
                                    // Container owns the press (rows have no cursor/Button): a
                                    // drag scrolls, a tap opens. Index matches state.roster.
                                    on_item_tap: |index|{
                                        if index >= state.roster.len() { return }
                                        let row = state.roster[index]
                                        mod.state.selected_pk = row.pk
                                        mod.state.selected_title = row.title
                                        mod.state.open_gen = mod.state.open_gen + 1
                                        ui.chat_log.render()
                                    }
                                    on_render: ||{
                                        if state.roster.len() == 0
                                            View{
                                                width: Fill
                                                height: Fit
                                                flow: Down
                                                spacing: 12
                                                padding: 28
                                                align: Center
                                                Label{
                                                    text: state.roster_empty_title
                                                    draw_text.color: #0B1220
                                                    draw_text.text_style.font_size: 18
                                                }
                                                Label{
                                                    width: Fill
                                                    text: state.roster_empty_body
                                                    draw_text.color: #7A8B9C
                                                }
                                            }
                                        else for i, row in state.roster {
                                            // Open room = green wash, unread = blue wash with a
                                            // count badge (Flutter roster chrome).
                                            if row.pk == state.selected_pk
                                                RosterRow{
                                                    draw_bg.color: #D6E8FF
                                                    avatar.draw_bg.color: #A8CBFF
                                                    avatar.initial.text: row.initial
                                                    body.title.text: row.title
                                                    body.status.text: row.status
                                                    body.status.visible: row.status != ""
                                                    body.preview.text: row.preview
                                                    side.chip.visible: row.unknown == "1"
                                                    side.badge.visible: row.unread != "0"
                                                    side.badge.count.text: row.unread
                                                }
                                            else if row.unread != "0"
                                                RosterRow{
                                                    draw_bg.color: #E7F3FF
                                                    avatar.draw_bg.color: #C5E1F7
                                                    avatar.initial.draw_text.color: #0B5CAB
                                                    avatar.initial.text: row.initial
                                                    body.title.text: row.title
                                                    body.title.draw_text.color: #0B5CAB
                                                    body.status.text: row.status
                                                    body.status.visible: row.status != ""
                                                    body.preview.text: row.preview
                                                    body.preview.draw_text.color: #0B1220
                                                    side.chip.visible: row.unknown == "1"
                                                    side.badge.visible: true
                                                    side.badge.count.text: row.unread
                                                }
                                            else
                                                RosterRow{
                                                    avatar.initial.text: row.initial
                                                    body.title.text: row.title
                                                    body.status.text: row.status
                                                    body.status.visible: row.status != ""
                                                    body.preview.text: row.preview
                                                    side.chip.visible: row.unknown == "1"
                                                    side.badge.visible: false
                                                }
                                        }
                                    }
                                }
                            }
                            chat_col := SolidView{
                                show_bg: true
                                draw_bg.color: #ECE5DD
                                width: Fill
                                height: Fill
                                flow: Down
                                spacing: 0
                                // Title row + horizontally scrollable action strip (icons).
                                chat_bar := SolidView{
                                    show_bg: true
                                    draw_bg.color: #EEF2F8
                                    width: Fill
                                    height: Fit
                                    flow: Down
                                    spacing: 0
                                    View{
                                        width: Fill
                                        height: Fit
                                        flow: Right
                                        spacing: 10
                                        padding: Inset{left: 10. right: 10. top: 8. bottom: 4.}
                                        align: Align{y: 0.5}
                                        chat_back_button := IconBtn{ text: "\u{f060}" visible: false }
                                        chat_title_label := Label{
                                            text: state.selected_title
                                            draw_text.color: #0B1220
                                            draw_text.text_style.font_size: 17
                                            width: Fill
                                            draw_text.max_lines: 1
                                            draw_text.text_overflow: Ellipsis
                                        }
                                    }
                                    chat_actions := HScroll{
                                        spacing: 6
                                        call_button := IconBtn{ text: "\u{f095}" }
                                        video_button := IconBtn{ text: "\u{f03d}" }
                                        chat_menu_button := IconBtn{ text: "\u{f142}" }
                                    }
                                    // Contact-trust banner (DESIGN.md § Contact trust): shown only
                                    // while the open peer is `is_known: false`. Add / Block here;
                                    // ack policy is untouched — this is chrome only.
                                    trust_banner := SolidView{
                                        visible: false
                                        show_bg: true
                                        draw_bg.color: #E8F4FD
                                        width: Fill
                                        height: Fit
                                        flow: Right
                                        spacing: 8
                                        padding: Inset{left: 14. right: 10. top: 8. bottom: 8.}
                                        align: Align{y: 0.5}
                                        Label{
                                            width: Fill
                                            text: "You have not added this person yet."
                                            draw_text.color: #0B1220
                                            draw_text.text_style.font_size: 13
                                            draw_text.max_lines: 2
                                            draw_text.text_overflow: Ellipsis
                                        }
                                        trust_add_button := Quiet{
                                            text: "Add"
                                            padding: Inset{left: 14. right: 14. top: 8. bottom: 8.}
                                            draw_text.color: #0183FB
                                            draw_text.color_hover: #0183FB
                                            draw_text.color_down: #0183FB
                                            draw_text.color_focus: #0183FB
                                        }
                                        trust_block_button := Quiet{
                                            text: "Block"
                                            padding: Inset{left: 14. right: 14. top: 8. bottom: 8.}
                                            draw_text.color: #C62828
                                            draw_text.color_hover: #C62828
                                            draw_text.color_down: #C62828
                                            draw_text.color_focus: #C62828
                                        }
                                    }
                                    // Flutter `_hubChatRoomPopupMenu` — toggled by ⋮.
                                    chat_menu := SolidView{
                                        visible: false
                                        show_bg: true
                                        draw_bg.color: #FFFFFF
                                        width: Fill
                                        height: Fit
                                        flow: Down
                                        spacing: 0
                                        padding: Inset{left: 8. right: 8. top: 4. bottom: 8.}
                                        menu_call_button := MenuRow{ text: "Voice call" }
                                        menu_video_button := MenuRow{ text: "Video call" }
                                        menu_share_button := MenuRow{ text: "Share invitation" }
                                        menu_join_button := MenuRow{ text: "Join someone" }
                                        menu_add_button := MenuRow{ text: "Add contact" }
                                        menu_block_button := MenuRow{ text: "Block" }
                                        menu_lock_button := MenuRow{ text: "Lock" }
                                    }
                                }
                                chat_log := VScroll{
                                    spacing: 10
                                    padding: 16
                                    on_render: ||{
                                        // Labels, not Selectable: nothing in the scrolling log may
                                        // take key focus from the composer.
                                        call_label := Label{
                                            width: Fill
                                            text: state.call_line
                                            draw_text.color: #7A8B9C
                                        }
                                        if state.lines.len() == 0
                                            Label{
                                                width: Fill
                                                text: state.transcript
                                                draw_text.color: #5A6B7D
                                            }
                                        else for i, line in state.lines {
                                            // Outgoing (right) / inbound (left). Opposite-side
                                            // gutter (~48) so chips are not full-bleed; ticks +
                                            // caption live inside the bubble meta row (Flutter).
                                            if line.side == "out"
                                                View{
                                                    width: Fill
                                                    height: Fit
                                                    flow: Right
                                                    align: Align{x: 1.0}
                                                    padding: Inset{left: 48. right: 8. top: 0. bottom: 0.}
                                                    if line.action != ""
                                                        View{
                                                            width: Fit{max: FitBound.Abs(280)}
                                                            height: Fit
                                                            flow: Down
                                                            spacing: 2
                                                            Bubble{
                                                                width: Fit{max: FitBound.Abs(280)}
                                                                draw_bg.color: #D6E8FF
                                                                draw_bg.color_hover: #BFD9FF
                                                                draw_bg.color_down: #A8CBFF
                                                                draw_bg.border_color: #D6E8FF
                                                                draw_text.color: #0B1220
                                                                text: line.body
                                                                on_click: ||{
                                                                    mod.state.play_path = line.path
                                                                    mod.state.play_action = line.action
                                                                    mod.state.play_gen = mod.state.play_gen + 1
                                                                }
                                                            }
                                                            View{
                                                                width: Fill
                                                                height: Fit
                                                                flow: Right
                                                                spacing: 4
                                                                align: Align{x: 1.0 y: 0.5}
                                                                Label{
                                                                    text: line.caption
                                                                    draw_text.color: #7A8B9C
                                                                    draw_text.text_style.font_size: 11
                                                                }
                                                                if line.read == "1"
                                                                    Label{
                                                                        text: line.tick
                                                                        draw_text.color: #34B7F1
                                                                        draw_text.text_style.font_size: 12
                                                                    }
                                                                else if line.read == "failed"
                                                                    Label{
                                                                        text: line.tick
                                                                        draw_text.color: #D84315
                                                                        draw_text.text_style.font_size: 12
                                                                    }
                                                                else
                                                                    Label{
                                                                        text: line.tick
                                                                        draw_text.color: #7A8B9C
                                                                        draw_text.text_style.font_size: 12
                                                                    }
                                                            }
                                                        }
                                                    else if line.read == "1"
                                                        TextBubble{
                                                            draw_bg.color: #D6E8FF
                                                            body.text: line.body
                                                            meta.caption.text: line.caption
                                                            meta.tick.text: line.tick
                                                            meta.tick.draw_text.color: #34B7F1
                                                        }
                                                    else if line.read == "failed"
                                                        TextBubble{
                                                            draw_bg.color: #D6E8FF
                                                            body.text: line.body
                                                            meta.caption.text: line.caption
                                                            meta.tick.text: line.tick
                                                            meta.tick.draw_text.color: #D84315
                                                        }
                                                    else
                                                        TextBubble{
                                                            draw_bg.color: #D6E8FF
                                                            body.text: line.body
                                                            meta.caption.text: line.caption
                                                            meta.tick.text: line.tick
                                                        }
                                                }
                                            else
                                                View{
                                                    width: Fill
                                                    height: Fit
                                                    flow: Right
                                                    align: Align{x: 0.0}
                                                    padding: Inset{left: 8. right: 48. top: 0. bottom: 0.}
                                                    if line.action == "fetch"
                                                        View{
                                                            width: Fit{max: FitBound.Abs(280)}
                                                            height: Fit
                                                            flow: Down
                                                            spacing: 4
                                                            RoundedView{
                                                                width: Fit{max: FitBound.Abs(280)}
                                                                height: Fit
                                                                flow: Down
                                                                spacing: 6
                                                                padding: Inset{left: 12. right: 12. top: 10. bottom: 8.}
                                                                draw_bg.border_radius: 12.0
                                                                draw_bg.border_size: 0.0
                                                                draw_bg.color: #FFFFFF
                                                                Label{
                                                                    width: Fit{max: FitBound.Abs(256)}
                                                                    text: line.body
                                                                    draw_text.color: #0B1220
                                                                    draw_text.text_style.font_size: 14
                                                                    draw_text.max_lines: 2
                                                                    draw_text.text_overflow: Ellipsis
                                                                }
                                                                Label{
                                                                    text: line.detail
                                                                    draw_text.color: #7A8B9C
                                                                    draw_text.text_style.font_size: 12
                                                                }
                                                                View{
                                                                    width: Fill
                                                                    height: Fit
                                                                    flow: Right
                                                                    spacing: 4
                                                                    align: Align{y: 0.5}
                                                                    Bubble{
                                                                        width: Fit
                                                                        text: "Download"
                                                                        padding: Inset{left: 12. right: 12. top: 8. bottom: 8.}
                                                                        on_click: ||{
                                                                            mod.state.play_path = line.path
                                                                            mod.state.play_action = "fetch"
                                                                            mod.state.play_gen = mod.state.play_gen + 1
                                                                        }
                                                                    }
                                                                    Filler{}
                                                                    Label{
                                                                        text: line.caption
                                                                        draw_text.color: #7A8B9C
                                                                        draw_text.text_style.font_size: 11
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    else if line.action == "file"
                                                        View{
                                                            width: Fit{max: FitBound.Abs(280)}
                                                            height: Fit
                                                            flow: Down
                                                            spacing: 4
                                                            RoundedView{
                                                                width: Fit{max: FitBound.Abs(280)}
                                                                height: Fit
                                                                flow: Down
                                                                spacing: 6
                                                                padding: Inset{left: 12. right: 12. top: 10. bottom: 8.}
                                                                draw_bg.border_radius: 12.0
                                                                draw_bg.border_size: 0.0
                                                                draw_bg.color: #FFFFFF
                                                                Label{
                                                                    width: Fit{max: FitBound.Abs(256)}
                                                                    text: line.body
                                                                    draw_text.color: #0B1220
                                                                    draw_text.text_style.font_size: 14
                                                                    draw_text.max_lines: 2
                                                                    draw_text.text_overflow: Ellipsis
                                                                }
                                                                Label{
                                                                    text: line.detail
                                                                    draw_text.color: #7A8B9C
                                                                    draw_text.text_style.font_size: 12
                                                                }
                                                                View{
                                                                    width: Fill
                                                                    height: Fit
                                                                    flow: Right
                                                                    spacing: 4
                                                                    Bubble{
                                                                        width: Fit
                                                                        text: "Open"
                                                                        padding: Inset{left: 12. right: 12. top: 8. bottom: 8.}
                                                                        on_click: ||{
                                                                            mod.state.play_path = line.path
                                                                            mod.state.play_action = "file"
                                                                            mod.state.play_gen = mod.state.play_gen + 1
                                                                        }
                                                                    }
                                                                    Bubble{
                                                                        width: Fit
                                                                        text: "Save"
                                                                        padding: Inset{left: 12. right: 12. top: 8. bottom: 8.}
                                                                        on_click: ||{
                                                                            mod.state.play_path = line.path
                                                                            mod.state.play_action = "save"
                                                                            mod.state.play_gen = mod.state.play_gen + 1
                                                                        }
                                                                    }
                                                                    Filler{}
                                                                    Label{
                                                                        text: line.caption
                                                                        draw_text.color: #7A8B9C
                                                                        draw_text.text_style.font_size: 11
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    else if line.action != ""
                                                        View{
                                                            width: Fit{max: FitBound.Abs(280)}
                                                            height: Fit
                                                            flow: Down
                                                            spacing: 2
                                                            Bubble{
                                                                width: Fit{max: FitBound.Abs(280)}
                                                                draw_bg.color: #FFFFFF
                                                                draw_bg.color_hover: #E4EAF3
                                                                draw_bg.color_down: #DCE4F0
                                                                draw_bg.border_color: #FFFFFF
                                                                draw_text.color: #0B1220
                                                                text: line.body
                                                                on_click: ||{
                                                                    mod.state.play_path = line.path
                                                                    mod.state.play_action = line.action
                                                                    mod.state.play_gen = mod.state.play_gen + 1
                                                                }
                                                            }
                                                            Label{
                                                                text: line.caption
                                                                draw_text.color: #7A8B9C
                                                                draw_text.text_style.font_size: 11
                                                            }
                                                        }
                                                    else
                                                        TextBubble{
                                                            body.text: line.body
                                                            meta.caption.text: line.caption
                                                            meta.tick.text: ""
                                                        }
                                                }
                                        }
                                    }
                                }
                                // Messenger-dense chrome (Telegram/WhatsApp): ~40px icons, tight
                                // gutters, field max ~4 lines then scrolls — not a growing slab.
                                composer_bar := SolidView{
                                    show_bg: true
                                    draw_bg.color: #E4EAF3
                                    width: Fill
                                    height: Fit
                                    flow: Down
                                    spacing: 4
                                    padding: Inset{left: 4. right: 4. top: 4. bottom: 4.}
                                    older_button := BarChip{ text: "Older messages" visible: false }
                                    // Flutter `_voiceRecordingBar`: Discard · red · timer · Send.
                                    // Shown while `host::voice_note_recording()`; hides composer_row.
                                    recording_bar := View{
                                        visible: false
                                        width: Fill
                                        height: Fit
                                        flow: Right
                                        spacing: 4
                                        align: Align{y: 0.5}
                                        record_discard_button := IconBtn{
                                            width: 40
                                            height: 40
                                            text: "\u{f1f8}"
                                            draw_text.text_style.font_size: 16
                                            draw_text.color: #D32F2F
                                            draw_text.color_hover: #D32F2F
                                            draw_text.color_down: #B71C1C
                                            draw_text.color_focus: #D32F2F
                                        }
                                        RoundedView{
                                            width: Fill
                                            height: 40
                                            flow: Right
                                            spacing: 8
                                            padding: Inset{left: 10. right: 10. top: 6. bottom: 6.}
                                            align: Align{y: 0.5}
                                            draw_bg.border_radius: 20.0
                                            draw_bg.border_size: 1.0
                                            draw_bg.border_color: #C5D0DE
                                            draw_bg.color: #FFFFFF
                                            RoundedView{
                                                width: 8
                                                height: 8
                                                draw_bg.border_radius: 4.0
                                                draw_bg.border_size: 0.0
                                                draw_bg.color: #FF5252
                                            }
                                            record_timer_label := Label{
                                                text: "0:00"
                                                draw_text.color: #0B1220
                                                draw_text.text_style.font_size: 14
                                            }
                                        }
                                        record_send_button := IconGo{
                                            width: 40
                                            height: 40
                                            text: "\u{f1d8}"
                                            draw_text.text_style.font_size: 16
                                        }
                                    }
                                    // Icon | Fill field | Send — never HScroll long labels (that
                                    // squeezed the field into a vertical "M/e/s/s/a/g/e" pill).
                                    composer_row := View{
                                        width: Fill
                                        height: Fit
                                        flow: Right
                                        spacing: 4
                                        // Bottom-align icons when the field grows with wrapped lines.
                                        align: Align{y: 1.0}
                                        record_button := IconBtn{
                                            width: 40
                                            height: 40
                                            text: "\u{f130}"
                                            draw_text.text_style.font_size: 16
                                        }
                                        choose_file_button := IconBtn{
                                            width: 40
                                            height: 40
                                            text: "\u{f0c6}"
                                            draw_text.text_style.font_size: 16
                                        }
                                        // Multiline like Flutter — wrap needs Right{wrap:true}.
                                        // Desktop: Enter sends, Shift+Enter newline (submit_on_enter).
                                        // Android soft keyboard: never return_key_type:Send — that
                                        // replaces Enter with IME Send and kills newlines. Soft Enter
                                        // inserts newline; paper-plane IconGo sends.
                                        // Cap height so long drafts scroll inside the field.
                                        composer := Field{
                                            empty_text: "Message"
                                            width: Fill
                                            height: Fit{max: FitBound.Abs(100)}
                                            padding: Inset{left: 12. right: 12. top: 8. bottom: 8.}
                                            draw_bg.border_radius: 18.0
                                            is_multiline: true
                                            submit_on_enter: true
                                            flow: Right{wrap: true}
                                        }
                                        // Press (not click-up): IME reflow must not move the
                                        // hit target before FingerUp. grab_key_focus is off on IconGo.
                                        send_button := IconGo{
                                            width: 40
                                            height: 40
                                            text: "\u{f1d8}"
                                            draw_text.text_style.font_size: 16
                                        }
                                    }
                                }
                            }
                        }
                        contacts_pane := Panel{
                            visible: false
                            draw_bg.color: #FFFFFF
                            width: Fill
                            height: Fill
                            flow: Down
                            spacing: 0
                            contacts_scroll := VScroll{
                                spacing: 10
                                padding: 32
                                Selectable{
                                    text: "Contacts"
                                    draw_text.color: #0B1220
                                    draw_text.text_style.font_size: 24
                                }
                                Selectable{
                                    width: Fill
                                    text: "People you chat with. Add by public key, rename, or remove. Blocking is local to this device."
                                    draw_text.color: #5A6B7D
                                }
                                contacts_button := HubDestBtn{ text: "Contacts — add, rename, or remove by public key" }
                                add_contact_button := HubDestBtn{ text: "Add contact by public key" }
                                blocked_button := HubDestBtn{ text: "Blocked contacts — people you blocked on this device" }
                            }
                        }
                        invite_pane := Panel{
                            visible: false
                            draw_bg.color: #FFFFFF
                            width: Fill
                            height: Fill
                            flow: Down
                            spacing: 0
                            invite_scroll := VScroll{
                                spacing: 10
                                padding: 32
                                Selectable{
                                    text: "Invite"
                                    draw_text.color: #0B1220
                                    draw_text.text_style.font_size: 24
                                }
                                Selectable{
                                    width: Fill
                                    text: "Share your invitation so a guest can store your public key and dial. You do not need theirs first."
                                    draw_text.color: #5A6B7D
                                }
                                invite_button_id := StackPrimaryBtn{ text: "Show QR invitation" }
                                ActionStack{
                                    invite_copy_button := StackSecondaryBtn{ text: "Copy invitation link" }
                                    key_copy_button := StackSecondaryBtn{ text: "Copy public key" }
                                }
                                Selectable{
                                    text: "Join someone"
                                    draw_text.color: #0B1220
                                    draw_text.text_style.font_size: 18
                                }
                                join_nav_button := HubDestBtn{ text: "Paste invitation link or ghalbol:// URI" }
                                scan_nav_button := HubDestBtn{ text: "Scan invitation QR with camera" }
                                scan_file_button := HubDestBtn{ text: "Join from QR picture — PNG or JPEG" }
                            }
                        }
                        settings_pane := Panel{
                            visible: false
                            draw_bg.color: #FFFFFF
                            width: Fill
                            height: Fill
                            flow: Down
                            spacing: 0
                            settings_scroll := VScroll{
                                spacing: 14
                                padding: 32
                                Selectable{
                                    text: "Settings"
                                    draw_text.color: #0B1220
                                    draw_text.text_style.font_size: 24
                                }
                                Selectable{
                                    width: Fill
                                    text: state.identity_blurb
                                    draw_text.color: #5A6B7D
                                }
                                Selectable{
                                    text: "Your identity"
                                    draw_text.color: #0B1220
                                    draw_text.text_style.font_size: 18
                                }
                                Selectable{
                                    width: Fill
                                    text: state.identity_detail
                                    draw_text.color: #0B1220
                                }
                                Selectable{
                                    text: "Display name on invitations"
                                    draw_text.color: #0B1220
                                    draw_text.text_style.font_size: 16
                                }
                                Selectable{
                                    width: Fill
                                    text: "Optional name guests see when they scan your QR. Saved on this device."
                                    draw_text.color: #7A8B9C
                                }
                                display_name_input := Field{
                                    empty_text: "Your name (optional)"
                                    width: Fill
                                }
                                save_display_name_button := StackSecondaryBtn{ text: "Save display name" }
                                Selectable{
                                    text: "Backup and private key"
                                    draw_text.color: #0B1220
                                    draw_text.text_style.font_size: 16
                                }
                                ActionStack{
                                    key_button := StackSecondaryBtn{ text: "Show private key" }
                                    backup_button := StackSecondaryBtn{ text: "Export backup" }
                                    password_button := StackSecondaryBtn{ text: "Change password" }
                                }
                                Selectable{
                                    width: Fill
                                    text: "App password is required to view the secret. Keep backups with your password."
                                    draw_text.color: #7A8B9C
                                }
                                language_button := HubDestBtn{ text: "Language — display language for this app" }
                                delivery_nav_button := HubDestBtn{ text: "Delivery storage" }
                                log_button := HubDestBtn{ text: "App log — session diagnostics" }
                                about_button := HubDestBtn{ text: "Availability and About" }
                                logout_button := Stop{ text: "Log out" width: Fit }
                            }
                        }
                        }
                        bottom_nav := SolidView{
                            visible: false
                            show_bg: true
                            draw_bg.color: #E4EAF3
                            width: Fill
                            height: Fit
                            flow: Right
                            spacing: 0
                            padding: Inset{left: 8. right: 8. top: 8. bottom: 8.}
                            align: Align{y: 0.5}
                            bottom_nav_scroll := HScroll{
                                height: 64
                                padding: Inset{left: 4. right: 4. top: 4. bottom: 4.}
                                bottom_chats_slot := View{
                                    width: Fit height: Fit flow: Overlay
                                    bottom_chats := NavChip{ text: "Chats" }
                                    bottom_chats_on := NavChipOn{ text: "Chats" visible: false }
                                }
                                bottom_contacts_slot := View{
                                    width: Fit height: Fit flow: Overlay
                                    bottom_contacts := NavChip{ text: "Contacts" }
                                    bottom_contacts_on := NavChipOn{ text: "Contacts" visible: false }
                                }
                                bottom_invite_slot := View{
                                    width: Fit height: Fit flow: Overlay
                                    bottom_invite := NavChip{ text: "Invite" }
                                    bottom_invite_on := NavChipOn{ text: "Invite" visible: false }
                                }
                                bottom_settings_slot := View{
                                    width: Fit height: Fit flow: Overlay
                                    bottom_settings := NavChip{ text: "Settings" }
                                    bottom_settings_on := NavChipOn{ text: "Settings" visible: false }
                                }
                            }
                        }
                    }
                    sheet_view := Panel{
                        visible: false
                        draw_bg.color: #EEF2F8
                        width: Fill
                        height: Fill
                        flow: Down
                        spacing: 12
                        padding: Inset{
                            left: 24.
                            right: 24.
                            top: (mod.widgets.SAFE_INSET_PAD_TOP + 20.)
                            bottom: (mod.widgets.SAFE_INSET_PAD_BOTTOM + 16.)
                        }
                        back_button := Quiet{ text: "Back" }
                        sheet_scroll := VScroll{
                            spacing: 14
                            padding: Inset{left: 0. right: 0. top: 4. bottom: 12.}
                            sheet_copy := View{
                                width: Fill
                                height: Fit
                                flow: Down
                                spacing: 12
                                on_render: ||{
                                    sheet_title := Selectable{
                                        width: Fill
                                        text: state.sheet_title
                                        draw_text.color: #0B1220
                                        draw_text.text_style.font_size: 22
                                    }
                                    sheet_body := Selectable{
                                        width: Fill
                                        text: state.sheet_body
                                        draw_text.color: #5A6B7D
                                    }
                                    if state.languages.len() > 0
                                        for i, row in state.languages {
                                            if row.code == ""
                                                Selectable{
                                                    width: Fill
                                                    margin: Inset{left: 4. right: 4. top: 12. bottom: 4.}
                                                    text: row.name
                                                    draw_text.color: #7A8B9C
                                                    draw_text.text_style.font_size: 12
                                                }
                                            else
                                                Quiet{
                                                    width: Fill
                                                    label_walk: Walk{width: Fill, height: Fit}
                                                    text: row.name
                                                    on_click: ||{
                                                        mod.state.lang_code = row.code
                                                        mod.state.lang_gen = mod.state.lang_gen + 1
                                                    }
                                                }
                                        }
                                }
                            }
                            invite_frame := SolidView{
                                visible: false
                                show_bg: true
                                draw_bg.color: #FFFFFF
                                width: Fill
                                height: Fit
                                padding: 18
                                align: Center
                                invite_qr := Image{
                                    width: 280
                                    height: 280
                                }
                            }
                            scan_frame := SolidView{
                                visible: false
                                show_bg: true
                                draw_bg.color: #0B1220
                                width: Fill
                                height: Fit
                                padding: 12
                                align: Center
                                scan_preview := Image{
                                    width: 320
                                    height: 240
                                }
                            }
                            paste_row := View{
                                width: Fill
                                height: Fit
                                flow: Right
                                spacing: 8
                                align: Align{y: 0.5}
                                paste_input := Field{
                                    empty_text: "Invitation link"
                                    width: Fill
                                    height: Fit
                                }
                                paste_eye := Quiet{ text: "Show" visible: false }
                            }
                            pk_row := View{
                                width: Fill
                                height: Fit
                                flow: Right
                                spacing: 8
                                align: Align{y: 0.5}
                                pk_input := Field{
                                    empty_text: "Public key"
                                    width: Fill
                                    height: Fit
                                }
                                pk_eye := Quiet{ text: "Show" visible: false }
                            }
                            alias_row := View{
                                width: Fill
                                height: Fit
                                flow: Right
                                spacing: 8
                                align: Align{y: 0.5}
                                alias_input := Field{
                                    empty_text: "Display name"
                                    width: Fill
                                    height: Fit
                                }
                                alias_eye := Quiet{ text: "Show" visible: false }
                            }
                            status_row := View{
                                visible: false
                                width: Fill
                                height: Fit
                                flow: Right{wrap: true}
                                spacing: 10
                                status_available := Quiet{ text: "Available" }
                                status_busy := Quiet{ text: "Busy" }
                                status_away := Quiet{ text: "Away" }
                                status_call := Quiet{ text: "In a call" }
                                status_clear := Quiet{ text: "Clear" }
                            }
                        }
                        sheet_action := Go{ text: "Save" }
                    }
                    // Voice-first call chrome: peer avatar + name + status. Video tiles
                    // stay hidden until a video call is actually active (empty Image
                    // widgets were the two black rectangles on every voice dial).
                    call_view := SolidView{
                        visible: false
                        show_bg: true
                        draw_bg.color: #0B1220
                        width: Fill
                        height: Fill
                        flow: Down
                        spacing: 0
                        padding: Inset{
                            top: (mod.widgets.SAFE_INSET_PAD_TOP)
                            bottom: (mod.widgets.SAFE_INSET_PAD_BOTTOM)
                            left: (mod.widgets.SAFE_INSET_PAD_LEFT)
                            right: (mod.widgets.SAFE_INSET_PAD_RIGHT)
                        }
                        call_body := View{
                            width: Fill
                            height: Fill
                            flow: Down
                            spacing: 20
                            padding: 28
                            align: Center
                            call_peer_block := View{
                                width: Fill
                                height: Fit
                                flow: Down
                                spacing: 14
                                align: Center
                                call_avatar := RoundedView{
                                    width: 112
                                    height: 112
                                    align: Center
                                    draw_bg.border_radius: 56.0
                                    draw_bg.border_size: 0.0
                                    draw_bg.color: #0183FB
                                    call_initial := Label{
                                        text: "?"
                                        draw_text.color: #FFFFFF
                                        draw_text.text_style.font_size: 42
                                    }
                                }
                                call_peer_name := Label{
                                    width: Fill
                                    text: ""
                                    draw_text.color: #FFFFFF
                                    draw_text.text_style.font_size: 24
                                    draw_text.max_lines: 2
                                    draw_text.text_overflow: Ellipsis
                                }
                            }
                            call_copy := View{
                                width: Fill
                                height: Fit
                                flow: Down
                                spacing: 8
                                align: Center
                                on_render: ||{
                                    // Label — not Selectable (that looked like a text field).
                                    call_screen_title := Label{
                                        width: Fill
                                        text: state.call_line
                                        draw_text.color: #AEC4CF
                                        draw_text.text_style.font_size: 16
                                        draw_text.max_lines: 3
                                        draw_text.text_overflow: Ellipsis
                                    }
                                }
                            }
                            video_row := View{
                                visible: false
                                width: Fill
                                height: Fit
                                flow: Down
                                spacing: 10
                                remote_video := Image{ width: Fill height: 280 }
                                local_video := Image{ width: 140 height: 105 }
                            }
                        }
                        // Labeled controls; HScroll when the row overflows.
                        call_actions := HScroll{
                            width: Fill
                            spacing: 12
                            accept_call_button := Go{ text: "Accept" height: Fit }
                            mute_button := Quiet{ text: "Mute" height: Fit }
                            speaker_button := Quiet{ text: "Speaker" height: Fit }
                            flip_camera_button := Quiet{ text: "Flip camera" height: Fit }
                            end_call_button := Stop{ text: "End call" height: Fit }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Script, ScriptHook)]
pub struct App {
    #[live]
    ui: WidgetRef,
    #[rust]
    password: String,
    #[rust]
    composer: String,
    /// Shown above the transcript when the delivery mailbox is down after a send.
    #[rust]
    delivery_hint: String,
    #[rust]
    namespace: String,
    #[rust]
    session: Option<UnlockedSession>,
    #[rust]
    open_peer: String,
    #[rust]
    seen_open_gen: f64,
    #[rust]
    create_new: bool,
    #[rust]
    algorithm: String,
    #[rust]
    secret: String,
    #[rust]
    search: String,
    #[rust]
    paste: String,
    #[rust]
    add_pk: String,
    #[rust]
    add_alias: String,
    #[rust]
    page: String,
    #[rust]
    hub_tab: u8,
    /// Live window width. `0` means not measured yet — use the design desktop
    /// width so the side rail is not hidden before the first geom event.
    #[rust]
    window_w: f64,
    #[rust]
    shell_split: bool,
    #[rust]
    narrow_show_room: bool,
    #[rust]
    mic_muted: bool,
    #[rust]
    speaker_on: bool,
    #[rust]
    call_open: bool,
    #[rust]
    ui_locked: bool,
    #[rust]
    needs_backup: bool,
    #[rust]
    window_limit: usize,
    #[rust]
    has_more: bool,
    #[rust]
    lang: String,
    #[rust]
    seen_play: f64,
    #[rust]
    seen_lang: f64,
    #[rust]
    seen_algo: f64,
    #[rust]
    pending_invite: String,
    #[rust]
    roster_ver: u64,
    #[rust]
    roster_query: String,
    #[rust]
    hub_history: HubHistoryStack,
    #[rust]
    history_suppress: bool,
    /// Skip `apply_shell_layout` redraw when chrome flags/walks are unchanged.
    #[rust]
    shell_layout_sig: u64,
    /// `(public_key_hex, title)` per rendered roster row, same order as
    /// `roster_list` children — maps a tapped child index back to a peer.
    #[rust]
    roster_rows: Vec<(String, String)>,
    /// Frames left in which `chat_log` is re-pinned to its end. Set after a
    /// transcript change while the user was near the bottom (Flutter
    /// stick-to-bottom); applied on NextFrame once the new content height is laid out.
    #[rust]
    pin_chat_frames: u8,
    /// Line count of the last rendered transcript — growth + near-bottom ⇒ pin.
    #[rust]
    chat_lines_len: usize,
    /// Peer whose transcript `chat_log` currently shows; a change forces a pin.
    #[rust]
    chat_lines_peer: String,
    /// `host::transcript_revision` at the last transcript load.
    #[rust]
    chat_rev: u64,
    /// Room ⋮ menu is open.
    #[rust]
    chat_menu_open: bool,
    /// Nav WebP marks loaded once into rail Image widgets.
    #[rust]
    nav_icons_loaded: bool,
    /// Last recording-bar visibility we applied (avoids redraw spam).
    #[rust]
    recording_ui: bool,
}

impl App {
    fn push_identity_copy(&mut self, cx: &mut Cx) {
        let exists = host::keystore_exists(&self.namespace);
        let primary = if exists {
            "Unlock"
        } else if self.create_new {
            "Create identity"
        } else {
            "Import identity"
        };
        let hint = if exists {
            "Enter your password to unlock this device. An app password encrypts your identity on this device and is never sent to a server."
        } else if self.create_new {
            "Choose a password to create a secp256k1 device identity (required for mailbox text). An app password encrypts it on this device and is never sent to a server."
        } else {
            "Paste a private key and choose its algorithm. Non-secp256k1 keys can call but cannot use mailbox text until multi-algo seal ships."
        };
        let mode = if self.create_new {
            "Import an existing key"
        } else {
            "Create a new identity"
        };
        let setup = !exists && !self.ui_locked;
        let show_secret = setup && !self.create_new;
        let algo_line = if setup {
            format!("Algorithm: {}", self.algorithm)
        } else {
            String::new()
        };
        script_eval!(cx, {
            mod.state.hint = #(hint)
            mod.state.primary = #(primary)
            mod.state.algo_line = #(algo_line)
            ui.unlock_button.set_text(#(primary))
            ui.mode_button.set_text(#(mode))
            ui.mode_button.set_visible(#(setup))
            ui.secret_row.set_visible(#(show_secret))
            ui.algo_row.set_visible(#(setup))
            ui.delete_button.set_visible(#(exists))
            ui.import_backup_button.set_visible(#(setup))
            ui.identity_copy.render()
        });
        self.ui.widget(cx, ids!(mode_button)).set_visible(cx, setup);
        self.ui.widget(cx, ids!(secret_row)).set_visible(cx, show_secret);
        self.ui.widget(cx, ids!(algo_row)).set_visible(cx, setup);
        self.ui.widget(cx, ids!(delete_button)).set_visible(cx, exists);
        self.ui.widget(cx, ids!(import_backup_button)).set_visible(cx, setup);
        // Algo line + chips only while creating/importing — never on unlock.
        // Create: secp256k1 only (mailbox seal). Import: all three (calls work; text needs secp).
        if setup {
            if self.create_new {
                self.algorithm = "secp256k1".into();
            }
            self.mark_algorithm(cx);
            let show_multi = !self.create_new;
            self.ui.widget(cx, ids!(algo_ed)).set_visible(cx, show_multi);
            self.ui.widget(cx, ids!(algo_p256)).set_visible(cx, show_multi);
        }
    }

    fn mark_algorithm(&self, cx: &mut Cx) {
        let secp = if self.algorithm == "secp256k1" {
            "secp256k1  [on]"
        } else {
            "secp256k1"
        };
        let ed = if self.algorithm == "ed25519" {
            "ed25519  [on]"
        } else {
            "ed25519"
        };
        let p256 = if self.algorithm == "ecdsa-p256" {
            "ecdsa-p256  [on]"
        } else {
            "ecdsa-p256"
        };
        let algo_line = if self.create_new {
            "Algorithm: secp256k1 (mailbox text)".to_string()
        } else {
            format!("Algorithm: {}", self.algorithm)
        };
        // Green chip on the active algo; Quiet white on the others.
        let (secp_on, ed_on, p256_on) = (
            self.algorithm == "secp256k1",
            self.algorithm == "ed25519",
            self.algorithm == "ecdsa-p256",
        );
        script_eval!(cx, {
            mod.state.algo_line = #(algo_line)
            ui.algo_secp.set_text(#(secp))
            ui.algo_ed.set_text(#(ed))
            ui.algo_p256.set_text(#(p256))
            ui.identity_copy.render()
        });
        self.style_algo_chip(cx, ids!(algo_secp), secp_on);
        self.style_algo_chip(cx, ids!(algo_ed), ed_on);
        self.style_algo_chip(cx, ids!(algo_p256), p256_on);
    }

    fn style_algo_chip(&self, cx: &mut Cx, path: &[LiveId], on: bool) {
        apply_algo_chip_style(cx, &mut self.ui.widget(cx, path), on);
    }

    fn set_screen(&mut self, cx: &mut Cx, screen: &str) {
        self.ui.widget(cx, ids!(identity_view)).set_visible(cx, screen == "identity");
        self.ui.widget(cx, ids!(hub_view)).set_visible(cx, screen == "hub");
        self.ui.widget(cx, ids!(sheet_view)).set_visible(cx, screen == "sheet");
        self.ui.widget(cx, ids!(call_view)).set_visible(cx, screen == "call");
        if screen == "hub" {
            self.set_hub_tab(cx, self.hub_tab);
        }
        // Only the screen that just became visible — redrawing all four roots
        // on every hop was the navigation stall, not APK size.
        let root = match screen {
            "identity" => ids!(identity_view),
            "hub" => ids!(hub_view),
            "sheet" => ids!(sheet_view),
            "call" => ids!(call_view),
            _ => return,
        };
        self.ui.widget(cx, root).redraw(cx);
    }

    fn set_hub_tab(&mut self, cx: &mut Cx, tab: u8) {
        let prev = self.hub_tab;
        if !self.history_suppress && prev != tab {
            // Pin the UI we are leaving (room, sheet page, …) as the tip before pushing.
            self.hub_history.replace_top(self.hub_snapshot());
        }
        self.hub_tab = tab;
        if tab != 0 {
            self.narrow_show_room = false;
            if !self.shell_split {
                host::set_open_room(None);
            }
        } else if self.shell_split && !self.open_peer.is_empty() {
            host::set_open_room(Some(&self.open_peer));
        } else if !self.shell_split && self.narrow_show_room && !self.open_peer.is_empty() {
            host::set_open_room(Some(&self.open_peer));
        }
        // 0 Chats · 1 Contacts · 2 Invite · 3 Settings (no More hub).
        let chats = tab == 0;
        self.ui.widget(cx, ids!(chats_pane)).set_visible(cx, chats);
        self.ui
            .widget(cx, ids!(contacts_pane))
            .set_visible(cx, tab == 1);
        self.ui
            .widget(cx, ids!(invite_pane))
            .set_visible(cx, tab == 2);
        self.ui
            .widget(cx, ids!(settings_pane))
            .set_visible(cx, tab == 3);
        self.paint_nav_tabs(cx);
        if !self.history_suppress && prev != tab {
            self.record_hub_nav();
        }
        self.apply_shell_layout(cx);
    }

    fn hub_snapshot(&self) -> HubHistoryEntry {
        HubHistoryEntry {
            hub_tab: self.hub_tab,
            narrow_show_room: self.narrow_show_room,
            conversation_key: self.open_peer.clone(),
            page: self.page.clone(),
        }
    }

    fn record_hub_nav(&mut self) {
        if self.history_suppress {
            return;
        }
        self.hub_history.record(self.hub_snapshot());
    }

    fn apply_hub_history(&mut self, cx: &mut Cx, entry: HubHistoryEntry) {
        self.history_suppress = true;
        self.open_peer = entry.conversation_key.clone();
        let pk = self.open_peer.clone();
        let title = if pk.is_empty() {
            "Select a chat".to_string()
        } else {
            // Roster display name when known (Flutter showed the alias, not the key).
            self.roster_rows
                .iter()
                .find(|(row_pk, _)| *row_pk == pk)
                .map(|(_, t)| t.clone())
                .unwrap_or_else(|| short_key(&pk))
        };
        let pk_script = pk.clone();
        let title_script = title.clone();
        script_eval!(cx, {
            mod.state.selected_pk = #(pk_script)
            mod.state.selected_title = #(title_script)
            ui.chat_title_label.set_text(#(title))
        });
        // Apply tab first (may clear narrow_show_room), then restore room flag.
        self.set_hub_tab(cx, entry.hub_tab);
        self.narrow_show_room = entry.narrow_show_room;
        if entry.hub_tab == 0
            && !pk.is_empty()
            && (entry.narrow_show_room || self.shell_split)
        {
            host::set_open_room(Some(&pk));
            self.refresh_transcript(cx);
        } else {
            host::set_open_room(None);
        }
        // Open-room wash + trust banner follow the restored room.
        self.roster_ver = u64::MAX;
        self.refresh_roster(cx);

        let want_page = entry.page.clone();
        if want_page.is_empty() {
            if !self.page.is_empty() {
                self.force_close_sheet(cx);
            } else if self.session.is_none() {
                self.set_screen(cx, "identity");
            }
        } else if want_page == "lang" {
            self.open_languages(cx);
        } else if want_page == "import_backup" {
            self.open_import_backup_sheet(cx);
        } else if want_page != self.page {
            self.open_page(cx, &want_page);
        }

        self.history_suppress = false;
        self.apply_shell_layout(cx);
    }

    /// Close sheet UI without the forced-backup re-prompt (history restore path).
    fn force_close_sheet(&mut self, cx: &mut Cx) {
        if self.page == "scan" {
            self.end_qr_scan_ui();
        }
        self.page.clear();
        let screen = if self.session.is_some() {
            "hub"
        } else {
            "identity"
        };
        let show_hub = screen == "hub";
        script_eval!(cx, {
            ui.sheet_view.set_visible(false)
            ui.call_view.set_visible(false)
            ui.hub_view.set_visible(#(show_hub))
            ui.identity_view.set_visible(#(!show_hub))
        });
        self.set_screen(cx, screen);
        if show_hub {
            self.refresh_roster(cx);
        }
    }

    /// One step of natural back (mouse side button, Android back/gesture, Escape, toolbar Back).
    ///
    /// Hub chrome uses a single browser-style history stack. Guards only for active call
    /// signalling and the forced first-time backup sheet. UI lock is dismissed as one step
    /// (not a hub history entry). Otherwise: `go_back` + apply. At root, `allow_quit` exits.
    fn navigate_back(&mut self, cx: &mut Cx, allow_quit: bool) -> bool {
        if self.call_open {
            match host::call_back_nav() {
                host::CallBackNav::Block => return true,
                host::CallBackNav::End => {
                    let _ = host::end_call();
                    self.refresh_call_line(cx);
                    return true;
                }
                host::CallBackNav::None => {}
            }
        }
        // First-time backup is required — back cannot leave until Save succeeds.
        if self.needs_backup {
            return true;
        }
        if self.ui_locked && self.session.is_some() {
            self.dismiss_ui_lock(cx);
            return true;
        }
        if self.hub_history.can_go_back() {
            let before = self.hub_snapshot();
            if let Some(prev) = self.hub_history.go_back() {
                self.apply_hub_history(cx, prev);
                if self.hub_snapshot() != before {
                    return true;
                }
                // Duplicate / no-op entry — keep walking so Back still moves.
                for _ in 0..8 {
                    let before = self.hub_snapshot();
                    let Some(prev) = self.hub_history.go_back() else {
                        break;
                    };
                    self.apply_hub_history(cx, prev);
                    if self.hub_snapshot() != before {
                        return true;
                    }
                }
            }
        }
        if allow_quit {
            let _ = host::end_call();
            cx.quit();
            return true;
        }
        false
    }

    /// Mouse forward / Alt+Right / browser-style forward through hub history.
    fn navigate_forward(&mut self, cx: &mut Cx) -> bool {
        if self.call_open || self.ui_locked || self.needs_backup {
            return false;
        }
        if self.session.is_none() {
            return false;
        }
        if let Some(next) = self.hub_history.go_forward() {
            self.apply_hub_history(cx, next);
            return true;
        }
        false
    }

    fn dismiss_ui_lock(&mut self, cx: &mut Cx) {
        if !self.ui_locked || self.session.is_none() {
            return;
        }
        self.ui_locked = false;
        self.password.clear();
        script_eval!(cx, {
            ui.password_input.set_text("")
            ui.identity_view.set_visible(false)
            ui.sheet_view.set_visible(false)
            ui.call_view.set_visible(false)
            ui.hub_view.set_visible(true)
        });
        self.configure_secret_field(cx, ids!(password_input), ids!(password_eye), true);
        self.set_screen(cx, "hub");
        self.refresh_roster(cx);
    }

    fn apply_shell_layout(&mut self, cx: &mut Cx) {
        // ≥720 → side rail + list + room; <720 → bottom tabs + stacked list/room.
        // Before the first WindowGeomChange, assume the Window design size (1180) so desktop
        // does not boot into the mobile chrome (missing sidebar).
        const DESIGN_DESKTOP_W: f64 = 1180.0;
        const SPLIT_BREAKPOINT: f64 = 720.0;
        let window_w = if self.window_w >= 1.0 {
            self.window_w
        } else {
            DESIGN_DESKTOP_W
        };
        let split = window_w >= SPLIT_BREAKPOINT;
        if self.shell_split && !split {
            self.narrow_show_room = false;
            host::set_open_room(None);
        }
        if !self.shell_split && split && !self.open_peer.is_empty() {
            host::set_open_room(Some(&self.open_peer));
        }
        self.shell_split = split;

        let on_hub = self.session.is_some() && !self.ui_locked;
        let chats = self.hub_tab == 0;
        let room_open = !self.open_peer.is_empty()
            && chats
            && (split || self.narrow_show_room);
        let show_rail = on_hub && split;
        let show_bottom = on_hub && !split && !(chats && self.narrow_show_room);
        let show_roster = chats && (split || !self.narrow_show_room);
        let show_chat = chats && room_open;
        let form_w = (window_w - 48.0).clamp(240.0, 360.0);
        let rail = if window_w >= 1100.0 { 220.0 } else { 176.0 };
        let roster_w = if split && show_chat {
            (window_w * 0.34).clamp(280.0, 400.0)
        } else {
            0.0
        };
        // Cheap fingerprint — geom spam / no-op tab presses must not repaint hub+identity.
        let sig = (window_w as u64)
            ^ ((self.hub_tab as u64) << 40)
            ^ ((on_hub as u64) << 44)
            ^ ((show_rail as u64) << 45)
            ^ ((show_bottom as u64) << 46)
            ^ ((show_roster as u64) << 47)
            ^ ((show_chat as u64) << 48)
            ^ ((self.narrow_show_room as u64) << 49)
            ^ ((self.ui_locked as u64) << 50)
            ^ ((form_w as u64) << 8)
            ^ ((rail as u64) << 20)
            ^ ((roster_w as u64) << 28);
        if sig == self.shell_layout_sig {
            return;
        }
        self.shell_layout_sig = sig;

        self.ui.view(cx, ids!(identity_column)).set_walk(
            cx,
            Walk {
                width: Size::Fixed(form_w),
                height: Size::fit(),
                ..Default::default()
            },
        );

        self.ui.widget(cx, ids!(nav_rail)).set_visible(cx, show_rail);
        self.ui.widget(cx, ids!(rail_divider)).set_visible(cx, show_rail);
        self.ui.widget(cx, ids!(bottom_nav)).set_visible(cx, show_bottom);
        self.ui.widget(cx, ids!(roster_col)).set_visible(cx, show_roster);
        self.ui.widget(cx, ids!(chat_col)).set_visible(cx, show_chat);
        self.ui.widget(cx, ids!(chat_back_button)).set_visible(cx, show_chat && !split);
        // Flutter puts Lock in the list AppBar on narrow; the side rail owns it on wide.
        self.ui
            .widget(cx, ids!(hub_lock_button))
            .set_visible(cx, on_hub && !show_rail);

        if show_rail {
            // Never use a skinny icon-only rail: peer labels ("Contacts", "Settings")
            // must fit with padding + nav marks. Wide windows get a roomier rail.
            self.ui.view(cx, ids!(nav_rail)).set_walk(
                cx,
                Walk {
                    width: Size::Fixed(rail),
                    height: Size::fill(),
                    ..Default::default()
                },
            );
        }
        if show_roster {
            let width = if roster_w > 0.0 {
                Size::Fixed(roster_w)
            } else {
                Size::fill()
            };
            self.ui.view(cx, ids!(roster_col)).set_walk(
                cx,
                Walk {
                    width,
                    height: Size::fill(),
                    ..Default::default()
                },
            );
        }
        if show_chat {
            self.ui.view(cx, ids!(chat_col)).set_walk(
                cx,
                Walk {
                    width: Size::fill(),
                    height: Size::fill(),
                    ..Default::default()
                },
            );
        }
        if on_hub && !chats {
            let pane = match self.hub_tab {
                1 => ids!(contacts_pane),
                2 => ids!(invite_pane),
                _ => ids!(settings_pane),
            };
            self.ui.view(cx, pane).set_walk(
                cx,
                Walk {
                    width: Size::fill(),
                    height: Size::fill(),
                    ..Default::default()
                },
            );
        }
        if on_hub {
            self.ui.widget(cx, ids!(hub_view)).redraw(cx);
        } else {
            self.ui.widget(cx, ids!(identity_view)).redraw(cx);
        }
    }

    fn pull_password(&mut self, cx: &mut Cx) {
        let live = self.ui.text_input(cx, ids!(password_input)).text();
        let live = live.trim();
        if !live.is_empty() {
            self.password = live.to_string();
        }
        let secret = self.ui.text_input(cx, ids!(secret_input)).text();
        let secret = secret.trim();
        if !secret.is_empty() {
            self.secret = secret.to_string();
        }
        self.sync_algorithm(cx);
    }

    fn sync_algorithm(&mut self, cx: &mut Cx) {
        let gen_val = script_eval!(cx, { mod.state.algo_gen });
        let algo_n = splash_f64(cx, gen_val);
        if algo_n == self.seen_algo || algo_n == 0.0 {
            return;
        }
        self.seen_algo = algo_n;
        let code_val = script_eval!(cx, { mod.state.algo });
        let code = splash_string(cx, code_val);
        if !code.is_empty() {
            self.algorithm = code;
        }
    }

    fn submit_unlock(&mut self, cx: &mut Cx) {
        load_env_once();
        self.pull_password(cx);
        let password = self.password.clone();
        if password.is_empty() {
            script_eval!(cx, {
                mod.state.status = "App password is required."
                ui.identity_copy.render()
            });
            return;
        }
        if self.ui_locked && self.session.is_some() {
            match host::reveal_secret(&self.namespace, &password) {
                Ok(_) => {
                    self.ui_locked = false;
                    self.password.clear();
                    let pk = self
                        .session
                        .as_ref()
                        .map(|s| s.public_key_hex.clone())
                        .unwrap_or_default();
                    self.show_hub(cx, &pk, "");
                }
                Err(e) => {
                    script_eval!(cx, {
                        mod.state.status = #(e)
                        ui.identity_copy.render()
                    });
                }
            }
            return;
        }
        let creating = !host::keystore_exists(&self.namespace) && self.create_new;
        // Shipping mailbox seal is secp256k1-only — force secp on create.
        if creating && self.algorithm != "secp256k1" {
            self.algorithm = "secp256k1".into();
        }
        let secret = if self.create_new { None } else { Some(self.secret.as_str()) };
        let algo = if host::keystore_exists(&self.namespace) {
            None
        } else {
            Some(self.algorithm.as_str())
        };
        match host::unlock_with_options(&self.namespace, &password, algo, secret) {
            Ok(session) => {
                self.password.clear();
                script_eval!(cx, { ui.password_input.set_text("") });
                self.configure_secret_field(cx, ids!(password_input), ids!(password_eye), true);
                self.configure_secret_field(cx, ids!(secret_input), ids!(secret_eye), true);
                let pk = session.public_key_hex.clone();
                let ns = session.app_namespace.clone();
                self.session = Some(session);
                if creating {
                    self.needs_backup = true;
                    self.show_sheet(
                        cx,
                        "backup",
                        "Save your backup",
                        "This identity exists only on this device until you export an encrypted backup. Save it, then keep the file and your app password together.".into(),
                        "Save backup",
                    );
                    return;
                }
                let net = host::start_network(&ns);
                let mut status = match &net {
                    Ok(_) => String::new(),
                    Err(e) => format!("Network: {e}"),
                };
                if let Some(algo) = host::unlocked_algorithm() {
                    if algo != "secp256k1" {
                        let warn = "This identity is not secp256k1 — mailbox text, voice notes, and files will not work until multi-algo seal ships. Calls still work.";
                        status = if status.is_empty() {
                            warn.into()
                        } else {
                            format!("{status}\n{warn}")
                        };
                    }
                }
                self.show_hub(cx, &pk, &status);
            }
            Err(e) => {
                script_eval!(cx, {
                    mod.state.status = #(e)
                    ui.identity_copy.render()
                });
            }
        }
    }

    fn take_pending_invite(&mut self) -> String {
        let raw = std::mem::take(&mut self.pending_invite);
        let raw = raw.trim();
        if raw.is_empty() {
            return String::new();
        }
        match host::accept_invite(&self.namespace, raw) {
            Ok(pk) => format!("Joined {}", short_key(&pk)),
            Err(e) => e,
        }
    }

    fn show_hub(&mut self, cx: &mut Cx, public_key_hex: &str, status: &str) {
        let joined = self.take_pending_invite();
        let status = if joined.is_empty() {
            status.to_string()
        } else if status.is_empty() {
            joined
        } else {
            format!("{status}\n{joined}")
        };
        let self_line = format!("You  {}", short_key(public_key_hex));
        let roster = host::list_roster(&self.namespace).unwrap_or_default();
        script_eval!(cx, {
            mod.state.screen = "hub"
            mod.state.status = #(status)
            mod.state.self_line = #(self_line)
            mod.state.selected_title = "Select a chat"
            mod.state.transcript = ""
            mod.state.roster = []
            ui.identity_view.set_visible(false)
            ui.sheet_view.set_visible(false)
            ui.hub_view.set_visible(true)
            true
        });
        self.push_roster_rows(cx, roster);
        script_eval!(cx, {
            ui.roster_list.render()
            ui.chat_log.render()
            ui.chat_title_label.set_text("Select a chat")
        });
        self.push_settings_pane(cx);
        self.apply_language(cx);
        self.hub_tab = 0;
        self.narrow_show_room = false;
        self.hub_history.reset(self.hub_snapshot());
        self.set_screen(cx, "hub");
        self.paint_nav_tabs(cx);
        self.apply_shell_layout(cx);
    }

    fn submit_lock(&mut self, cx: &mut Cx) {
        self.ui_locked = true;
        self.password.clear();
        script_eval!(cx, {
            mod.state.status = "Enter your password to show chats. The network stays connected."
            ui.hub_view.set_visible(false)
            ui.sheet_view.set_visible(false)
            ui.identity_view.set_visible(true)
            ui.identity_copy.render()
        });
        self.push_identity_copy(cx);
        self.set_screen(cx, "identity");
    }

    fn submit_logout(&mut self, cx: &mut Cx) {
        host::lock();
        self.session = None;
        self.ui_locked = false;
        self.open_peer.clear();
        self.composer.clear();
        self.password.clear();
        script_eval!(cx, {
            mod.state.screen = "identity"
            mod.state.status = ""
            mod.state.roster = []
            mod.state.selected_pk = ""
            mod.state.open_gen = 0
            ui.hub_view.set_visible(false)
            ui.sheet_view.set_visible(false)
            ui.identity_view.set_visible(true)
            ui.identity_copy.render()
        });
        self.seen_open_gen = 0.0;
        self.page.clear();
        self.push_identity_copy(cx);
        self.set_screen(cx, "identity");
    }

    /// Splash `text:` on read-only TextInput often skips relayout until resize.
    /// After `sheet_copy.render()`, push through `set_text` and redraw.
    fn sync_sheet_copy_text(&mut self, cx: &mut Cx, title: &str, body: &str) {
        self.ui.text_input(cx, ids!(sheet_title)).set_text(cx, title);
        self.sync_sheet_body_text(cx, body);
    }

    fn sync_sheet_body_text(&mut self, cx: &mut Cx, body: &str) {
        self.ui.text_input(cx, ids!(sheet_body)).set_text(cx, body);
        self.ui.widget(cx, ids!(sheet_body)).redraw(cx);
        self.ui.widget(cx, ids!(sheet_copy)).redraw(cx);
        self.ui.widget(cx, ids!(sheet_scroll)).redraw(cx);
        self.ui.widget(cx, ids!(sheet_view)).redraw(cx);
    }

    fn show_sheet(&mut self, cx: &mut Cx, page: &str, title: &str, body: String, action: &str) {
        if !self.history_suppress {
            // Pin hub/room under this sheet before page changes.
            self.hub_history.replace_top(self.hub_snapshot());
        }
        if self.page == "scan" && page != "scan" {
            self.end_qr_scan_ui();
        }
        self.page = page.to_string();
        if page == "scan" {
            host::set_qr_scan_ui_active(true);
        }
        let title = title.to_string();
        let action = action.to_string();
        let show_invite = page == "invite";
        let show_scan = page == "scan";
        let show_paste = matches!(
            page,
            "join" | "key" | "password" | "invite" | "import_backup" | "delivery"
        );
        let show_pk = matches!(page, "add" | "blocked" | "password");
        let show_alias = matches!(page, "add" | "about" | "password" | "contacts");
        let paste_secret = matches!(page, "key" | "password");
        let pk_secret = page == "password";
        let alias_secret = page == "password";
        let title_sync = title.clone();
        let body_sync = body.clone();
        script_eval!(cx, {
            mod.state.sheet_title = #(title)
            mod.state.sheet_body = #(body)
            mod.state.languages = []
            ui.sheet_action.set_text(#(action))
            ui.status_row.set_visible(false)
            ui.invite_frame.set_visible(#(show_invite))
            ui.scan_frame.set_visible(#(show_scan))
            ui.paste_row.set_visible(#(show_paste))
            ui.pk_row.set_visible(#(show_pk))
            ui.alias_row.set_visible(#(show_alias))
            ui.hub_view.set_visible(false)
            ui.sheet_view.set_visible(true)
            ui.sheet_copy.render()
        });
        self.sync_sheet_copy_text(cx, &title_sync, &body_sync);
        self.ui.widget(cx, ids!(status_row)).set_visible(cx, false);
        self.ui.widget(cx, ids!(invite_frame)).set_visible(cx, show_invite);
        self.ui.widget(cx, ids!(scan_frame)).set_visible(cx, show_scan);
        self.ui.widget(cx, ids!(paste_row)).set_visible(cx, show_paste);
        self.ui.widget(cx, ids!(pk_row)).set_visible(cx, show_pk);
        self.ui.widget(cx, ids!(alias_row)).set_visible(cx, show_alias);
        self.configure_secret_field(cx, ids!(paste_input), ids!(paste_eye), paste_secret);
        self.configure_secret_field(cx, ids!(pk_input), ids!(pk_eye), pk_secret);
        self.configure_secret_field(cx, ids!(alias_input), ids!(alias_eye), alias_secret);
        if paste_secret {
            self.ui
                .text_input(cx, ids!(paste_input))
                .set_empty_text(cx, "Current password".into());
        } else if page == "join" || page == "invite" {
            self.ui
                .text_input(cx, ids!(paste_input))
                .set_empty_text(cx, "Invitation link".into());
        } else if page == "import_backup" {
            self.ui
                .text_input(cx, ids!(paste_input))
                .set_empty_text(cx, "Paste keystore JSON backup".into());
        }
        if pk_secret {
            self.ui
                .text_input(cx, ids!(pk_input))
                .set_empty_text(cx, "Confirm new password".into());
            self.ui
                .text_input(cx, ids!(alias_input))
                .set_empty_text(cx, "New password".into());
        } else {
            if show_pk {
                self.ui
                    .text_input(cx, ids!(pk_input))
                    .set_empty_text(cx, "Public key".into());
            }
            if show_alias && page != "about" {
                self.ui
                    .text_input(cx, ids!(alias_input))
                    .set_empty_text(cx, "Display name".into());
            } else if show_alias {
                self.ui
                    .text_input(cx, ids!(alias_input))
                    .set_empty_text(cx, "Custom status".into());
            }
        }
        self.set_screen(cx, "sheet");
        self.record_hub_nav();
    }

    /// Mask a field and show a Show/Hide control, or clear masking for ordinary text.
    fn configure_secret_field(
        &mut self,
        cx: &mut Cx,
        input: &[LiveId],
        eye: &[LiveId],
        secret: bool,
    ) {
        let field = self.ui.text_input(cx, input);
        field.set_is_password(cx, secret);
        self.ui.widget(cx, eye).set_visible(cx, secret);
        if secret {
            self.ui.button(cx, eye).set_text(cx, "Show");
        }
    }

    fn toggle_secret_field(&mut self, cx: &mut Cx, input: &[LiveId], eye: &[LiveId]) {
        let field = self.ui.text_input(cx, input);
        field.toggle_is_password(cx);
        let label = if field.is_password() { "Show" } else { "Hide" };
        self.ui.button(cx, eye).set_text(cx, label);
    }

    fn open_page(&mut self, cx: &mut Cx, page: &str) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let lang = self.lang.clone();
        let roster = host::list_roster(&self.namespace).unwrap_or_default();
        match page {
            "invite" => {
                let alias = host::local_display_name(&self.namespace);
                let links = host::invite_links(
                    &session.public_key_hex,
                    if alias.is_empty() {
                        None
                    } else {
                        Some(alias.as_str())
                    },
                )
                .unwrap_or_default();
                let body = "QR encodes your https://ghalbol.com invite (public key and display name). The other person taps Join and scans this code, or pastes the link.".to_string();
                self.show_sheet(
                    cx,
                    page,
                    "Your QR invitation",
                    body,
                    "Copy link",
                );
                self.paste = links.0.clone();
                self.ui.text_input(cx, ids!(paste_input)).set_text(cx, &links.0);
                if let Some(png) = qr_png(&links.0) {
                    let _ = self
                        .ui
                        .image(cx, ids!(invite_qr))
                        .load_png_from_data(cx, &png);
                }
            }
            "join" => self.show_sheet(
                cx,
                page,
                &i18n::phrase(&lang, "paste"),
                i18n::phrase(&lang, "paste_body"),
                &i18n::phrase(&lang, "join_go"),
            ),
            "scan" => {
                self.show_sheet(
                    cx,
                    page,
                    &i18n::phrase(&lang, "scan_qr"),
                    "Point the camera at a Ghal Bol invitation QR (https://ghalbol.com/connect/… or ghalbol://). Live preview below; scanning continues until an invite is read or you go Back.".into(),
                    "Scan again",
                );
                if let Some(png) = scan_placeholder_png() {
                    let _ = self
                        .ui
                        .image(cx, ids!(scan_preview))
                        .load_png_from_data(cx, &png);
                }
            }
            "add" => self.show_sheet(
                cx,
                page,
                &i18n::phrase(&lang, "add"),
                i18n::phrase(&lang, "add_body"),
                &i18n::phrase(&lang, "add_go"),
            ),
            "contacts" => {
                let body = roster
                    .iter()
                    .filter(|r| !r.is_blocked)
                    .map(|r| format!("{}  {}", r.title, short_key(&r.public_key_hex)))
                    .collect::<Vec<_>>()
                    .join("\n");
                let body = if body.is_empty() {
                    i18n::phrase(&lang, "none")
                } else {
                    body
                };
                let target = short_key(&self.open_peer);
                let body = if self.open_peer.is_empty() {
                    format!(
                        "{body}\n\nOpen a chat first to rename or remove that contact. Type a new name in Display name, then Save to rename; leave the name empty and Save to remove."
                    )
                } else {
                    format!(
                        "{body}\n\nOpen chat: {target}\nType a new name in Display name → Save to rename.\nClear the name and Save to remove."
                    )
                };
                self.show_sheet(
                    cx,
                    page,
                    &i18n::phrase(&lang, "contacts"),
                    body,
                    "Save",
                );
            }
            "blocked" => {
                let body = roster
                    .iter()
                    .filter(|r| r.is_blocked)
                    .map(|r| format!("{}  {}", r.title, r.public_key_hex))
                    .collect::<Vec<_>>()
                    .join("\n");
                let body = if body.is_empty() {
                    i18n::phrase(&lang, "blocked_empty")
                } else {
                    format!(
                        "{body}\n\n{}",
                        i18n::phrase(&lang, "blocked_help")
                    )
                };
                self.show_sheet(cx, page, &i18n::phrase(&lang, "blocked"), body, &i18n::phrase(&lang, "toggle"));
            }
            "delivery" => {
                self.show_sheet(
                    cx,
                    page,
                    &i18n::phrase(&lang, "delivery"),
                    host::delivery_summary(),
                    &i18n::phrase(&lang, "refresh"),
                );
                // Paste a message_id then Save to extend TTL (+7 days) while still queued.
            }
            "log" => {
                self.show_sheet(cx, page, &i18n::phrase(&lang, "log"), host::app_log_text(), &i18n::phrase(&lang, "refresh"));
            }
            "key" => self.show_sheet(
                cx,
                page,
                &i18n::phrase(&lang, "key"),
                i18n::phrase(&lang, "key_body"),
                &i18n::phrase(&lang, "show_key"),
            ),
            "backup" => self.show_sheet(
                cx,
                page,
                &i18n::phrase(&lang, "backup"),
                i18n::phrase(&lang, "backup_body"),
                &i18n::phrase(&lang, "save"),
            ),
            "password" => self.show_sheet(
                cx,
                page,
                &i18n::phrase(&lang, "password"),
                i18n::phrase(&lang, "password_body"),
                &i18n::phrase(&lang, "change"),
            ),
            "about" => {
                let status = host::availability_status();
                let status_line = if status.is_empty() {
                    i18n::phrase(&lang, "no_status")
                } else {
                    status.clone()
                };
                let body = format!(
                    "Ghal Bol\n{}\n\nYou  {}\n\n{}",
                    i18n::phrase(&lang, "about_body"),
                    short_key(&session.public_key_hex),
                    status_line
                );
                self.show_sheet(cx, page, &i18n::phrase(&lang, "about"), body, &i18n::phrase(&lang, "save_status"));
                self.add_alias = status;
                let custom = self.add_alias.clone();
                script_eval!(cx, { ui.status_row.set_visible(true) ui.sheet_copy.render() });
                self.ui.widget(cx, ids!(status_row)).set_visible(cx, true);
                self.ui.text_input(cx, ids!(alias_input)).set_text(cx, &custom);
            }
            _ => {}
        }
    }

    fn save_backup(&self) -> Result<String, String> {
        let json = host::export_keystore(&self.namespace)?;
        #[cfg(target_os = "android")]
        {
            return host::android_save_backup_json(&json);
        }
        #[cfg(not(target_os = "android"))]
        {
            let lang = self.lang.clone();
            let Some(path) = rfd::FileDialog::new()
                .set_file_name("ghalbol-keystore.json")
                .save_file()
            else {
                return Err(i18n::phrase(&lang, "must_save"));
            };
            std::fs::write(&path, json).map_err(|e| e.to_string())?;
            Ok(format!(
                "Saved {}. Keep this file and your app password. Older backups still use the password they were exported with.",
                path.display()
            ))
        }
    }

    fn finish_backup(&mut self, cx: &mut Cx, msg: String) {
        if !self.needs_backup {
            script_eval!(cx, {
                mod.state.sheet_body = #(msg.clone())
                ui.sheet_copy.render()
            });
            self.sync_sheet_body_text(cx, &msg);
            return;
        }
        self.needs_backup = false;
        let ns = self.namespace.clone();
        let status = match host::start_network(&ns) {
            Ok(_) => msg,
            Err(e) => format!("{msg}\nNetwork: {e}"),
        };
        let pk = self
            .session
            .as_ref()
            .map(|s| s.public_key_hex.clone())
            .unwrap_or_default();
        self.page.clear();
        self.show_hub(cx, &pk, &status);
    }

    #[cfg(not(target_os = "android"))]
    fn import_backup(&mut self) -> Result<host::UnlockedSession, String> {
        let lang = self.lang.clone();
        if self.password.is_empty() {
            return Err(i18n::phrase(&lang, "enter_password"));
        }
        let Some(path) = rfd::FileDialog::new().pick_file() else {
            return Err(i18n::phrase(&lang, "choose_file"));
        };
        let json = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        host::import_keystore(&self.namespace, &self.password, &json)
    }

    fn import_backup_from_paste(&mut self) -> Result<host::UnlockedSession, String> {
        let lang = self.lang.clone();
        if self.password.is_empty() {
            return Err(i18n::phrase(&lang, "enter_password"));
        }
        let json = self.paste.trim();
        if json.is_empty() {
            return Err("Paste the keystore JSON backup.".into());
        }
        if !(json.starts_with('{') && json.contains("identity_ciphertext")) {
            return Err("That does not look like a Ghal Bol keystore backup JSON.".into());
        }
        host::import_keystore(&self.namespace, &self.password, json)
    }

    /// Status line above the transcript (attach / QR picture / join results).
    fn show_call_line(&mut self, cx: &mut Cx, msg: String) {
        script_eval!(cx, {
            mod.state.call_line = #(msg)
            ui.chat_log.render()
        });
    }

    fn show_attach_result(&mut self, cx: &mut Cx, sent: Result<(), String>) {
        let queued = sent.is_ok();
        self.show_call_line(cx, sent.map(|()| "File queued".to_string()).unwrap_or_else(|e| e));
        if queued {
            self.refresh_transcript(cx);
        }
    }

    /// Decode an invite QR from a picture (desktop path or Android `content://` URI) and join.
    fn join_from_qr_picture(&mut self, cx: &mut Cx, picked: &str) {
        let ns = self.namespace.clone();
        let joined = host::decode_qr_file(picked).and_then(|text| host::accept_invite(&ns, &text));
        let msg = match joined {
            Ok(pk) => format!("Joined {pk}"),
            Err(e) => e,
        };
        self.show_call_line(cx, msg);
        self.refresh_roster(cx);
    }

    #[cfg(target_os = "android")]
    fn on_android_pick(&mut self, cx: &mut Cx, answer: android_pick::Answer) {
        use android_pick::Answer;
        match answer {
            Answer::Picked(id, uri) if id == android_pick::ATTACHMENT => {
                let peer = android_pick::take_attach_peer();
                let sent = host::send_picked_attachment(&self.namespace, &peer, &uri);
                self.show_attach_result(cx, sent);
            }
            Answer::Picked(id, uri) if id == android_pick::BACKUP => {
                let imported = host::read_picked_file(&uri, host::MAX_PICKED_BACKUP_BYTES)
                    .and_then(|(_, bytes)| {
                        String::from_utf8(bytes)
                            .map_err(|_| "That file is not a keystore backup JSON.".to_string())
                    })
                    .and_then(|json| {
                        self.paste = json;
                        self.import_backup_from_paste()
                    });
                match imported {
                    Ok(session) => self.finish_import_backup(cx, session),
                    Err(e) => {
                        script_eval!(cx, {
                            mod.state.status = #(e)
                            ui.identity_copy.render()
                        });
                    }
                }
            }
            Answer::Picked(id, uri) if id == android_pick::QR_PICTURE => {
                self.join_from_qr_picture(cx, &uri);
            }
            // No file chosen: offer pasting the backup JSON instead.
            Answer::Cancelled(id) if id == android_pick::BACKUP => {
                self.open_import_backup_sheet(cx);
            }
            _ => {}
        }
    }

    fn open_import_backup_sheet(&mut self, cx: &mut Cx) {
        self.pull_password(cx);
        let lang = self.lang.clone();
        if self.password.is_empty() {
            script_eval!(cx, {
                mod.state.status = #(i18n::phrase(&lang, "enter_password"))
                ui.identity_copy.render()
            });
            return;
        }
        self.paste.clear();
        self.show_sheet(
            cx,
            "import_backup",
            "Import keystore backup",
            "Paste the full keystore JSON (the file saved as ghalbol-keystore.json). Use the same app password you enter on the unlock screen.".into(),
            "Import",
        );
        self.ui.text_input(cx, ids!(paste_input)).set_text(cx, "");
    }

    fn finish_import_backup(&mut self, cx: &mut Cx, session: host::UnlockedSession) {
        self.password.clear();
        let pk = session.public_key_hex.clone();
        let ns = session.app_namespace.clone();
        self.session = Some(session);
        self.page.clear();
        let status = match host::start_network(&ns) {
            Ok(_) => "Backup imported".into(),
            Err(e) => format!("Network: {e}"),
        };
        self.show_hub(cx, &pk, &status);
    }

    fn close_sheet(&mut self, cx: &mut Cx) {
        let lang = self.lang.clone();
        if self.needs_backup {
            self.show_sheet(
                cx,
                "backup",
                &i18n::phrase(&lang, "save_backup"),
                i18n::phrase(&lang, "save_backup_body"),
                &i18n::phrase(&lang, "save"),
            );
            return;
        }
        if self.page == "scan" {
            self.end_qr_scan_ui();
        }
        self.page.clear();
        script_eval!(cx, {
            ui.sheet_view.set_visible(false)
            ui.identity_view.set_visible(false)
            ui.call_view.set_visible(false)
            ui.hub_view.set_visible(true)
        });
        self.set_screen(cx, "hub");
        self.refresh_roster(cx);
        // Finished a sheet via Back button / action — replace so Forward does not
        // resurrect a completed flow. History walks use force_close_sheet instead.
        if !self.history_suppress {
            self.hub_history.replace_top(self.hub_snapshot());
        }
    }

    fn save_availability(&mut self, status: &str) -> Result<String, String> {
        self.add_alias = status.to_string();
        save_status_line(status)
    }

    fn apply_status_preset(&mut self, cx: &mut Cx, status: &str) {
        let msg = save_status_line(status).unwrap_or_else(|e| e);
        self.add_alias = status.to_string();
        let custom = self.add_alias.clone();
        script_eval!(cx, {
            mod.state.sheet_body = #(msg.clone())
            ui.sheet_copy.render()
        });
        self.sync_sheet_body_text(cx, &msg);
        self.ui.text_input(cx, ids!(alias_input)).set_text(cx, &custom);
        self.refresh_roster(cx);
    }

    fn sheet_go(&mut self, cx: &mut Cx) {
        let paste = self.ui.text_input(cx, ids!(paste_input)).text();
        if !paste.is_empty() {
            self.paste = paste;
        }
        let pk = self.ui.text_input(cx, ids!(pk_input)).text();
        if !pk.is_empty() {
            self.add_pk = pk;
        }
        let alias = self.ui.text_input(cx, ids!(alias_input)).text();
        if !alias.is_empty()
            || self.page == "about"
            || self.page == "password"
            || self.page == "contacts"
        {
            self.add_alias = alias;
        }
        let page = self.page.clone();
        let ns = self.namespace.clone();
        let result = match page.as_str() {
            "join" => host::accept_invite(&ns, &self.paste).map(|pk| format!("Added {}", short_key(&pk))),
            "add" => host::add_contact(&ns, &self.add_pk, Some(self.add_alias.trim())).map(|()| "Contact added".into()),
            "contacts" => {
                let pk = self.open_peer.trim().to_string();
                if pk.is_empty() {
                    Err("Open a chat first, then rename or remove that contact.".into())
                } else if self.add_alias.trim().is_empty() {
                    host::remove_contact(&ns, &pk).map(|()| "Removed".into())
                } else {
                    host::rename_contact(&ns, &pk, self.add_alias.trim())
                        .map(|()| "Renamed".into())
                }
            }
            "blocked" => {
                let pk = self.add_pk.trim().to_string();
                let blocked = host::list_roster(&ns)
                    .unwrap_or_default()
                    .into_iter()
                    .find(|r| r.public_key_hex.eq_ignore_ascii_case(&pk))
                    .map(|r| r.is_blocked)
                    .unwrap_or(false);
                host::set_blocked(&ns, &pk, !blocked).map(|()| {
                    if blocked {
                        "Unblocked".into()
                    } else {
                        "Blocked".into()
                    }
                })
            }
            "about" => {
                let status = self.add_alias.trim().to_string();
                self.save_availability(&status)
            }
            "delivery" => {
                let mid = self.paste.trim();
                if mid.is_empty() {
                    // Refresh — reopen paints a fresh summary + quota.
                    self.open_page(cx, "delivery");
                    return;
                }
                host::extend_mailbox_ttl(mid).map(|()| "TTL extended".into())
            }
            "log" => Ok(String::new()),
            "invite" => {
                let link = self.paste.trim();
                if link.is_empty() {
                    Err("Invitation link is empty.".into())
                } else {
                    copy_to_clipboard(cx, link).map(|()| "Invitation copied".into())
                }
            }
            "scan" => {
                match host::start_qr_scan() {
                    Ok(()) => Ok("Point the camera at an invitation QR…".into()),
                    Err(e) => Err(e),
                }
            }
            "key" => host::reveal_secret(&ns, &self.paste),
            "backup" => self.save_backup(),
            "import_backup" => {
                self.pull_password(cx);
                match self.import_backup_from_paste() {
                    Ok(session) => {
                        self.finish_import_backup(cx, session);
                        return;
                    }
                    Err(e) => Err(e),
                }
            }
            "password" => {
                let current = self.paste.trim();
                let new_password = self.add_alias.trim();
                let confirm = self.add_pk.trim();
                if current.is_empty() || new_password.is_empty() {
                    Err("Enter the current password and a new password.".into())
                } else if new_password != confirm {
                    Err("New password and confirmation do not match.".into())
                } else if new_password == current {
                    Err("Choose a different password.".into())
                } else {
                    host::change_password(&ns, current, new_password)
                        .map(|()| "Password changed. Export a new backup. Older backups still use the old password.".into())
                }
            }
            _ => {
                self.close_sheet(cx);
                return;
            }
        };
        match result {
            Ok(msg) => {
                if page == "delivery" || page == "log" {
                    self.open_page(cx, &page);
                } else if page == "key" || page == "password" || page == "invite" || page == "scan" {
                    script_eval!(cx, {
                        mod.state.sheet_body = #(msg.clone())
                        ui.sheet_copy.render()
                    });
                    self.sync_sheet_body_text(cx, &msg);
                } else if page == "backup" {
                    self.finish_backup(cx, msg);
                } else {
                    if page == "contacts" {
                        self.open_peer.clear();
                        self.roster_ver = u64::MAX;
                    }
                    self.close_sheet(cx);
                }
            }
            Err(e) => {
                script_eval!(cx, {
                    mod.state.sheet_body = #(e.clone())
                    ui.sheet_copy.render()
                });
                self.sync_sheet_body_text(cx, &e);
            }
        }
    }

    fn transcript_limit(&self) -> usize {
        if self.window_limit == 0 { 50 } else { self.window_limit }
    }

    fn toggle_voice_note(&mut self, cx: &mut Cx) {
        let peer = self.open_peer.clone();
        if peer.is_empty() {
            return;
        }
        let msg = match host::voice_note_toggle() {
            Ok(None) => {
                self.sync_recording_bar(cx);
                String::new()
            }
            Ok(Some((duration_ms, opus))) => {
                self.sync_recording_bar(cx);
                if duration_ms < 400 {
                    "Recording too short".into()
                } else {
                    match host::send_voice_note(&peer, duration_ms, opus) {
                        Ok(()) => "Voice note sent".into(),
                        Err(e) => e,
                    }
                }
            }
            Err(e) => {
                self.sync_recording_bar(cx);
                e
            }
        };
        let sent = msg == "Voice note sent";
        if !msg.is_empty() {
            script_eval!(cx, {
                mod.state.call_line = #(msg)
                ui.chat_log.render()
            });
        }
        if sent {
            self.refresh_transcript(cx);
        }
    }

    fn cancel_voice_note(&mut self, cx: &mut Cx) {
        let _ = host::voice_note_cancel();
        self.sync_recording_bar(cx);
        script_eval!(cx, {
            mod.state.call_line = "Recording discarded"
            ui.chat_log.render()
        });
    }

    /// Show/hide the Flutter-style recording bar and refresh the elapsed timer.
    fn sync_recording_bar(&mut self, cx: &mut Cx) {
        let recording = host::voice_note_recording();
        if recording != self.recording_ui {
            self.recording_ui = recording;
            self.ui
                .widget(cx, ids!(recording_bar))
                .set_visible(cx, recording);
            self.ui
                .widget(cx, ids!(composer_row))
                .set_visible(cx, !recording);
            self.ui.view(cx, ids!(composer_bar)).redraw(cx);
        }
        if recording {
            let ms = host::voice_note_elapsed_ms().unwrap_or(0);
            let label = format_mmss(ms);
            script_eval!(cx, { ui.record_timer_label.set_text(#(label)) });
        }
    }

    fn set_chat_menu_open(&mut self, cx: &mut Cx, open: bool) {
        if self.chat_menu_open == open {
            return;
        }
        self.chat_menu_open = open;
        self.ui.widget(cx, ids!(chat_menu)).set_visible(cx, open);
        self.ui.view(cx, ids!(chat_bar)).redraw(cx);
    }

    fn open_languages(&mut self, cx: &mut Cx) {
        if !self.history_suppress {
            self.hub_history.replace_top(self.hub_snapshot());
        }
        self.page = "lang".into();
        script_eval!(cx, {
            mod.state.sheet_title = "Language"
            mod.state.sheet_body = "Choose the language for this app. Native names need the bundled script fonts."
            mod.state.languages = []
            ui.hub_view.set_visible(false)
            ui.sheet_view.set_visible(true)
            true
        });
        let mut last_region = "";
        for lang in i18n::LANGUAGES {
            if lang.region != last_region {
                last_region = lang.region;
                let region = lang.region.to_string();
                script_eval!(cx, {
                    mod.state.languages.push({ code: "" name: #(region) })
                    true
                });
            }
            let code = lang.code.to_string();
            let name = if lang.native_name == lang.name_en {
                lang.native_name.to_string()
            } else {
                format!("{} — {}", lang.native_name, lang.name_en)
            };
            script_eval!(cx, {
                mod.state.languages.push({ code: #(code) name: #(name) })
                true
            });
        }
        script_eval!(cx, { ui.sheet_copy.render() });
        self.sync_sheet_copy_text(
            cx,
            "Language",
            "Choose the language for this app. Native names need the bundled script fonts.",
        );
        self.ui.widget(cx, ids!(invite_frame)).set_visible(cx, false);
        self.ui.widget(cx, ids!(scan_frame)).set_visible(cx, false);
        self.ui.widget(cx, ids!(paste_row)).set_visible(cx, false);
        self.ui.widget(cx, ids!(pk_row)).set_visible(cx, false);
        self.ui.widget(cx, ids!(alias_row)).set_visible(cx, false);
        self.ui.widget(cx, ids!(status_row)).set_visible(cx, false);
        self.set_screen(cx, "sheet");
        self.record_hub_nav();
    }

    fn sync_play(&mut self, cx: &mut Cx) {
        let gen_val = script_eval!(cx, { mod.state.play_gen });
        let play_n = splash_f64(cx, gen_val);
        if play_n == self.seen_play {
            return;
        }
        self.seen_play = play_n;
        let path_val = script_eval!(cx, { mod.state.play_path });
        let path = splash_string(cx, path_val);
        let action_val = script_eval!(cx, { mod.state.play_action });
        let action = splash_string(cx, action_val);
        if path.is_empty() {
            return;
        }
        let peer = self.open_peer.clone();
        let msg: Result<String, String> = if action == "voice" {
            host::play_voice_file(&path)
        } else if action == "fetch" {
            host::fetch_attachment(&peer, &path)
        } else if action == "save" {
            host::save_attachment(&path)
        } else if action == "resend" {
            host::resend_failed(&peer, &path).map(|()| "Resent".into())
        } else {
            host::open_local_file(&path).map(|()| "Opened".into())
        };
        let note = match &msg {
            Ok(text) if action == "fetch" || action == "voice" || action == "resend" => {
                text.clone()
            }
            Err(e) => e.clone(),
            Ok(_) => String::new(),
        };
        if action == "resend" && msg.is_ok() {
            self.chat_rev = 0;
            self.refresh_transcript(cx);
        }
        if !note.is_empty() {
            script_eval!(cx, {
                mod.state.call_line = #(note)
                ui.chat_log.render()
            });
        }
    }

    fn sync_language(&mut self, cx: &mut Cx) {
        let gen_val = script_eval!(cx, { mod.state.lang_gen });
        let lang_n = splash_f64(cx, gen_val);
        if lang_n == self.seen_lang || lang_n == 0.0 {
            return;
        }
        self.seen_lang = lang_n;
        let code_val = script_eval!(cx, { mod.state.lang_code });
        let code = splash_string(cx, code_val);
        if code.is_empty() {
            return;
        }
        self.lang = code.clone();
        let _ = host::set_ui_locale(&self.namespace, &code);
        self.apply_language(cx);
        self.close_sheet(cx);
    }

    fn apply_language(&self, cx: &mut Cx) {
        let lang = self.lang.as_str();
        let unlock = i18n::text(lang, i18n::Key::Unlock).to_string();
        let new_chat = i18n::text(lang, i18n::Key::NewChat).to_string();
        let invitation = i18n::text(lang, i18n::Key::Invitation).to_string();
        let join = i18n::text(lang, i18n::Key::Join).to_string();
        let contacts = i18n::text(lang, i18n::Key::Contacts).to_string();
        let blocked = i18n::text(lang, i18n::Key::Blocked).to_string();
        let delivery = i18n::text(lang, i18n::Key::Delivery).to_string();
        let log = i18n::text(lang, i18n::Key::AppLog).to_string();
        let key = i18n::text(lang, i18n::Key::PrivateKey).to_string();
        let backup = i18n::text(lang, i18n::Key::Backup).to_string();
        let password = i18n::text(lang, i18n::Key::PasswordMenu).to_string();
        let about = i18n::text(lang, i18n::Key::About).to_string();
        let language = i18n::text(lang, i18n::Key::Language).to_string();
        let lock = i18n::text(lang, i18n::Key::Lock).to_string();
        let lock_hub = lock.clone();
        let logout = i18n::text(lang, i18n::Key::Logout).to_string();
        let older = i18n::text(lang, i18n::Key::Older).to_string();
        let accept = i18n::text(lang, i18n::Key::Accept).to_string();
        let mute = i18n::text(lang, i18n::Key::Mute).to_string();
        let speaker = i18n::phrase(lang, "speaker");
        let flip_camera = i18n::phrase(lang, "flip_camera").to_string();
        let end = i18n::text(lang, i18n::Key::EndCall).to_string();
        let scan = i18n::text(lang, i18n::Key::ScanQr).to_string();
        let scan_file = i18n::text(lang, i18n::Key::QrPicture).to_string();
        let chats = i18n::text(lang, i18n::Key::Chats).to_string();
        let invite_tab = i18n::text(lang, i18n::Key::InviteTab).to_string();
        let settings = i18n::text(lang, i18n::Key::Settings).to_string();
        let contacts_menu = contacts.clone();
        let voice_call = i18n::text(lang, i18n::Key::VoiceCall).to_string();
        let video_call = i18n::text(lang, i18n::Key::VideoCall).to_string();
        let share_menu = i18n::phrase(lang, "share");
        let join_menu = join.clone();
        let add_menu = i18n::phrase(lang, "add_short");
        let block_menu = i18n::text(lang, i18n::Key::Block).to_string();
        let lock_menu = lock.clone();
        let add_contact_nav = i18n::phrase(lang, "add");
        let join_nav = i18n::phrase(lang, "paste");
        let scan_nav = scan.clone();
        let delivery_nav = delivery.clone();
        let show_qr = invitation.clone();
        let nav = [
            chats.clone(),
            chats.clone(),
            chats.clone(),
            chats,
            contacts.clone(),
            contacts.clone(),
            contacts.clone(),
            contacts,
            invite_tab.clone(),
            invite_tab.clone(),
            invite_tab.clone(),
            invite_tab,
            settings.clone(),
            settings.clone(),
            settings.clone(),
            settings,
        ];
        let [
            tab_chats_l,
            tab_chats_on_l,
            bottom_chats_l,
            bottom_chats_on_l,
            tab_contacts_l,
            tab_contacts_on_l,
            bottom_contacts_l,
            bottom_contacts_on_l,
            tab_invite_l,
            tab_invite_on_l,
            bottom_invite_l,
            bottom_invite_on_l,
            tab_settings_l,
            tab_settings_on_l,
            bottom_settings_l,
            bottom_settings_on_l,
        ] = nav;
        script_eval!(cx, {
            ui.unlock_button.set_text(#(unlock))
            ui.new_chat_button.set_text(#(new_chat))
            ui.invite_button.set_text(#(invitation))
            ui.join_button.set_text(#(join))
            ui.scan_button.set_text(#(scan))
            ui.scan_file_button.set_text(#(scan_file))
            ui.contacts_button.set_text(#(contacts_menu))
            ui.blocked_button.set_text(#(blocked))
            ui.delivery_button.set_text(#(delivery))
            ui.log_button.set_text(#(log))
            ui.key_button.set_text(#(key))
            ui.backup_button.set_text(#(backup))
            ui.password_button.set_text(#(password))
            ui.about_button.set_text(#(about))
            ui.language_button.set_text(#(language))
            ui.lock_button.set_text(#(lock))
            ui.hub_lock_button.set_text(#(lock_hub))
            ui.logout_button.set_text(#(logout))
            ui.older_button.set_text(#(older))
            // Hub peers — idle + active faces share the same label.
            ui.tab_chats.set_text(#(tab_chats_l))
            ui.tab_chats_on.set_text(#(tab_chats_on_l))
            ui.bottom_chats.set_text(#(bottom_chats_l))
            ui.bottom_chats_on.set_text(#(bottom_chats_on_l))
            ui.tab_contacts.set_text(#(tab_contacts_l))
            ui.tab_contacts_on.set_text(#(tab_contacts_on_l))
            ui.bottom_contacts.set_text(#(bottom_contacts_l))
            ui.bottom_contacts_on.set_text(#(bottom_contacts_on_l))
            ui.tab_invite.set_text(#(tab_invite_l))
            ui.tab_invite_on.set_text(#(tab_invite_on_l))
            ui.bottom_invite.set_text(#(bottom_invite_l))
            ui.bottom_invite_on.set_text(#(bottom_invite_on_l))
            ui.tab_settings.set_text(#(tab_settings_l))
            ui.tab_settings_on.set_text(#(tab_settings_on_l))
            ui.bottom_settings.set_text(#(bottom_settings_l))
            ui.bottom_settings_on.set_text(#(bottom_settings_on_l))
            // Call screen keeps readable labels (not FA-only).
            ui.accept_call_button.set_text(#(accept))
            ui.mute_button.set_text(#(mute))
            ui.speaker_button.set_text(#(speaker))
            ui.flip_camera_button.set_text(#(flip_camera))
            ui.end_call_button.set_text(#(end))
            // Room ⋮ menu + peer HubDest rows (composer IconBtn glyphs stay FA).
            ui.menu_call_button.set_text(#(voice_call))
            ui.menu_video_button.set_text(#(video_call))
            ui.menu_share_button.set_text(#(share_menu))
            ui.menu_join_button.set_text(#(join_menu))
            ui.menu_add_button.set_text(#(add_menu))
            ui.menu_block_button.set_text(#(block_menu))
            ui.menu_lock_button.set_text(#(lock_menu))
            ui.add_contact_button.set_text(#(add_contact_nav))
            ui.join_nav_button.set_text(#(join_nav))
            ui.scan_nav_button.set_text(#(scan_nav))
            ui.delivery_nav_button.set_text(#(delivery_nav))
            ui.invite_button_id.set_text(#(show_qr))
        });
        // Composer/mic/attach stay FA IconBtn glyphs — keep Key variants constructed
        // so catalogs stay complete without translating onto icon faces.
        let _chrome_icon_keys = (
            i18n::text(lang, i18n::Key::VoiceNote),
            i18n::text(lang, i18n::Key::ChooseFile),
            i18n::text(lang, i18n::Key::Send),
        );
        let _ = _chrome_icon_keys;
    }

    fn sync_open_chat(&mut self, cx: &mut Cx) {
        if self.session.is_none() {
            return;
        }
        let open_val = script_eval!(cx, { mod.state.open_gen });
        let open_gen = splash_f64(cx, open_val);
        let pk_val = script_eval!(cx, { mod.state.selected_pk });
        let pk = splash_string(cx, pk_val);
        if open_gen == self.seen_open_gen && pk == self.open_peer {
            return;
        }
        self.seen_open_gen = open_gen;
        let peer_changed = pk != self.open_peer;
        self.open_peer = pk.clone();
        if pk.is_empty() {
            host::set_open_room(None);
            self.narrow_show_room = false;
            self.sync_trust_banner(cx, &[]);
            self.apply_shell_layout(cx);
            if peer_changed {
                self.roster_ver = u64::MAX;
                self.refresh_roster(cx);
                self.hub_history.replace_top(self.hub_snapshot());
            }
            return;
        }
        let opened_room = if !self.shell_split {
            let was = self.narrow_show_room;
            self.narrow_show_room = true;
            !was
        } else {
            peer_changed
        };
        host::set_open_room(Some(&pk));
        host::clear_unread(&self.namespace, &pk);
        // Force roster repaint so open-room green / unread blue wash update now
        // (not only after the next contacts_version bump).
        self.roster_ver = u64::MAX;
        self.refresh_roster(cx);
        let title_val = script_eval!(cx, { mod.state.selected_title });
        let title = splash_string(cx, title_val);
        if !title.is_empty() {
            script_eval!(cx, { ui.chat_title_label.set_text(#(title)) });
        }
        self.apply_shell_layout(cx);
        self.refresh_transcript(cx);
        if opened_room {
            self.record_hub_nav();
        }
    }

    /// Leave the open room entirely (block, remove): native leave drain via
    /// `set_open_room(None)`, hub state cleared so the poll does not reopen it.
    fn close_open_room(&mut self, cx: &mut Cx) {
        self.open_peer.clear();
        host::set_open_room(None);
        self.narrow_show_room = false;
        self.chat_lines_peer.clear();
        self.chat_lines_len = 0;
        self.set_chat_menu_open(cx, false);
        if self.recording_ui {
            let _ = host::voice_note_cancel();
            self.sync_recording_bar(cx);
        }
        script_eval!(cx, {
            mod.state.selected_pk = ""
            mod.state.selected_title = "Select a chat"
            mod.state.lines = []
            mod.state.transcript = ""
            ui.chat_title_label.set_text("Select a chat")
            ui.chat_log.render()
        });
        self.roster_ver = u64::MAX;
        self.refresh_roster(cx);
        self.apply_shell_layout(cx);
        self.hub_history.replace_top(self.hub_snapshot());
    }

    /// Contact-trust banner (Add / Block) for the open peer — visible only while
    /// `is_known` is false. Chrome only; ack policy lives in `ghalbol_core`.
    fn sync_trust_banner(&mut self, cx: &mut Cx, roster: &[RosterEntry]) {
        let unknown = !self.open_peer.is_empty()
            && roster
                .iter()
                .find(|r| r.public_key_hex == self.open_peer)
                .map(|r| !r.is_known && !r.is_blocked)
                .unwrap_or(false);
        let banner = self.ui.view(cx, ids!(trust_banner));
        if banner.visible() != unknown {
            banner.set_visible(cx, unknown);
            self.ui.view(cx, ids!(chat_col)).redraw(cx);
        }
    }

    /// True when the last rendered chat row sits within ~140 px of the visible
    /// bottom of `chat_log` (Flutter `_stickToBottomThreshold`), or nothing is drawn yet.
    fn chat_log_near_bottom(&self, cx: &mut Cx) -> bool {
        let log = self.ui.view(cx, ids!(chat_log));
        let log_rect = log.area().rect(cx);
        if log_rect.size.y <= 0.0 {
            return true;
        }
        let Some(inner) = log.borrow_mut() else {
            return true;
        };
        let Some((_, last)) = inner.children.last() else {
            return true;
        };
        let last_rect = last.area().rect(cx);
        if last_rect.size.y <= 0.0 {
            return true;
        }
        last_rect.pos.y + last_rect.size.y <= log_rect.pos.y + log_rect.size.y + 140.0
    }

    /// Scroll `chat_log` to its end once the new content height is known.
    /// `set_scroll_pos` clamps to the *last drawn* total, so an immediate call
    /// after `render()` stops short — re-apply on the next two frames.
    fn pin_chat_bottom(&mut self, cx: &mut Cx) {
        self.pin_chat_frames = 2;
        self.ui
            .view(cx, ids!(chat_log))
            .set_scroll_pos(cx, dvec2(0.0, 1.0e9));
        let _ = cx.new_next_frame();
    }

    fn apply_chat_pin_frame(&mut self, cx: &mut Cx) {
        if self.pin_chat_frames == 0 {
            return;
        }
        self.pin_chat_frames -= 1;
        self.ui
            .view(cx, ids!(chat_log))
            .set_scroll_pos(cx, dvec2(0.0, 1.0e9));
        if self.pin_chat_frames > 0 {
            let _ = cx.new_next_frame();
        }
    }

    fn refresh_transcript(&mut self, cx: &mut Cx) {
        let Some(session) = self.session.clone() else {
            return;
        };
        if self.open_peer.is_empty() {
            return;
        }
        self.chat_rev = host::transcript_revision(&self.namespace, &self.open_peer);
        let page = match host::load_transcript(&session.app_namespace, &self.open_peer, self.transcript_limit()) {
            Ok(page) => page,
            Err(e) => {
                let err = format!("Could not load chat: {e}");
                script_eval!(cx, {
                    mod.state.transcript = #(err)
                    mod.state.lines = []
                    ui.chat_log.render()
                });
                return;
            }
        };
        let empty = if page.lines.is_empty() {
            "No messages yet".to_string()
        } else {
            String::new()
        };
        self.has_more = page.has_more;
        self.ui
            .widget(cx, ids!(older_button))
            .set_visible(cx, self.has_more);
        // Stick-to-bottom (Flutter): a freshly opened room always lands on the
        // newest line; a grown transcript follows only if the user was already
        // near the end — never yank someone reading older history.
        let room_changed = self.chat_lines_peer != self.open_peer;
        let grew = page.lines.len() > self.chat_lines_len;
        let follow = room_changed || (grew && self.chat_log_near_bottom(cx));
        self.chat_lines_peer = self.open_peer.clone();
        self.chat_lines_len = page.lines.len();
        let peer_title = self
            .roster_rows
            .iter()
            .find(|(pk, _)| pk == &self.open_peer)
            .map(|(_, t)| t.clone())
            .unwrap_or_else(|| short_key(&self.open_peer));
        script_eval!(cx, {
            mod.state.transcript = #(empty)
            mod.state.lines = []
            true
        });
        for line in page.lines {
            let bubble = bubble_of(&line, &peer_title);
            let body = bubble.body;
            let detail = bubble.detail;
            let caption = bubble.caption;
            let tick = bubble.tick;
            let read = if bubble.read {
                "1"
            } else if bubble.failed {
                "failed"
            } else {
                "0"
            };
            let side = if line.outgoing { "out" } else { "in" };
            let open = if line.kind == "voice" && !line.audio_path.is_empty() {
                line.audio_path.clone()
            } else if !line.local_path.is_empty() {
                line.local_path.clone()
            } else if line.kind == "attachment_offer" && !line.message_id.is_empty() {
                line.message_id.clone()
            } else {
                String::new()
            };
            let (open, action) = if line.outgoing
                && line.delivery == "failed"
                && !line.message_id.is_empty()
                && (line.kind == "text"
                    || line.kind.is_empty()
                    || line.kind == "voice"
                    || (line.kind == "attachment_offer" && !line.local_path.is_empty()))
            {
                (line.message_id.clone(), "resend".to_string())
            } else if line.kind == "voice" && !line.audio_path.is_empty() {
                (open, "voice".to_string())
            } else if line.kind == "attachment_offer" && line.local_path.is_empty() && !line.message_id.is_empty() {
                (line.message_id.clone(), "fetch".to_string())
            } else if line.kind == "attachment_offer" && !line.local_path.is_empty() {
                (open, "file".to_string())
            } else if !open.is_empty() && line.kind != "text" {
                (open, "file".to_string())
            } else {
                (open, String::new())
            };
            let body = if action == "resend" {
                format!(
                    "{body}\n{}",
                    i18n::phrase(&self.lang, "tap_to_resend")
                )
            } else {
                body
            };
            script_eval!(cx, {
                mod.state.lines.push({
                    body: #(body)
                    detail: #(detail)
                    caption: #(caption)
                    tick: #(tick)
                    read: #(read)
                    side: #(side)
                    path: #(open)
                    action: #(action)
                })
                true
            });
        }
        script_eval!(cx, { ui.chat_log.render() });
        if follow {
            self.pin_chat_bottom(cx);
        }
    }

    fn refresh_roster(&mut self, cx: &mut Cx) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let ver = host::contacts_version();
        if ver == self.roster_ver && self.roster_query == self.search {
            return;
        }
        self.roster_ver = ver;
        self.roster_query = self.search.clone();
        let self_line = format!("You  {}", short_key(&session.public_key_hex));
        let roster = host::list_roster(&self.namespace).unwrap_or_default();
        script_eval!(cx, {
            mod.state.self_line = #(self_line)
            mod.state.roster = []
            true
        });
        self.push_roster_rows(cx, roster);
        script_eval!(cx, { ui.roster_list.render() });
    }

    /// Fill `mod.state.roster` (and `self.roster_rows`) from the native roster:
    /// blocked peers are hidden, the search filter applies, and each row carries
    /// the fields `RosterRow` renders. Also syncs the open room's trust banner.
    fn push_roster_rows(&mut self, cx: &mut Cx, roster: Vec<RosterEntry>) {
        self.sync_trust_banner(cx, &roster);
        self.roster_rows.clear();
        let searching = !self.search.trim().is_empty();
        let mut shown = 0usize;
        for row in roster
            .into_iter()
            .filter(|row| !row.is_blocked && roster_matches(row, &self.search))
        {
            let title = row.title.clone();
            let pk = row.public_key_hex.clone();
            let initial = roster_initial(&row);
            let status = row.availability.clone();
            let preview = if row.preview.is_empty() {
                "Tap to open chat".to_string()
            } else {
                row.preview.clone()
            };
            let unknown = if row.is_known { "0" } else { "1" };
            let unread = row.unread.to_string();
            self.roster_rows.push((pk.clone(), title.clone()));
            shown += 1;
            script_eval!(cx, {
                mod.state.roster.push({
                    title: #(title)
                    initial: #(initial)
                    status: #(status)
                    preview: #(preview)
                    unknown: #(unknown)
                    pk: #(pk)
                    unread: #(unread)
                })
                true
            });
        }
        // Flutter empty-search copy — distinguish "no chats" from "no match".
        let (empty_title, empty_body) = if shown == 0 && searching {
            (
                "No matches".to_string(),
                "Try a different search.".to_string(),
            )
        } else {
            (
                "No chats yet".to_string(),
                "Scan a QR or paste an invitation to start a 1:1 chat.".to_string(),
            )
        };
        script_eval!(cx, {
            mod.state.roster_empty_title = #(empty_title)
            mod.state.roster_empty_body = #(empty_body)
        });
    }

    fn push_settings_pane(&self, cx: &mut Cx) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let blurb = "QR and invitation links encode your identity public key — not IP addresses. After someone adds you, the app finds you via coordination, same Wi‑Fi, or when you connect to them.".to_string();
        let detail = format!(
            "Namespace: {}\nIdentity (share this):\n{}",
            session.app_namespace, session.public_key_hex
        );
        let self_line = format!("You  {}", short_key(&session.public_key_hex));
        let display = host::local_display_name(&session.app_namespace);
        script_eval!(cx, {
            mod.state.identity_blurb = #(blurb)
            mod.state.identity_detail = #(detail)
            mod.state.self_line = #(self_line)
            ui.display_name_input.set_text(#(display))
        });
    }

    fn apply_shell_widths(&mut self, cx: &mut Cx, window_w: f64) {
        self.window_w = window_w;
        self.apply_shell_layout(cx);
    }

    fn submit_send(&mut self, cx: &mut Cx) {
        // Prefer live widget text — self.composer can lag behind Android IME.
        let typed = self.ui.text_input(cx, ids!(composer)).text();
        if !typed.is_empty() {
            self.composer = typed;
        }
        let peer = self.open_peer.clone();
        if peer.is_empty() {
            script_eval!(cx, {
                mod.state.transcript = "Select a chat first"
                ui.chat_log.render()
            });
            return;
        }
        let text = self.composer.trim().to_string();
        if text.is_empty() {
            return;
        }
        match host::send_text(&peer, &text) {
            Ok(()) => {
                self.composer.clear();
                script_eval!(cx, { ui.composer.set_text("") });
                self.delivery_hint = host::delivery_offline_hint().unwrap_or_default();
                self.refresh_transcript(cx);
                self.refresh_call_line(cx);
                // Own send always lands on the new line (Flutter `force: true`).
                self.pin_chat_bottom(cx);
            }
            Err(e) => {
                script_eval!(cx, {
                    mod.state.transcript = #(e)
                    ui.chat_log.render()
                });
            }
        }
    }

    fn drain_events(&mut self, cx: &mut Cx) {
        let mut dirty = false;
        let mut wake_call = false;
        let mut wake_unlock = false;
        while let Some(ev) = host::poll_event() {
            match ev.get("kind").and_then(|v| v.as_str()) {
                Some("incoming_call_wake") => wake_call = true,
                Some("unlock_wake") => wake_unlock = true,
                _ => dirty = true,
            }
        }
        if wake_unlock && self.session.is_none() {
            script_eval!(cx, {
                mod.state.screen = "identity"
                ui.identity_view.set_visible(true)
                ui.hub_view.set_visible(false)
                ui.call_view.set_visible(false)
                ui.identity_copy.render()
            });
            self.set_screen(cx, "identity");
        }
        if self.session.is_none() {
            return;
        }
        if wake_call {
            self.refresh_call_line(cx);
        }
        if dirty {
            self.refresh_call_line(cx);
        }
        if self.page.is_empty() {
            // Roster only when contacts_version moved (refresh_roster no-ops otherwise).
            self.refresh_roster(cx);
            if !self.open_peer.is_empty() {
                // Transcript only when the open room's in-memory revision moved —
                // never full re-render every poll tick (that fought IME / scroll).
                let rev = host::transcript_revision(&self.namespace, &self.open_peer);
                if rev != self.chat_rev {
                    self.refresh_transcript(cx);
                }
                self.maybe_autoload_older(cx);
            }
        }
        if self.recording_ui || host::voice_note_recording() {
            self.sync_recording_bar(cx);
        }
        self.refresh_video(cx);
        self.refresh_qr_preview(cx);
        self.take_qr(cx);
    }

    /// Flutter scroll-up: when the user reaches the top of `chat_log` and the
    /// store still has older pages, load the next page without yanking the
    /// viewport (stick-to-bottom stays off because `grew` near-bottom is false).
    fn maybe_autoload_older(&mut self, cx: &mut Cx) {
        if !self.has_more || self.open_peer.is_empty() {
            return;
        }
        let log = self.ui.view(cx, ids!(chat_log));
        let log_rect = log.area().rect(cx);
        let Some(inner) = log.borrow_mut() else {
            return;
        };
        // Skip call_label (child 0); the first message row is usually child 1.
        let Some((_, first)) = inner.children.get(1).or_else(|| inner.children.first()) else {
            return;
        };
        let first_rect = first.area().rect(cx);
        // At top when the first message sits at/above the visible top edge.
        if first_rect.pos.y < log_rect.pos.y - 8.0 {
            return;
        }
        drop(inner);
        let prev_limit = self.transcript_limit();
        self.window_limit = prev_limit.saturating_add(50).max(50);
        if self.window_limit == prev_limit {
            return;
        }
        self.refresh_transcript(cx);
    }

    fn refresh_video(&mut self, cx: &mut Cx) {
        if !self.call_open {
            return;
        }
        let show_video = host::call_shows_video();
        self.ui
            .widget(cx, ids!(video_row))
            .set_visible(cx, show_video);
        self.ui
            .widget(cx, ids!(call_peer_block))
            .set_visible(cx, !show_video);
        if !show_video {
            return;
        }
        let (local, remote) = host::call_picture_pngs();
        if let Some(png) = local {
            let _ = self
                .ui
                .image(cx, ids!(local_video))
                .load_png_from_data(cx, &png);
        }
        if let Some(png) = remote {
            let _ = self
                .ui
                .image(cx, ids!(remote_video))
                .load_png_from_data(cx, &png);
        }
    }

    fn end_qr_scan_ui(&mut self) {
        host::set_qr_scan_ui_active(false);
        host::stop_qr_scan();
    }

    fn refresh_qr_preview(&mut self, cx: &mut Cx) {
        if self.page != "scan" {
            return;
        }
        let Some((w, h, rgba)) = host::take_qr_preview_rgba() else {
            return;
        };
        // Live path: raw BGRA texture, no PNG, no mipmaps. `load_png_from_data` on
        // Linux builds a mip chain per frame and skips redraw outside draw events —
        // that is why the camera log showed ~30fps while the UI looked frozen.
        let pixels = (w as usize).saturating_mul(h as usize);
        let mut data = Vec::with_capacity(pixels);
        for px in rgba.chunks_exact(4).take(pixels) {
            data.push(
                ((px[3] as u32) << 24)
                    | ((px[0] as u32) << 16)
                    | ((px[1] as u32) << 8)
                    | (px[2] as u32),
            );
        }
        let texture = Texture::new_with_format(
            cx,
            TextureFormat::VecBGRAu8_32 {
                width: w as usize,
                height: h as usize,
                data: Some(data),
                updated: TextureUpdated::Full,
            },
        );
        let image = self.ui.image(cx, ids!(scan_preview));
        image.set_texture(cx, Some(texture));
        // set_texture only auto-redraws inside a draw event; Signal polls need this.
        self.ui.widget(cx, ids!(scan_preview)).redraw(cx);
    }

    fn take_qr(&mut self, cx: &mut Cx) {
        let Some(result) = host::take_qr_scan() else {
            return;
        };
        let ns = self.namespace.clone();
        match result.and_then(|text| host::accept_invite(&ns, &text)) {
            Ok(pk) => {
                host::set_qr_scan_ui_active(false);
                let line = format!("Joined {}", short_key(&pk));
                script_eval!(cx, {
                    mod.state.sheet_body = #(line)
                    ui.sheet_copy.render()
                });
                self.open_peer = pk.clone();
                self.refresh_roster(cx);
                let title = self
                    .session
                    .as_ref()
                    .and_then(|_| host::list_roster(&ns).ok())
                    .and_then(|rows| {
                        rows.into_iter()
                            .find(|r| r.public_key_hex.eq_ignore_ascii_case(&pk))
                            .map(|r| r.title)
                    })
                    .unwrap_or_else(|| short_key(&pk));
                // Leave the scan sheet and open the new chat.
                self.close_sheet(cx);
                script_eval!(cx, {
                    mod.state.selected_pk = #(pk)
                    mod.state.selected_title = #(title)
                    mod.state.open_gen = mod.state.open_gen + 1
                    ui.chat_log.render()
                });
            }
            Err(e) if e == "Scan stopped" => {
                // Back / leaving the sheet — not an error for the user.
            }
            Err(e) => {
                // Camera already stopped on Ready; offer Scan again on the sheet.
                if self.page == "scan" {
                    script_eval!(cx, {
                        mod.state.sheet_body = #(e.clone())
                        ui.sheet_copy.render()
                    });
                    self.sync_sheet_body_text(cx, &e);
                } else {
                    script_eval!(cx, {
                        mod.state.call_line = #(e)
                        ui.chat_log.render()
                    });
                }
            }
        }
    }

    fn refresh_call_line(&mut self, cx: &mut Cx) {
        let call = host::call_banner();
        host::note_call_banner(&call);
        let calling = !call.is_empty();
        if host::delivery_offline_hint().is_none() {
            self.delivery_hint.clear();
        }
        if let Some(qw) = host::take_delivery_quota_warning() {
            self.delivery_hint = qw;
        }
        let line = if !call.is_empty() {
            call
        } else {
            self.delivery_hint.clone()
        };
        // Accept is callee-only. Showing it on an outbound dial made the caller
        // think the app was asking them to accept their own call.
        let incoming = host::call_is_incoming_ringing();
        self.ui
            .widget(cx, ids!(accept_call_button))
            .set_visible(cx, calling && incoming);
        let end_label = if incoming { "Decline" } else { "End call" };
        let peer_label = host::call_peer_label();
        let initial = call_avatar_initial(&peer_label);
        script_eval!(cx, {
            mod.state.call_line = #(line)
            ui.end_call_button.set_text(#(end_label))
            ui.call_peer_name.set_text(#(peer_label))
            ui.call_initial.set_text(#(initial))
            ui.call_copy.render()
            ui.chat_log.render()
        });
        if calling {
            self.refresh_video(cx);
        }
        if calling == self.call_open {
            if calling {
                self.ui.widget(cx, ids!(call_view)).redraw(cx);
            }
            return;
        }
        self.call_open = calling;
        if calling {
            script_eval!(cx, {
                ui.identity_view.set_visible(false)
                ui.hub_view.set_visible(false)
                ui.sheet_view.set_visible(false)
                ui.call_view.set_visible(true)
                ui.call_copy.render()
            });
            self.set_screen(cx, "call");
            self.refresh_video(cx);
        } else if self.session.is_some() && !self.ui_locked {
            self.ui
                .widget(cx, ids!(video_row))
                .set_visible(cx, false);
            if self.page.is_empty() {
                script_eval!(cx, {
                    ui.call_view.set_visible(false)
                    ui.sheet_view.set_visible(false)
                    ui.hub_view.set_visible(true)
                });
                self.set_screen(cx, "hub");
            } else {
                script_eval!(cx, {
                    ui.call_view.set_visible(false)
                    ui.hub_view.set_visible(false)
                    ui.sheet_view.set_visible(true)
                    ui.sheet_copy.render()
                });
                self.set_screen(cx, "sheet");
            }
        } else {
            script_eval!(cx, {
                ui.call_view.set_visible(false)
                ui.identity_view.set_visible(true)
            });
            self.set_screen(cx, "identity");
        }
    }
}

impl MatchEvent for App {
    fn handle_back_pressed(&mut self, cx: &mut Cx) -> bool {
        self.navigate_back(cx, true)
    }

    fn handle_actions(&mut self, cx: &mut Cx, actions: &Actions) {
        #[cfg(target_os = "android")]
        for action in actions {
            if let Some(answer) = android_pick::answer(action) {
                self.on_android_pick(cx, answer);
            }
        }
        self.sync_algorithm(cx);
        if let Some(text) = self.ui.text_input(cx, ids!(password_input)).changed(actions) {
            self.password = text;
        }
        if let Some(text) = self.ui.text_input(cx, ids!(composer)).changed(actions) {
            self.composer = text;
        }
        let unlock = self.ui.button(cx, ids!(unlock_button)).clicked(actions)
            || self
                .ui
                .text_input(cx, ids!(password_input))
                .returned(actions)
                .is_some();
        if unlock {
            if let Some((text, _)) = self
                .ui
                .text_input(cx, ids!(password_input))
                .returned(actions)
            {
                self.password = text;
            }
            self.submit_unlock(cx);
        }
        if let Some(text) = self.ui.text_input(cx, ids!(secret_input)).changed(actions) {
            self.secret = text;
        }
        if self.ui.button(cx, ids!(password_eye)).clicked(actions) {
            self.toggle_secret_field(cx, ids!(password_input), ids!(password_eye));
        }
        if self.ui.button(cx, ids!(secret_eye)).clicked(actions) {
            self.toggle_secret_field(cx, ids!(secret_input), ids!(secret_eye));
        }
        if self.ui.button(cx, ids!(paste_eye)).clicked(actions) {
            self.toggle_secret_field(cx, ids!(paste_input), ids!(paste_eye));
        }
        if self.ui.button(cx, ids!(pk_eye)).clicked(actions) {
            self.toggle_secret_field(cx, ids!(pk_input), ids!(pk_eye));
        }
        if self.ui.button(cx, ids!(alias_eye)).clicked(actions) {
            self.toggle_secret_field(cx, ids!(alias_input), ids!(alias_eye));
        }
        if let Some(text) = self.ui.text_input(cx, ids!(search_input)).changed(actions) {
            self.search = text;
            if self.page.is_empty() && self.session.is_some() {
                self.refresh_roster(cx);
            }
        }
        if let Some(text) = self.ui.text_input(cx, ids!(paste_input)).changed(actions) {
            self.paste = text;
        }
        if let Some(text) = self.ui.text_input(cx, ids!(pk_input)).changed(actions) {
            self.add_pk = text;
        }
        if let Some(text) = self.ui.text_input(cx, ids!(alias_input)).changed(actions) {
            self.add_alias = text;
        }
        if self.ui.button(cx, ids!(mode_button)).clicked(actions) {
            self.create_new = !self.create_new;
            self.push_identity_copy(cx);
        }
        if self.ui.button(cx, ids!(algo_secp)).clicked(actions) {
            self.algorithm = "secp256k1".into();
            self.mark_algorithm(cx);
        }
        if self.ui.button(cx, ids!(algo_ed)).clicked(actions) && !self.create_new {
            self.algorithm = "ed25519".into();
            self.mark_algorithm(cx);
        }
        if self.ui.button(cx, ids!(algo_p256)).clicked(actions) && !self.create_new {
            self.algorithm = "ecdsa-p256".into();
            self.mark_algorithm(cx);
        }
        if self.ui.button(cx, ids!(delete_button)).clicked(actions) {
            self.pull_password(cx);
            match host::delete_identity(&self.namespace, &self.password) {
                Ok(()) => {
                    self.session = None;
                    self.password.clear();
                    self.create_new = true;
                    script_eval!(cx, {
                        mod.state.status = "Identity deleted from this device"
                        ui.identity_copy.render()
                    });
                    self.push_identity_copy(cx);
                }
                Err(e) => {
                    script_eval!(cx, {
                        mod.state.status = #(e)
                        ui.identity_copy.render()
                    });
                }
            }
        }
        if self.ui.button(cx, ids!(import_backup_button)).clicked(actions) {
            self.pull_password(cx);
            #[cfg(target_os = "android")]
            {
                if self.password.is_empty() {
                    // Shows the "enter your password" hint.
                    self.open_import_backup_sheet(cx);
                } else {
                    let title = "Choose your keystore backup";
                    android_pick::open(cx, android_pick::BACKUP, title, None);
                }
            }
            #[cfg(not(target_os = "android"))]
            {
                match self.import_backup() {
                    Ok(session) => self.finish_import_backup(cx, session),
                    Err(e) => {
                        script_eval!(cx, {
                            mod.state.status = #(e)
                            ui.identity_copy.render()
                        });
                    }
                }
            }
        }
        if self.ui.button(cx, ids!(new_chat_button)).clicked(actions) {
            self.set_hub_tab(cx, 0);
            self.open_page(cx, "add");
        }
        if self.ui.button(cx, ids!(tab_chats)).clicked(actions)
            || self.ui.button(cx, ids!(tab_chats_on)).clicked(actions)
            || self.ui.button(cx, ids!(bottom_chats)).clicked(actions)
            || self.ui.button(cx, ids!(bottom_chats_on)).clicked(actions)
        {
            self.set_hub_tab(cx, 0);
        }
        if self.ui.button(cx, ids!(tab_contacts)).clicked(actions)
            || self.ui.button(cx, ids!(tab_contacts_on)).clicked(actions)
            || self.ui.button(cx, ids!(bottom_contacts)).clicked(actions)
            || self.ui.button(cx, ids!(bottom_contacts_on)).clicked(actions)
        {
            self.set_hub_tab(cx, 1);
        }
        if self.ui.button(cx, ids!(tab_invite)).clicked(actions)
            || self.ui.button(cx, ids!(tab_invite_on)).clicked(actions)
            || self.ui.button(cx, ids!(bottom_invite)).clicked(actions)
            || self.ui.button(cx, ids!(bottom_invite_on)).clicked(actions)
        {
            self.set_hub_tab(cx, 2);
        }
        if self.ui.button(cx, ids!(tab_settings)).clicked(actions)
            || self.ui.button(cx, ids!(tab_settings_on)).clicked(actions)
            || self.ui.button(cx, ids!(bottom_settings)).clicked(actions)
            || self.ui.button(cx, ids!(bottom_settings_on)).clicked(actions)
        {
            self.push_settings_pane(cx);
            self.set_hub_tab(cx, 3);
        }
        if self.ui.button(cx, ids!(chat_back_button)).clicked(actions) {
            let _ = self.navigate_back(cx, false);
        }
        if self.ui.button(cx, ids!(invite_button)).clicked(actions) {
            self.open_page(cx, "invite");
        }
        if self.ui.button(cx, ids!(invite_button_id)).clicked(actions) {
            self.open_page(cx, "invite");
        }
        if self.ui.button(cx, ids!(invite_copy_button)).clicked(actions) {
            if let Some(session) = self.session.clone() {
                let alias = host::local_display_name(&self.namespace);
                let msg = match host::invite_links(
                    &session.public_key_hex,
                    if alias.is_empty() {
                        None
                    } else {
                        Some(alias.as_str())
                    },
                ) {
                    Ok((https, _)) => match copy_to_clipboard(cx, &https) {
                        Ok(()) => {
                            "Invitation copied. Paste it on the other device under Join.".to_string()
                        }
                        Err(e) => e,
                    },
                    Err(e) => e,
                };
                script_eval!(cx, { mod.state.identity_blurb = #(msg) });
            }
        }
        if self.ui.button(cx, ids!(key_copy_button)).clicked(actions) {
            if let Some(session) = self.session.clone() {
                let msg = match copy_to_clipboard(cx, &session.public_key_hex) {
                    Ok(()) => "Public key copied".to_string(),
                    Err(e) => e,
                };
                script_eval!(cx, { mod.state.identity_blurb = #(msg) });
            }
        }
        if self.ui.button(cx, ids!(join_button)).clicked(actions)
            || self.ui.button(cx, ids!(join_nav_button)).clicked(actions)
        {
            self.open_page(cx, "join");
        }
        if self.ui.button(cx, ids!(add_contact_button)).clicked(actions) {
            self.open_page(cx, "add");
        }
        if self.ui.button(cx, ids!(delivery_nav_button)).clicked(actions) {
            self.open_page(cx, "delivery");
        }
        if self.ui.button(cx, ids!(scan_button)).clicked(actions)
            || self.ui.button(cx, ids!(scan_nav_button)).clicked(actions)
        {
            self.open_page(cx, "scan");
            let msg = match host::start_qr_scan() {
                Ok(()) => "Point the camera at an invitation QR…".to_string(),
                Err(e) => e,
            };
            script_eval!(cx, {
                mod.state.sheet_body = #(msg.clone())
                ui.sheet_copy.render()
            });
            self.sync_sheet_body_text(cx, &msg);
        }
        if self.ui.button(cx, ids!(scan_file_button)).clicked(actions) {
            #[cfg(target_os = "android")]
            android_pick::open(
                cx,
                android_pick::QR_PICTURE,
                "Choose a QR picture",
                Some(("Pictures", &["png", "jpg", "jpeg"])),
            );
            #[cfg(not(target_os = "android"))]
            match rfd::FileDialog::new()
                .add_filter("Pictures", &["png", "jpg", "jpeg"])
                .pick_file()
            {
                Some(path) => self.join_from_qr_picture(cx, &path.display().to_string()),
                None => self.show_call_line(cx, "No picture chosen".into()),
            }
        }
        if self.ui.button(cx, ids!(contacts_button)).clicked(actions) {
            self.open_page(cx, "contacts");
        }
        if self.ui.button(cx, ids!(blocked_button)).clicked(actions) {
            self.open_page(cx, "blocked");
        }
        if self.ui.button(cx, ids!(about_button)).clicked(actions) {
            self.open_page(cx, "about");
        }
        if self.ui.button(cx, ids!(back_button)).clicked(actions) {
            let _ = self.navigate_back(cx, false);
        }
        if self.ui.button(cx, ids!(sheet_action)).clicked(actions) {
            self.sheet_go(cx);
        }
        if self.ui.button(cx, ids!(delivery_button)).clicked(actions) {
            self.open_page(cx, "delivery");
        }
        if self.ui.button(cx, ids!(log_button)).clicked(actions) {
            self.open_page(cx, "log");
        }
        if self.ui.button(cx, ids!(video_button)).clicked(actions) {
            let peer = self.open_peer.clone();
            if peer.is_empty() {
                script_eval!(cx, {
                    mod.state.call_line = "Select a chat first"
                    ui.chat_log.render()
                });
            } else if let Err(e) = host::start_video_call(&peer) {
                script_eval!(cx, {
                    mod.state.call_line = #(e)
                    ui.chat_log.render()
                });
            } else {
                // Open call chrome immediately — do not wait for a later poll event.
                self.refresh_call_line(cx);
            }
        }
        if self.ui.button(cx, ids!(mute_button)).clicked(actions) {
            self.mic_muted = !self.mic_muted;
            let label = if self.mic_muted { "Unmute" } else { "Mute" };
            script_eval!(cx, { ui.mute_button.set_text(#(label)) });
            let muted = self.mic_muted;
            let msg = match host::set_mic_muted(muted) {
                Ok(()) => {
                    if muted {
                        "Microphone off".into()
                    } else {
                        "Microphone on".into()
                    }
                }
                Err(e) => e,
            };
            script_eval!(cx, {
                mod.state.call_line = #(msg)
                ui.chat_log.render()
            });
        }
        if self.ui.button(cx, ids!(speaker_button)).clicked(actions) {
            self.speaker_on = !self.speaker_on;
            let on = self.speaker_on;
            let label = if on {
                i18n::phrase(&self.lang, "speaker")
            } else {
                i18n::phrase(&self.lang, "speaker_off")
            };
            let msg = match host::set_speaker(on) {
                Ok(()) => label.clone(),
                Err(e) => e,
            };
            script_eval!(cx, {
                ui.speaker_button.set_text(#(label))
                mod.state.call_line = #(msg)
                ui.call_copy.render()
            });
        }
        if self.ui.button(cx, ids!(flip_camera_button)).clicked(actions) {
            let msg = match host::flip_call_camera() {
                Ok(()) => i18n::phrase(&self.lang, "flip_camera").to_string(),
                Err(e) => e,
            };
            script_eval!(cx, {
                mod.state.call_line = #(msg)
                ui.call_copy.render()
            });
        }
        if self.ui.button(cx, ids!(call_button)).clicked(actions) {
            let peer = self.open_peer.clone();
            if peer.is_empty() {
                script_eval!(cx, {
                    mod.state.call_line = "Select a chat first"
                    ui.chat_log.render()
                });
            } else if let Err(e) = host::start_voice_call(&peer) {
                script_eval!(cx, {
                    mod.state.call_line = #(e)
                    ui.chat_log.render()
                });
            } else {
                self.refresh_call_line(cx);
            }
        }
        if self.ui.button(cx, ids!(accept_call_button)).clicked(actions) {
            if let Err(e) = host::accept_incoming_call() {
                script_eval!(cx, {
                    mod.state.call_line = #(e)
                    ui.chat_log.render()
                });
            } else {
                self.refresh_call_line(cx);
            }
        }
        if self.ui.button(cx, ids!(end_call_button)).clicked(actions) {
            if let Err(e) = host::end_call() {
                script_eval!(cx, {
                    mod.state.call_line = #(e)
                    ui.chat_log.render()
                });
            } else {
                self.refresh_call_line(cx);
            }
        }
        if self.ui.button(cx, ids!(status_available)).clicked(actions) {
            self.apply_status_preset(cx, "Available");
        }
        if self.ui.button(cx, ids!(status_busy)).clicked(actions) {
            self.apply_status_preset(cx, "Busy");
        }
        if self.ui.button(cx, ids!(status_away)).clicked(actions) {
            self.apply_status_preset(cx, "Away");
        }
        if self.ui.button(cx, ids!(status_call)).clicked(actions) {
            self.apply_status_preset(cx, "In a call");
        }
        if self.ui.button(cx, ids!(status_clear)).clicked(actions) {
            self.apply_status_preset(cx, "");
        }
        if self.ui.button(cx, ids!(trust_add_button)).clicked(actions)
            || self.ui.button(cx, ids!(menu_add_button)).clicked(actions)
        {
            self.set_chat_menu_open(cx, false);
            let peer = self.open_peer.clone();
            let msg = if peer.is_empty() {
                "Select a chat first".into()
            } else {
                match host::mark_known(&self.namespace, &peer) {
                    Ok(()) => "Added".into(),
                    Err(e) => e,
                }
            };
            script_eval!(cx, {
                mod.state.call_line = #(msg)
                ui.chat_log.render()
            });
            self.roster_ver = u64::MAX;
            self.refresh_roster(cx);
        }
        if self.ui.button(cx, ids!(trust_block_button)).clicked(actions)
            || self.ui.button(cx, ids!(menu_block_button)).clicked(actions)
        {
            self.set_chat_menu_open(cx, false);
            let peer = self.open_peer.clone();
            if !peer.is_empty() {
                let _ = host::set_blocked(&self.namespace, &peer, true);
                self.close_open_room(cx);
            }
        }
        if self.ui.button(cx, ids!(chat_menu_button)).clicked(actions) {
            let open = !self.chat_menu_open;
            self.set_chat_menu_open(cx, open);
        }
        if self.ui.button(cx, ids!(menu_call_button)).clicked(actions) {
            self.set_chat_menu_open(cx, false);
            let peer = self.open_peer.clone();
            if peer.is_empty() {
                script_eval!(cx, {
                    mod.state.call_line = "Select a chat first"
                    ui.chat_log.render()
                });
            } else if let Err(e) = host::start_voice_call(&peer) {
                script_eval!(cx, {
                    mod.state.call_line = #(e)
                    ui.chat_log.render()
                });
            } else {
                self.refresh_call_line(cx);
            }
        }
        if self.ui.button(cx, ids!(menu_video_button)).clicked(actions) {
            self.set_chat_menu_open(cx, false);
            let peer = self.open_peer.clone();
            if peer.is_empty() {
                script_eval!(cx, {
                    mod.state.call_line = "Select a chat first"
                    ui.chat_log.render()
                });
            } else if let Err(e) = host::start_video_call(&peer) {
                script_eval!(cx, {
                    mod.state.call_line = #(e)
                    ui.chat_log.render()
                });
            } else {
                self.refresh_call_line(cx);
            }
        }
        if self.ui.button(cx, ids!(menu_share_button)).clicked(actions) {
            self.set_chat_menu_open(cx, false);
            self.open_page(cx, "invite");
        }
        if self.ui.button(cx, ids!(menu_join_button)).clicked(actions) {
            self.set_chat_menu_open(cx, false);
            self.open_page(cx, "join");
        }
        if self.ui.button(cx, ids!(menu_lock_button)).clicked(actions)
            || self.ui.button(cx, ids!(hub_lock_button)).clicked(actions)
        {
            self.set_chat_menu_open(cx, false);
            self.submit_lock(cx);
        }
        if self.ui.button(cx, ids!(record_discard_button)).clicked(actions) {
            self.cancel_voice_note(cx);
        }
        if self.ui.button(cx, ids!(record_send_button)).clicked(actions)
            || self.ui.button(cx, ids!(record_send_button)).pressed(actions)
        {
            // Stop + send (same as a second mic tap).
            self.toggle_voice_note(cx);
        }
        if self.ui.button(cx, ids!(choose_file_button)).clicked(actions) {
            let peer = self.open_peer.clone();
            if peer.is_empty() {
                self.show_call_line(cx, "Select a chat first".into());
            } else {
                #[cfg(target_os = "android")]
                android_pick::pick_attachment(cx, &peer);
                #[cfg(not(target_os = "android"))]
                if let Some(path) = rfd::FileDialog::new().pick_file() {
                    let sent = host::send_attachment(&peer, &path.display().to_string());
                    self.show_attach_result(cx, sent);
                }
            }
        }
        if self.ui.button(cx, ids!(key_button)).clicked(actions) {
            self.open_page(cx, "key");
        }
        if self.ui.button(cx, ids!(backup_button)).clicked(actions) {
            self.open_page(cx, "backup");
        }
        if self.ui.button(cx, ids!(password_button)).clicked(actions) {
            self.open_page(cx, "password");
        }
        if self.ui.button(cx, ids!(save_display_name_button)).clicked(actions) {
            let name = self.ui.text_input(cx, ids!(display_name_input)).text();
            let msg = match host::set_local_display_name(&self.namespace, &name) {
                Ok(()) => {
                    if name.trim().is_empty() {
                        "Display name cleared".to_string()
                    } else {
                        "Display name saved — new invitations include it".to_string()
                    }
                }
                Err(e) => e,
            };
            script_eval!(cx, { mod.state.identity_blurb = #(msg) });
        }
        if self.ui.button(cx, ids!(lock_button)).clicked(actions) {
            self.submit_lock(cx);
        }
        if self.ui.button(cx, ids!(logout_button)).clicked(actions) {
            self.submit_logout(cx);
        }
        if self.ui.button(cx, ids!(language_button)).clicked(actions) {
            self.open_languages(cx);
        }
        if self.ui.button(cx, ids!(older_button)).clicked(actions) {
            if self.has_more {
                self.window_limit = self.transcript_limit().saturating_add(50);
                self.refresh_transcript(cx);
            }
        }
        if self.ui.button(cx, ids!(record_button)).clicked(actions) {
            self.toggle_voice_note(cx);
        }
        // Send on Pressed (finger-down), not Clicked: with the IME up, Android
        // reflow can move the button before FingerUp so Clicked never fires.
        // IconGo keeps grab_key_focus false so this press does not dismiss IME
        // and wipe the composer. Soft-keyboard Send / desktop Enter use Returned.
        let send = self.ui.button(cx, ids!(send_button)).pressed(actions)
            || self
                .ui
                .text_input(cx, ids!(composer))
                .returned(actions)
                .is_some();
        if send {
            if let Some((text, _)) = self.ui.text_input(cx, ids!(composer)).returned(actions) {
                self.composer = text;
            }
            self.submit_send(cx);
        }
        self.sync_open_chat(cx);
        self.sync_play(cx);
        self.sync_language(cx);
    }
}

impl AppMain for App {
    fn script_mod(vm: &mut ScriptVm) -> ScriptValue {
        crate::makepad_widgets::script_mod(vm);
        // Before the first namespace read: `GHALBOL_APP_ID` picks the data directory.
        load_env_once();
        if !vm.is_reload() {
            let ns = host::default_app_namespace();
            let exists = host::keystore_exists(&ns);
            let primary = if exists {
                "Unlock"
            } else {
                "Create identity"
            };
            let hint = if exists {
                "Enter your password to unlock this device."
            } else {
                "Choose a password. This creates a device identity on this computer."
            };
            script_eval!(vm, {
                mod.state = {
                    screen: "identity"
                    status: ""
                    hint: #(hint)
                    primary: #(primary)
                    self_line: ""
                    roster: []
                    roster_empty_title: "No chats yet"
                    roster_empty_body: "Scan a QR or paste an invitation to start a 1:1 chat."
                    transcript: ""
                    lines: []
                    selected_pk: ""
                    selected_title: "Select a chat"
                    call_line: ""
                    open_gen: 0
                    play_gen: 0
                    play_path: ""
                    play_action: ""
                    lang_gen: 0
                    algo: "secp256k1"
                    algo_line: "Algorithm: secp256k1"
                    algo_gen: 0
                    identity_blurb: ""
                    identity_detail: ""
                    languages: []
                }
                true
            });
        }
        self::script_mod(vm)
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        if self.namespace.is_empty() {
            self.namespace = host::default_app_namespace();
            self.create_new = true;
            self.algorithm = "secp256k1".into();
            self.lang = host::ui_locale(&self.namespace).unwrap_or_else(i18n::system_code);
            self.speaker_on = true;
            self.roster_ver = u64::MAX;
            #[cfg(target_os = "android")]
            {
                let vm = makepad_android_state::get_java_vm();
                let activity = makepad_android_state::get_activity();
                if !vm.is_null() && !activity.is_null() {
                    host::install_android_context(vm.cast(), activity.cast());
                }
                let _ = cx.request_permission(makepad_platform::permission::Permission::Camera);
                let _ = cx.request_permission(makepad_platform::permission::Permission::AudioInput);
            }
            if self.pending_invite.is_empty() {
                if let Some(url) = host::take_view_invite() {
                    self.pending_invite = url;
                }
            }
            host::desktop_session_start(&self.namespace);
            if let Some(arg) = std::env::args().skip(1).find(|arg| {
                arg.starts_with("ghalbol://") || arg.contains("ghalbol.com/connect/")
            }) {
                self.pending_invite = arg;
            }
            start_ui_poll();
            self.push_identity_copy(cx);
        }
        match event {
            Event::Shutdown | Event::QuitRequested(_) => {
                let _ = host::end_call();
            }
            Event::WindowGeomChange(change) => {
                self.apply_shell_widths(cx, change.new_geom.inner_size.x);
            }
            // After IME reflow shrinks chat_log, pin to the latest messages so
            // they stay above the keyboard — never left scrolled under it.
            Event::VirtualKeyboard(VirtualKeyboardEvent::DidShow { .. })
            | Event::VirtualKeyboard(VirtualKeyboardEvent::WillShow { .. }) => {
                if !self.open_peer.is_empty() {
                    self.pin_chat_bottom(cx);
                }
            }
            // The soft keyboard went down while the composer still holds key
            // focus: that is the user dismissing it (back / IME down arrow).
            // A focused TextInput re-requests the IME on its next draw, so
            // release focus — the keyboard comes back only when the user taps
            // a field again. Skipped with a hardware keyboard, where Android
            // hides the soft keyboard on its own while typing continues.
            Event::VirtualKeyboard(VirtualKeyboardEvent::DidHide { .. }) => {
                if !cx.keyboard.has_physical_keyboard() {
                    let composer = self.ui.text_input(cx, ids!(composer));
                    if cx.has_key_focus(composer.area()) {
                        cx.set_key_focus(Area::Empty);
                    }
                }
            }
            Event::NextFrame(_) => self.apply_chat_pin_frame(cx),
            Event::MouseUp(mouse) if mouse.button.is_back() => {
                if self.navigate_back(cx, true) {
                    return;
                }
            }
            Event::MouseUp(mouse) if mouse.button.is_forward() => {
                if self.navigate_forward(cx) {
                    return;
                }
            }
            Event::KeyDown(key)
                if key.key_code == KeyCode::Escape
                    || (key.key_code == KeyCode::ArrowLeft && key.modifiers.alt) =>
            {
                // Escape / Alt+Left only. Android KEYCODE_BACK is Event::BackPressed
                // (MatchEvent::handle_back_pressed) — do not also handle KeyCode::Back here
                // or every gesture would pop two levels.
                if self.navigate_back(cx, false) {
                    return;
                }
            }
            Event::KeyDown(key)
                if key.key_code == KeyCode::ArrowRight && key.modifiers.alt =>
            {
                if self.navigate_forward(cx) {
                    return;
                }
            }
            _ => {}
        }
        #[cfg(target_os = "android")]
        match event {
            Event::Background => host::set_app_visible(false),
            Event::Foreground | Event::Resume => {
                host::set_app_visible(true);
                if let Some(url) = host::take_view_invite() {
                    if self.session.is_some() {
                        let _ = host::accept_invite(&self.namespace, &url);
                        self.roster_ver = u64::MAX;
                    } else {
                        self.pending_invite = url;
                    }
                }
            }
            _ => {}
        }
        self.match_event(cx, event);
        self.ui.handle_event(cx, event, &mut Scope::empty());
        // `roster_list.on_item_tap` bumps `open_gen` during the FingerUp above;
        // that path does not always emit a WidgetAction, so open the room here
        // rather than waiting for a later Event::Actions.
        self.sync_open_chat(cx);
        if let Event::Signal = event {
            self.drain_events(cx);
        }
    }
}

/// Avatar glyph for a roster row: first letter of the display name, upper-cased;
/// `#` when the title is only a key fragment / punctuation (Flutter `_avatarInitial`).
fn roster_initial(row: &RosterEntry) -> String {
    call_avatar_initial(&row.title)
}

fn call_avatar_initial(title: &str) -> String {
    let title = title.trim();
    // `short_key` titles look like `0279be66…1798` — do not use a hex digit as the avatar.
    if title.contains('…') || title.contains("...") {
        return "#".to_string();
    }
    title
        .chars()
        .find(|c| c.is_alphabetic())
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_else(|| "#".to_string())
}

fn roster_matches(row: &RosterEntry, search: &str) -> bool {
    let q = search.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    row.title.to_lowercase().contains(&q)
        || row.public_key_hex.to_lowercase().contains(&q)
        || row.preview.to_lowercase().contains(&q)
}

struct Bubble {
    body: String,
    /// Secondary line under the body (size / Downloaded / Downloading…).
    detail: String,
    /// Flutter `_bubbleSenderCaption`: "You" or the peer title.
    caption: String,
    tick: String,
    read: bool,
    /// Outbound send gave up (`delivery == "failed"`) — orange `!` tick.
    failed: bool,
}

fn format_bytes(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1} MB", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1} KB", n as f64 / 1_000.0)
    } else if n > 0 {
        format!("{n} B")
    } else {
        String::new()
    }
}

fn format_mmss(ms: u32) -> String {
    let secs = ms / 1000;
    format!("{}:{:02}", secs / 60, secs % 60)
}

fn bubble_of(line: &ChatLine, peer_title: &str) -> Bubble {
    let caption = if line.outgoing {
        "You".to_string()
    } else if peer_title.is_empty() {
        "Contact".to_string()
    } else {
        peer_title.to_string()
    };
    let (body, detail) = if line.kind == "voice" {
        let secs = (line.duration_ms / 1000).max(1);
        (format!("▶  Voice note  {secs}s"), String::new())
    } else if line.kind == "attachment_offer" {
        let name = if line.text.is_empty() {
            "File".to_string()
        } else {
            line.text.clone()
        };
        let detail = if !line.local_path.is_empty() {
            "Downloaded".to_string()
        } else if line.delivery == "downloading" {
            "Downloading…".to_string()
        } else {
            let size = format_bytes(line.size_bytes);
            if size.is_empty() {
                "Tap Download".to_string()
            } else {
                size
            }
        };
        (name, detail)
    } else if line.kind == "text" || line.kind.is_empty() {
        (line.text.clone(), String::new())
    } else if line.text.is_empty() {
        (format!("[{}]", line.kind), String::new())
    } else {
        (line.text.clone(), String::new())
    };
    if !line.outgoing {
        return Bubble {
            body,
            detail,
            caption,
            tick: String::new(),
            read: false,
            failed: false,
        };
    }
    // Tick ladder (MAKEPAD_UI): pending / sent / delivered|read / failed.
    // FA PUA single glyphs — NotoSans lacks ✓ so "✓✓" became two wide FA
    // checks with a huge gap. check-double (U+F560) is one glyph.
    let read = line.delivery == "read";
    let failed = line.delivery == "failed";
    let tick = match line.delivery.as_str() {
        "pending" => "\u{f111}",       // FA circle
        "sent" => "\u{f00c}",          // FA check
        "delivered" | "read" => "\u{f560}", // FA check-double (one glyph)
        "downloaded" => "\u{f063}",    // FA arrow-down
        "failed" => "!",
        other if !other.is_empty() => other,
        _ => "",
    };
    Bubble {
        body,
        detail,
        caption,
        tick: tick.to_string(),
        read,
        failed,
    }
}

fn save_status_line(status: &str) -> Result<String, String> {
    host::set_availability_status(status).map(|()| {
        if status.is_empty() {
            "Status cleared".into()
        } else {
            format!("Status saved: {status}")
        }
    })
}

fn qr_png(text: &str) -> Option<Vec<u8>> {
    use ::image::ImageEncoder;
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    let img = code
        .render::<::image::Luma<u8>>()
        .quiet_zone(true)
        .min_dimensions(280, 280)
        .dark_color(::image::Luma([0]))
        .light_color(::image::Luma([255]))
        .build();
    let mut png = Vec::new();
    let encoder = ::image::codecs::png::PngEncoder::new(&mut png);
    encoder
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            ::image::ExtendedColorType::L8,
        )
        .ok()?;
    Some(png)
}

/// Dark placeholder so Scan never reuses leftover Invite QR pixels.
fn scan_placeholder_png() -> Option<Vec<u8>> {
    use ::image::ImageEncoder;
    let (w, h) = (320u32, 240u32);
    let pixels = vec![0x1Bu8; (w * h) as usize];
    let mut png = Vec::new();
    let encoder = ::image::codecs::png::PngEncoder::new(&mut png);
    encoder
        .write_image(&pixels, w, h, ::image::ExtendedColorType::L8)
        .ok()?;
    Some(png)
}

/// Android document picker (Storage Access Framework). Answers arrive later as a
/// `FileDialogAction` carrying a `content://` URI, which `host` reads for us.
#[cfg(target_os = "android")]
mod android_pick {
    use makepad_widgets::makepad_platform::file_dialogs::{FileDialog, FileDialogAction};
    use makepad_widgets::*;
    use std::sync::Mutex;

    pub const ATTACHMENT: LiveId = live_id!(pick_attachment);
    pub const BACKUP: LiveId = live_id!(pick_backup);
    pub const QR_PICTURE: LiveId = live_id!(pick_qr_picture);

    /// Chat that was open when Attach was tapped; the answer may arrive after a room switch.
    static ATTACH_PEER: Mutex<String> = Mutex::new(String::new());

    pub enum Answer {
        Picked(LiveId, String),
        Cancelled(LiveId),
    }

    pub fn open(cx: &mut Cx, id: LiveId, title: &str, filter: Option<(&str, &[&str])>) {
        let mut dialog = FileDialog::new().set_id(id).set_title(title.to_string());
        if let Some((name, extensions)) = filter {
            dialog = dialog.add_filter(
                name.to_string(),
                extensions.iter().map(|e| e.to_string()).collect(),
            );
        }
        cx.open_select_file_dialog(dialog);
    }

    pub fn pick_attachment(cx: &mut Cx, peer: &str) {
        if let Ok(mut g) = ATTACH_PEER.lock() {
            *g = peer.to_string();
        }
        open(cx, ATTACHMENT, "Send a file", None);
    }

    pub fn take_attach_peer() -> String {
        ATTACH_PEER
            .lock()
            .map(|mut g| std::mem::take(&mut *g))
            .unwrap_or_default()
    }

    pub fn answer(action: &Action) -> Option<Answer> {
        match action.downcast_ref::<FileDialogAction>()? {
            FileDialogAction::FileSelected { id, paths } => Some(Answer::Picked(
                *id,
                paths.first()?.to_string_lossy().into_owned(),
            )),
            FileDialogAction::FileCancelled { id } => Some(Answer::Cancelled(*id)),
            _ => None,
        }
    }
}

/// Linux tries the desktop clipboard tools first (the copy outlives the app), then
/// Makepad's own clipboard, which every other platform uses directly.
fn copy_to_clipboard(cx: &mut Cx, text: &str) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        for (bin, args) in [
            ("wl-copy", vec![] as Vec<&str>),
            ("xclip", vec!["-selection", "clipboard"]),
            ("xsel", vec!["--clipboard", "--input"]),
        ] {
            let Ok(mut child) = std::process::Command::new(bin)
                .args(&args)
                .stdin(std::process::Stdio::piped())
                .spawn()
            else {
                continue;
            };
            if let Some(mut stdin) = child.stdin.take() {
                use std::io::Write;
                let _ = stdin.write_all(text.as_bytes());
            }
            if child.wait().map(|s| s.success()).unwrap_or(false) {
                return Ok(());
            }
        }
    }
    cx.copy_to_clipboard(text);
    Ok(())
}

fn apply_algo_chip_style(cx: &mut Cx, button: &mut WidgetRef, on: bool) {
    if on {
        script_apply_eval!(cx, button, {
            draw_bg.color: #D6E8FF
            draw_bg.color_hover: #BFD9FF
            draw_bg.color_down: #A8CBFF
            draw_bg.color_focus: #D6E8FF
        });
    } else {
        script_apply_eval!(cx, button, {
            draw_bg.color: #FFFFFF
            draw_bg.color_hover: #DCE4F0
            draw_bg.color_down: #C5D0DE
            draw_bg.color_focus: #FFFFFF
        });
    }
    button.redraw(cx);
}

/// Makepad Button has no selected bit — only hover/down/focus. Selection is
/// Overlay idle+active faces painted from `hub_tab` (kumkaj lesson).
impl App {
    fn paint_nav_tabs(&mut self, cx: &mut Cx) {
        let tab = self.hub_tab;
        let pairs: [(&[LiveId], &[LiveId], bool); 4] = [
            (ids!(tab_chats), ids!(tab_chats_on), tab == 0),
            (ids!(tab_contacts), ids!(tab_contacts_on), tab == 1),
            (ids!(tab_invite), ids!(tab_invite_on), tab == 2),
            (ids!(tab_settings), ids!(tab_settings_on), tab == 3),
        ];
        let bottom: [(&[LiveId], &[LiveId], bool); 4] = [
            (ids!(bottom_chats), ids!(bottom_chats_on), tab == 0),
            (ids!(bottom_contacts), ids!(bottom_contacts_on), tab == 1),
            (ids!(bottom_invite), ids!(bottom_invite_on), tab == 2),
            (ids!(bottom_settings), ids!(bottom_settings_on), tab == 3),
        ];
        for (idle, on, selected) in pairs.into_iter().chain(bottom) {
            // Clear stuck hover/focus so a deselected face never stays lit.
            self.ui.button(cx, idle).reset_hover(cx);
            self.ui.button(cx, on).reset_hover(cx);
            self.ui.widget(cx, idle).set_visible(cx, !selected);
            self.ui.widget(cx, on).set_visible(cx, selected);
        }
        self.ensure_nav_icons(cx);
    }

    fn ensure_nav_icons(&mut self, cx: &mut Cx) {
        if self.nav_icons_loaded {
            return;
        }
        let marks: [(&[LiveId], &[u8]); 5] = [
            (
                ids!(mark_chats),
                include_bytes!("../resources/nav/chats.webp"),
            ),
            (
                ids!(mark_contacts),
                include_bytes!("../resources/nav/contacts.webp"),
            ),
            (
                ids!(mark_invite),
                include_bytes!("../resources/nav/invite.webp"),
            ),
            (
                ids!(mark_settings),
                include_bytes!("../resources/nav/settings.webp"),
            ),
            (ids!(mark_lock), include_bytes!("../resources/nav/lock.webp")),
        ];
        for (path, bytes) in marks {
            let img = self.ui.image(cx, path);
            let _ = img.load_image_from_data(cx, bytes);
        }
        self.nav_icons_loaded = true;
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
struct HubHistoryEntry {
    hub_tab: u8,
    narrow_show_room: bool,
    conversation_key: String,
    /// Sheet id (`invite`, `password`, `lang`, …). Empty = hub chrome only.
    page: String,
}

#[derive(Clone, Debug, Default)]
struct HubHistoryStack {
    entries: Vec<HubHistoryEntry>,
    /// Cursor into `entries` (browser-style; forward keeps truncated tail until a new navigate).
    index: usize,
}

impl HubHistoryStack {
    fn reset(&mut self, entry: HubHistoryEntry) {
        self.entries.clear();
        self.entries.push(entry);
        self.index = 0;
    }

    fn replace_top(&mut self, entry: HubHistoryEntry) {
        if self.entries.is_empty() {
            self.entries.push(entry);
            self.index = 0;
            return;
        }
        if self.index >= self.entries.len() {
            self.index = self.entries.len() - 1;
        }
        self.entries[self.index] = entry.clone();
        if self.index > 0 && self.entries[self.index - 1] == entry {
            self.entries.remove(self.index);
            self.index -= 1;
        }
    }

    fn record(&mut self, entry: HubHistoryEntry) {
        if self.entries.get(self.index) == Some(&entry) {
            return;
        }
        if self.entries.is_empty() {
            self.entries.push(entry);
            self.index = 0;
            return;
        }
        // New branch discards anything ahead of the cursor (browser navigate).
        self.entries.truncate(self.index + 1);
        self.entries.push(entry);
        self.index = self.entries.len() - 1;
        const MAX: usize = 64;
        while self.entries.len() > MAX {
            self.entries.remove(0);
            self.index = self.index.saturating_sub(1);
        }
    }

    fn go_back(&mut self) -> Option<HubHistoryEntry> {
        if self.index == 0 || self.entries.is_empty() {
            return None;
        }
        self.index -= 1;
        self.entries.get(self.index).cloned()
    }

    fn can_go_back(&self) -> bool {
        self.index > 0 && !self.entries.is_empty()
    }

    fn go_forward(&mut self) -> Option<HubHistoryEntry> {
        if self.index + 1 >= self.entries.len() {
            return None;
        }
        self.index += 1;
        self.entries.get(self.index).cloned()
    }
}

fn short_key(pk: &str) -> String {
    let t = pk.trim();
    if t.len() <= 16 {
        t.to_string()
    } else {
        format!("{}…{}", &t[..8], &t[t.len() - 4..])
    }
}

fn start_ui_poll() {
    use std::sync::Once;
    static START: Once = Once::new();
    START.call_once(|| {
        std::thread::spawn(|| {
            loop {
                let ms = if host::qr_scan_ui_active() { 50 } else { 400 };
                std::thread::sleep(std::time::Duration::from_millis(ms));
                makepad_widgets::makepad_platform::SignalToUI::set_ui_signal();
            }
        });
    });
}

fn splash_string(cx: &mut Cx, value: ScriptValue) -> String {
    let mut out = String::new();
    cx.with_vm(|vm| {
        vm.bx.heap.cast_to_string(value, &mut out);
    });
    out
}

fn splash_f64(cx: &mut Cx, value: ScriptValue) -> f64 {
    let _ = cx;
    value.as_f64().unwrap_or(0.0)
}

/// `apps/ghalbol/env/<profile>.env`, selected and embedded by `build.rs`.
const BAKED_ENV: &str = include_str!(concat!(env!("OUT_DIR"), "/ghalbol.env"));

/// Load the app environment once per process. Precedence, highest first:
/// the process environment; a developer override `env/.env.<profile>` in the working
/// directory (gitignored, desktop only in practice); the per-app values baked at build time.
/// The profile is `GHALBOL_ENV` at build time, else the Cargo profile — see `build.rs`.
fn load_env_once() {
    static LOADED: std::sync::Once = std::sync::Once::new();
    LOADED.call_once(|| {
        let profile = env!("GHALBOL_ENV");
        let path = format!("env/.env.{profile}");
        let from_file = match std::fs::read_to_string(&path) {
            Ok(raw) => {
                apply_dotenv_text(&raw);
                true
            }
            Err(_) => false,
        };
        apply_dotenv_text(BAKED_ENV);
        let coord = std::env::var("GHALBOL_COORD_URLS").unwrap_or_else(|_| "(unset)".into());
        let app_id = std::env::var("GHALBOL_APP_ID").unwrap_or_default();
        let src = if from_file { path.as_str() } else { "baked" };
        // Visible in logcat / stderr so wrong-profile installs are obvious.
        eprintln!(
            "ghalbol env profile={profile} src={src} GHALBOL_APP_ID={app_id} GHALBOL_COORD_URLS={coord}"
        );
    });
}

fn apply_dotenv_text(raw: &str) {
    for line in raw.lines() {
        let s = line.trim();
        if s.is_empty() || s.starts_with('#') {
            continue;
        }
        let s = s.strip_prefix("export ").unwrap_or(s);
        let Some((key, val)) = s.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let mut val = val.trim().to_string();
        if (val.starts_with('"') && val.ends_with('"') && val.len() >= 2)
            || (val.starts_with('\'') && val.ends_with('\'') && val.len() >= 2)
        {
            val = val[1..val.len() - 1].to_string();
        }
        if std::env::var(key).is_ok() {
            continue;
        }
        // SAFETY: process env is set once at startup before network reads it.
        unsafe { std::env::set_var(key, val) };
    }
}

#[cfg(test)]
mod hub_history_tests {
    use super::{HubHistoryEntry, HubHistoryStack};

    fn entry(tab: u8, page: &str) -> HubHistoryEntry {
        HubHistoryEntry {
            hub_tab: tab,
            narrow_show_room: false,
            conversation_key: String::new(),
            page: page.into(),
        }
    }

    #[test]
    fn back_walks_sheet_then_tab() {
        let mut h = HubHistoryStack::default();
        h.reset(entry(0, ""));
        h.record(entry(2, ""));
        h.record(entry(2, "lang"));
        assert!(h.can_go_back());
        assert_eq!(h.go_back().unwrap().page, "");
        assert_eq!(h.go_back().unwrap().hub_tab, 0);
        assert!(!h.can_go_back());
    }

    #[test]
    fn replace_top_collapses_duplicate() {
        let mut h = HubHistoryStack::default();
        h.reset(entry(0, ""));
        h.record(entry(0, "invite"));
        h.replace_top(entry(0, ""));
        assert_eq!(h.entries.len(), 1);
        assert_eq!(h.index, 0);
    }

    #[test]
    fn room_then_sheet_back_restores_room() {
        let mut h = HubHistoryStack::default();
        h.reset(HubHistoryEntry {
            hub_tab: 0,
            narrow_show_room: false,
            conversation_key: String::new(),
            page: String::new(),
        });
        h.record(HubHistoryEntry {
            hub_tab: 0,
            narrow_show_room: true,
            conversation_key: "aabb".into(),
            page: String::new(),
        });
        h.record(HubHistoryEntry {
            hub_tab: 0,
            narrow_show_room: true,
            conversation_key: "aabb".into(),
            page: "invite".into(),
        });
        let prev = h.go_back().unwrap();
        assert_eq!(prev.page, "");
        assert!(prev.narrow_show_room);
        assert_eq!(prev.conversation_key, "aabb");
    }

    #[test]
    fn contacts_tab_back_restores_chats_room() {
        let mut h = HubHistoryStack::default();
        h.reset(HubHistoryEntry {
            hub_tab: 0,
            narrow_show_room: true,
            conversation_key: "peer1".into(),
            page: String::new(),
        });
        // hub_tab 1 = Contacts peer (was Identity under the old More/Identity hub).
        h.record(HubHistoryEntry {
            hub_tab: 1,
            narrow_show_room: false,
            conversation_key: "peer1".into(),
            page: String::new(),
        });
        let prev = h.go_back().unwrap();
        assert_eq!(prev.hub_tab, 0);
        assert!(prev.narrow_show_room);
        assert_eq!(prev.conversation_key, "peer1");
    }

    #[test]
    fn forward_after_back_restores_sheet() {
        let mut h = HubHistoryStack::default();
        h.reset(entry(0, ""));
        h.record(entry(0, "lang"));
        assert_eq!(h.go_back().unwrap().page, "");
        assert_eq!(h.go_forward().unwrap().page, "lang");
    }
}

#[cfg(test)]
mod bubble_tests {
    use super::bubble_of;
    use ghalbol_core::host::ChatLine;

    fn line(outgoing: bool, delivery: &str, text: &str) -> ChatLine {
        ChatLine {
            outgoing,
            text: text.into(),
            delivery: delivery.into(),
            kind: "text".into(),
            duration_ms: 0,
            audio_path: String::new(),
            local_path: String::new(),
            message_id: String::new(),
            size_bytes: 0,
        }
    }

    #[test]
    fn read_tick_is_blue_and_inbound_has_none() {
        let pending = bubble_of(&line(true, "pending", "hi"), "Alice");
        assert_eq!(pending.tick, "\u{f111}");
        assert!(!pending.read);
        assert_eq!(pending.body, "hi");
        assert_eq!(pending.caption, "You");

        let sent = bubble_of(&line(true, "sent", "hi"), "Alice");
        assert_eq!(sent.tick, "\u{f00c}");
        assert!(!sent.read);

        let delivered = bubble_of(&line(true, "delivered", "hi"), "Alice");
        assert_eq!(delivered.tick, "\u{f560}");
        assert!(!delivered.read);

        let read = bubble_of(&line(true, "read", "hi"), "Alice");
        assert_eq!(read.tick, "\u{f560}");
        assert!(read.read);

        let inbound = bubble_of(&line(false, "read", "hi"), "Alice");
        assert!(inbound.tick.is_empty());
        assert!(!inbound.read);
        assert_eq!(inbound.caption, "Alice");
    }
}
