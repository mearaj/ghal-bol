//! Makepad 2 desktop shell. Product logic stays in `ghal_bol_core::host`.
//!
//! Splash uses `script_mod!` (Makepad 2).

pub use makepad_widgets;

mod i18n;
mod phrases;

use makepad_widgets::*;

use ghal_bol_core::host::{self, ChatLine, RosterEntry, UnlockedSession};

app_main!(App);

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
        icons := FontMember{res: crate_resource("self:resources/fonts/fa-solid-900.ttf") asc: 0.0 desc: 0.0}
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
        icons := FontMember{res: crate_resource("self:resources/fonts/fa-solid-900.ttf") asc: 0.0 desc: 0.0}
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

    let Quiet = Button{
        width: Fit
        height: Fit
        padding: Inset{left: 20. right: 20. top: 14. bottom: 14.}
        margin: Inset{left: 0. right: 0. top: 0. bottom: 0.}
        align: Center
        draw_bg.border_size: 1.0
        draw_bg.border_radius: 10.0
        draw_bg.color: #FFFFFF
        draw_bg.color_hover: #E9EDEF
        draw_bg.color_down: #D1D7DB
        draw_bg.color_focus: #FFFFFF
        draw_bg.border_color: #D1D7DB
        draw_bg.border_color_hover: #D1D7DB
        draw_bg.border_color_down: #D1D7DB
        draw_bg.border_color_focus: #008069
        draw_text.color: #111B21
        draw_text.color_hover: #111B21
        draw_text.color_down: #111B21
        draw_text.color_focus: #111B21
        draw_text.text_style.font_size: 13
    }
    let Go = Button{
        width: Fit
        height: Fit
        padding: Inset{left: 22. right: 22. top: 14. bottom: 14.}
        margin: Inset{left: 0. right: 0. top: 0. bottom: 0.}
        align: Center
        draw_bg.border_size: 1.0
        draw_bg.border_radius: 10.0
        draw_bg.color: #008069
        draw_bg.color_hover: #017561
        draw_bg.color_down: #006655
        draw_bg.color_focus: #008069
        draw_bg.border_color: #006655
        draw_bg.border_color_hover: #006655
        draw_bg.border_color_down: #006655
        draw_bg.border_color_focus: #006655
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
    let Field = TextInput{
        width: Fill
        height: Fit
        padding: Inset{left: 16. right: 16. top: 14. bottom: 14.}
        margin: Inset{left: 0. right: 0. top: 0. bottom: 0.}
        draw_bg.border_size: 1.0
        draw_bg.border_radius: 10.0
        draw_bg.color: #FFFFFF
        draw_bg.color_hover: #FFFFFF
        draw_bg.color_focus: #FFFFFF
        draw_bg.color_down: #FFFFFF
        draw_bg.color_empty: #FFFFFF
        draw_bg.border_color: #D1D7DB
        draw_bg.border_color_hover: #8696A0
        draw_bg.border_color_focus: #008069
        draw_bg.border_color_down: #008069
        draw_bg.border_color_empty: #D1D7DB
        draw_text.color: #111B21
        draw_text.color_hover: #111B21
        draw_text.color_focus: #111B21
        draw_text.color_down: #111B21
        draw_text.color_empty: #667781
        draw_text.color_empty_hover: #667781
        draw_text.color_empty_focus: #667781
        draw_text.text_style.font_size: 15
        // Theme default caret is white — invisible on our white fields.
        draw_cursor.color: #111B21
        draw_selection.color: #B2DFDB
        draw_selection.color_hover: #80CBC4
        draw_selection.color_focus: #80CBC4
        draw_selection.color_down: #4DB6AC
        blink_speed: 0.5
    }
    let Panel = SolidView{
        show_bg: true
        draw_bg.color: #FFFFFF
        width: Fill
        height: Fill
    }
    let Bar = SolidView{
        show_bg: true
        draw_bg.color: #F7F7F7
        width: Fill
        height: Fit
        flow: Right
        spacing: 10
        padding: Inset{left: 14. right: 14. top: 12. bottom: 12.}
        align: {y: 0.5}
    }
    // Constrained-width control: wrap inside the face; ellipsis if still too tall.
    let RailTab = Quiet{
        width: Fill
        height: Fit
        padding: Inset{left: 12. right: 12. top: 12. bottom: 12.}
        align: Center
        // Fill so long labels wrap; label_align centers the ink in that box
        // (layout `align` alone does not move glyphs when the walk is Fill).
        label_walk: Walk{width: Fill, height: Fit}
        label_align: Center
        draw_text.max_lines: 2
        draw_text.text_overflow: Ellipsis
    }
    let MenuRow = Quiet{
        width: Fill
        height: Fit
        padding: Inset{left: 20. right: 20. top: 16. bottom: 16.}
        margin: Inset{left: 0. right: 0. top: 4. bottom: 4.}
        align: {x: 0 y: 0.5}
        label_walk: Walk{width: Fill, height: Fit}
        draw_text.max_lines: 3
        draw_text.text_overflow: Ellipsis
        draw_text.text_style.font_size: 14
    }
    let Bubble = Quiet{
        width: 360
        height: Fit
        padding: Inset{left: 16. right: 16. top: 12. bottom: 12.}
        margin: Inset{left: 4. right: 4. top: 2. bottom: 2.}
        align: {x: 0 y: 0.5}
        label_walk: Walk{width: Fill, height: Fit}
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
    // Horizontal toolbar scroller. Mouse wheel must pan it too (desktop has no
    // horizontal wheel); drag works on every platform. Do not set align.x/y
    // centering here — it fights overflow scroll.
    let HScroll = ScrollXView{
        width: Fill
        height: Fit
        flow: Right
        spacing: 10
        scroll_bars: ScrollBars{
            show_scroll_x: true
            show_scroll_y: false
            scroll_bar_x.drag_scrolling: true
            scroll_bar_x.use_vertical_finger_scroll: true
        }
    }
    // Same scroll behaviour on a painted chrome strip (title + actions in one row).
    let ActionBar = SolidView{
        show_bg: true
        draw_bg.color: #F7F7F7
        width: Fill
        height: Fit
        flow: Right
        spacing: 10
        padding: Inset{left: 14. right: 14. top: 12. bottom: 12.}
        align: {y: 0.5}
        scroll_bars: ScrollBars{
            show_scroll_x: true
            show_scroll_y: false
            scroll_bar_x.drag_scrolling: true
            scroll_bar_x.use_vertical_finger_scroll: true
        }
    }

    startup() do #(App::script_component(vm)){
        ui: Root{
            on_startup: ||{
                ui.identity_copy.render()
            }
            main_window := Window{
                window.inner_size: vec2(1180, 760)
                window.title: "Ghal Bol"
                body +: {
                    identity_view := Panel{
                        draw_bg.color: #F7F7F7
                        width: Fill
                        height: Fill
                        flow: Down
                        spacing: 0
                        padding: 0
                        unlock_scroll := VScroll{
                            spacing: 16
                            padding: 40
                            align: Center
                            identity_copy := View{
                                width: Fill
                                height: Fit
                                flow: Down
                                spacing: 10
                                align: Center
                                on_render: ||{
                                    Label{
                                        text: "Ghal Bol"
                                        draw_text.color: #111B21
                                        draw_text.text_style.font_size: 30
                                    }
                                    hint_label := Label{
                                        width: Fill
                                        text: state.hint
                                        draw_text.color: #54656F
                                    }
                                    status_label := Label{
                                        width: Fill
                                        text: state.status
                                        draw_text.color: #667781
                                    }
                                    algo_label := Label{
                                        width: Fill
                                        text: state.algo_line
                                        draw_text.color: #008069
                                    }
                                }
                            }
                            password_row := View{
                                width: 360
                                height: Fit
                                flow: Right
                                spacing: 8
                                align: {y: 0.5}
                                password_input := Field{
                                    empty_text: "Password"
                                    is_password: true
                                    width: Fill
                                    height: Fit
                                }
                                password_eye := Quiet{ text: "Show" }
                            }
                            unlock_button := Go{ text: "Continue" width: 360 }
                            mode_button := Quiet{ text: "Import an existing key" width: 360 }
                            secret_row := View{
                                width: 360
                                height: Fit
                                flow: Right
                                spacing: 8
                                align: {y: 0.5}
                                secret_input := Field{
                                    empty_text: "Private key hex"
                                    is_password: true
                                    width: Fill
                                    height: Fit
                                }
                                secret_eye := Quiet{ text: "Show" }
                            }
                            algo_row := HScroll{
                                width: 360
                                spacing: 10
                                algo_secp := Quiet{
                                    text: "secp256k1  [on]"
                                    draw_bg.color: #DCF8C6
                                    on_click: ||{
                                        mod.state.algo = "secp256k1"
                                        mod.state.algo_line = "Algorithm: secp256k1"
                                        mod.state.algo_gen = mod.state.algo_gen + 1
                                        ui.algo_secp.set_text("secp256k1  [on]")
                                        ui.algo_ed.set_text("ed25519")
                                        ui.algo_p256.set_text("ecdsa-p256")
                                        ui.identity_copy.render()
                                    }
                                }
                                algo_ed := Quiet{
                                    text: "ed25519"
                                    on_click: ||{
                                        mod.state.algo = "ed25519"
                                        mod.state.algo_line = "Algorithm: ed25519"
                                        mod.state.algo_gen = mod.state.algo_gen + 1
                                        ui.algo_secp.set_text("secp256k1")
                                        ui.algo_ed.set_text("ed25519  [on]")
                                        ui.algo_p256.set_text("ecdsa-p256")
                                        ui.identity_copy.render()
                                    }
                                }
                                algo_p256 := Quiet{
                                    text: "ecdsa-p256"
                                    on_click: ||{
                                        mod.state.algo = "ecdsa-p256"
                                        mod.state.algo_line = "Algorithm: ecdsa-p256"
                                        mod.state.algo_gen = mod.state.algo_gen + 1
                                        ui.algo_secp.set_text("secp256k1")
                                        ui.algo_ed.set_text("ed25519")
                                        ui.algo_p256.set_text("ecdsa-p256  [on]")
                                        ui.identity_copy.render()
                                    }
                                }
                            }
                            delete_button := Stop{ text: "Delete identity on this device" width: 360 }
                            import_backup_button := Quiet{ text: "Import keystore backup" width: 360 }
                        }
                    }
                    hub_view := Panel{
                        visible: false
                        draw_bg.color: #F0F2F5
                        flow: Down
                        hub_body := View{
                            width: Fill
                            height: Fill
                            flow: Right
                        nav_rail := SolidView{
                            show_bg: true
                            draw_bg.color: #F7F7F7
                            width: 176
                            height: Fill
                            flow: Down
                            spacing: 10
                            padding: Inset{top: 16. bottom: 14. left: 12. right: 12.}
                            Label{
                                text: "Ghal Bol"
                                draw_text.color: #008069
                                draw_text.text_style.font_size: 14
                                width: Fill
                            }
                            tab_chats := RailTab{ text: "Chats" }
                            tab_identity := RailTab{ text: "Identity" }
                            tab_more := RailTab{ text: "More" }
                            Filler{}
                            lock_button := RailTab{ text: "Lock" }
                        }
                        rail_divider := SolidView{
                            show_bg: true
                            draw_bg.color: #D1D7DB
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
                                roster_bar := ActionBar{
                                    Label{
                                        text: "Ghal Bol"
                                        draw_text.color: #111B21
                                        draw_text.text_style.font_size: 18
                                        width: Fit
                                    }
                                    new_chat_button := Quiet{ text: "New chat" }
                                    invite_button := Quiet{ text: "Invite" }
                                    join_button := Quiet{ text: "Join" }
                                    scan_button := Quiet{ text: "Scan" }
                                    delivery_button := Quiet{ text: "Delivery" }
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
                                                    text: "No chats yet"
                                                    draw_text.color: #111B21
                                                    draw_text.text_style.font_size: 18
                                                }
                                                Label{
                                                    width: Fill
                                                    text: "Scan a QR or paste an invitation to start a 1:1 chat."
                                                    draw_text.color: #667781
                                                }
                                            }
                                        else for i, row in state.roster {
                                            Quiet{
                                                width: Fill
                                                height: Fit
                                                padding: Inset{left: 14. right: 14. top: 14. bottom: 14.}
                                                margin: Inset{left: 0. right: 0. top: 0. bottom: 0.}
                                                align: {x: 0 y: 0.5}
                                                label_walk: Walk{width: Fill, height: Fit}
                                                draw_text.max_lines: 3
                                                draw_text.text_overflow: Ellipsis
                                                draw_bg.border_radius: 10.0
                                                draw_bg.color: #FFFFFF
                                                draw_bg.color_hover: #F0F2F5
                                                draw_bg.color_down: #E9EDEF
                                                draw_bg.border_color: #E9EDEF
                                                draw_bg.border_color_hover: #E9EDEF
                                                draw_text.color: #111B21
                                                draw_text.text_style.font_size: 14
                                                text: row.line
                                                on_click: ||{
                                                    mod.state.selected_pk = row.pk
                                                    mod.state.selected_title = row.title
                                                    mod.state.open_gen = mod.state.open_gen + 1
                                                    ui.chat_log.render()
                                                }
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
                                chat_bar := SolidView{
                                    show_bg: true
                                    draw_bg.color: #F7F7F7
                                    width: Fill
                                    height: Fit
                                    flow: Down
                                    spacing: 0
                                    View{
                                        width: Fill
                                        height: Fit
                                        flow: Right
                                        spacing: 10
                                        padding: Inset{left: 14. right: 14. top: 12. bottom: 8.}
                                        align: {y: 0.5}
                                        chat_back_button := Quiet{ text: "Back" visible: false }
                                        chat_title_label := Label{
                                            text: state.selected_title
                                            draw_text.color: #111B21
                                            draw_text.text_style.font_size: 17
                                            width: Fill
                                            max_lines: 1
                                            text_overflow: Ellipsis
                                        }
                                    }
                                    chat_actions := HScroll{
                                        padding: Inset{left: 14. right: 14. top: 0. bottom: 12.}
                                        call_button := Quiet{ text: "Voice call" }
                                        video_button := Quiet{ text: "Video call" }
                                        add_button := Quiet{ text: "Add" }
                                        block_button := Quiet{ text: "Block" }
                                    }
                                }
                                chat_log := VScroll{
                                    spacing: 10
                                    padding: 16
                                    on_render: ||{
                                        call_label := Label{
                                            width: Fill
                                            text: state.call_line
                                            draw_text.color: #667781
                                        }
                                        if state.lines.len() == 0
                                            Label{
                                                width: Fill
                                                text: state.transcript
                                                draw_text.color: #54656F
                                            }
                                        else for i, line in state.lines {
                                            if line.side == "out"
                                                View{
                                                    width: Fill
                                                    height: Fit
                                                    flow: Right
                                                    spacing: 8
                                                    align: {x: 1 y: 1}
                                                    Bubble{
                                                        draw_bg.color: #DCF8C6
                                                        draw_bg.color_hover: #C8F0B0
                                                        draw_bg.color_down: #B7E89A
                                                        draw_bg.border_color: #DCF8C6
                                                        draw_text.color: #111B21
                                                        text: line.body
                                                        on_click: ||{
                                                            mod.state.play_path = line.path
                                                            mod.state.play_action = line.action
                                                            mod.state.play_gen = mod.state.play_gen + 1
                                                        }
                                                    }
                                                    if line.tick != ""
                                                        if line.read == "1"
                                                            Label{
                                                                text: line.tick
                                                                draw_text.color: #34B7F1
                                                            }
                                                        else
                                                            Label{
                                                                text: line.tick
                                                                draw_text.color: #667781
                                                            }
                                                }
                                            else
                                                View{
                                                    width: Fill
                                                    height: Fit
                                                    flow: Right
                                                    spacing: 8
                                                    align: {x: 0 y: 1}
                                                    Bubble{
                                                        draw_bg.color: #FFFFFF
                                                        draw_bg.color_hover: #F0F2F5
                                                        draw_bg.color_down: #E9EDEF
                                                        draw_bg.border_color: #FFFFFF
                                                        draw_text.color: #111B21
                                                        text: line.body
                                                        on_click: ||{
                                                            mod.state.play_path = line.path
                                                            mod.state.play_action = line.action
                                                            mod.state.play_gen = mod.state.play_gen + 1
                                                        }
                                                    }
                                                }
                                        }
                                    }
                                }
                                composer_bar := SolidView{
                                    show_bg: true
                                    draw_bg.color: #F0F0F0
                                    width: Fill
                                    height: Fit
                                    flow: Down
                                    spacing: 10
                                    padding: 14
                                    older_button := Quiet{ text: "Older messages" }
                                    composer_row := HScroll{
                                        record_button := Quiet{ text: "Voice note" }
                                        choose_file_button := Quiet{ text: "File" }
                                        composer := Field{
                                            empty_text: "Message"
                                            width: 280
                                        }
                                        send_button := Go{ text: "Send" }
                                    }
                                }
                            }
                        }
                        identity_pane := Panel{
                            visible: false
                            draw_bg.color: #FFFFFF
                            width: Fill
                            height: Fill
                            flow: Down
                            spacing: 0
                            padding: Inset{left: 0. right: 0. top: 0. bottom: 0.}
                            identity_scroll := VScroll{
                                spacing: 14
                                padding: 32
                                Label{
                                    text: "Identity"
                                    draw_text.color: #111B21
                                    draw_text.text_style.font_size: 24
                                }
                                Label{
                                    width: Fill
                                    text: state.identity_blurb
                                    draw_text.color: #54656F
                                }
                                invite_button_id := Go{ text: "Show QR invitation" width: Fit }
                                View{
                                    width: Fill
                                    height: Fit
                                    flow: Right{wrap: true}
                                    spacing: 10
                                    invite_copy_button := Quiet{ text: "Copy invitation" }
                                    key_copy_button := Quiet{ text: "Copy public key" }
                                }
                                Label{
                                    text: "Your identity"
                                    draw_text.color: #111B21
                                    draw_text.text_style.font_size: 18
                                }
                                Label{
                                    width: Fill
                                    text: state.identity_detail
                                    draw_text.color: #111B21
                                }
                                Label{
                                    text: "Backup and private key"
                                    draw_text.color: #111B21
                                    draw_text.text_style.font_size: 16
                                }
                                View{
                                    width: Fill
                                    height: Fit
                                    flow: Right{wrap: true}
                                    spacing: 10
                                    key_button := Quiet{ text: "Show private key" }
                                    backup_button := Quiet{ text: "Export backup" }
                                    password_button := Quiet{ text: "Change password" }
                                }
                                Label{
                                    width: Fill
                                    text: "App password is required to view the secret. Keep backups with your password."
                                    draw_text.color: #667781
                                }
                                about_button := MenuRow{ text: "Availability and About" }
                                logout_button := Stop{ text: "Log out" width: Fit }
                            }
                        }
                        more_pane := Panel{
                            visible: false
                            draw_bg.color: #FFFFFF
                            width: Fill
                            height: Fill
                            flow: Down
                            spacing: 0
                            more_scroll := VScroll{
                                spacing: 10
                                padding: 32
                                Label{
                                    text: "More"
                                    draw_text.color: #111B21
                                    draw_text.text_style.font_size: 24
                                }
                                Label{
                                    width: Fill
                                    text: "1:1 P2P encrypted chat with voice and video. No phone number or cloud account required."
                                    draw_text.color: #54656F
                                }
                                contacts_button := MenuRow{ text: "Contacts — add, rename, or remove by public key" }
                                blocked_button := MenuRow{ text: "Blocked contacts — people you blocked on this device" }
                                log_button := MenuRow{ text: "App log — session diagnostics" }
                                language_button := MenuRow{ text: "Language — display language for this app" }
                                scan_file_button := MenuRow{ text: "Join from QR picture — open a PNG or JPEG invite" }
                            }
                        }
                        }
                        bottom_nav := SolidView{
                            visible: false
                            show_bg: true
                            draw_bg.color: #F0F0F0
                            width: Fill
                            height: Fit
                            flow: Right
                            spacing: 10
                            padding: Inset{left: 12. right: 12. top: 12. bottom: 12.}
                            align: {y: 0.5}
                            bottom_chats := RailTab{ text: "Chats" }
                            bottom_identity := RailTab{ text: "Identity" }
                            bottom_more := RailTab{ text: "More" }
                        }
                    }
                    sheet_view := Panel{
                        visible: false
                        draw_bg.color: #F7F7F7
                        width: Fill
                        height: Fill
                        flow: Down
                        spacing: 12
                        padding: Inset{left: 24. right: 24. top: 20. bottom: 16.}
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
                                    sheet_title := Label{
                                        width: Fill
                                        text: state.sheet_title
                                        draw_text.color: #111B21
                                        draw_text.text_style.font_size: 22
                                    }
                                    sheet_body := Label{
                                        width: Fill
                                        text: state.sheet_body
                                        draw_text.color: #54656F
                                    }
                                    if state.languages.len() > 0
                                        for i, row in state.languages {
                                            if row.code == ""
                                                Label{
                                                    width: Fill
                                                    margin: Inset{left: 4. right: 4. top: 12. bottom: 4.}
                                                    text: row.name
                                                    draw_text.color: #667781
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
                            paste_row := View{
                                width: Fill
                                height: Fit
                                flow: Right
                                spacing: 8
                                align: {y: 0.5}
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
                                align: {y: 0.5}
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
                                align: {y: 0.5}
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
                    call_view := SolidView{
                        visible: false
                        show_bg: true
                        draw_bg.color: #111B21
                        width: Fill
                        height: Fill
                        flow: Down
                        spacing: 0
                        padding: 0
                        call_scroll := VScroll{
                            spacing: 20
                            padding: 32
                            align: Center
                            call_copy := View{
                                width: Fill
                                height: Fit
                                flow: Down
                                spacing: 10
                                align: Center
                                on_render: ||{
                                    call_screen_title := Label{
                                        width: Fill
                                        text: state.call_line
                                        draw_text.color: #FFFFFF
                                        draw_text.text_style.font_size: 22
                                    }
                                }
                            }
                            video_row := View{
                                width: Fill
                                height: Fit
                                flow: Right
                                spacing: 12
                                local_video := Image{ width: 160 height: 120 }
                                remote_video := Image{ width: Fill height: 280 }
                            }
                            call_actions := HScroll{
                                width: Fill
                                spacing: 12
                                accept_call_button := Go{ text: "Accept" }
                                mute_button := Quiet{ text: "Mute" }
                                speaker_button := Quiet{ text: "Speaker" }
                                end_call_button := Stop{ text: "End call" }
                            }
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
    #[rust]
    namespace: String,
    #[rust]
    session: Option<UnlockedSession>,
    #[rust]
    open_peer: String,
    #[rust]
    seen_open_gen: f64,
    #[rust]
    env_loaded: bool,
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
    file_path: String,
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
}

impl App {
    fn ensure_env(&mut self) {
        if self.env_loaded {
            return;
        }
        self.env_loaded = true;
        load_dotenv_if_present();
    }

    fn push_identity_copy(&self, cx: &mut Cx) {
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
            "Choose a password to create a device identity. An app password encrypts it on this device and is never sent to a server."
        } else {
            "Paste a private key and choose a password. Keys from cryptocurrency wallets are strongly discouraged."
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
        self.mark_algorithm(cx);
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
        script_eval!(cx, {
            ui.algo_secp.set_text(#(secp))
            ui.algo_ed.set_text(#(ed))
            ui.algo_p256.set_text(#(p256))
        });
    }

    fn set_screen(&mut self, cx: &mut Cx, screen: &str) {
        self.ui.widget(cx, ids!(identity_view)).set_visible(cx, screen == "identity");
        self.ui.widget(cx, ids!(hub_view)).set_visible(cx, screen == "hub");
        self.ui.widget(cx, ids!(sheet_view)).set_visible(cx, screen == "sheet");
        self.ui.widget(cx, ids!(call_view)).set_visible(cx, screen == "call");
        if screen == "hub" {
            self.set_hub_tab(cx, self.hub_tab);
        }
        cx.redraw_all();
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
        let chats = tab == 0;
        let identity = tab == 1;
        let more = tab == 2;
        self.ui.widget(cx, ids!(chats_pane)).set_visible(cx, chats);
        self.ui.widget(cx, ids!(identity_pane)).set_visible(cx, identity);
        self.ui.widget(cx, ids!(more_pane)).set_visible(cx, more);
        style_nav_tab(cx, &mut self.ui.widget(cx, ids!(tab_chats)), chats);
        style_nav_tab(cx, &mut self.ui.widget(cx, ids!(tab_identity)), identity);
        style_nav_tab(cx, &mut self.ui.widget(cx, ids!(tab_more)), more);
        style_nav_tab(cx, &mut self.ui.widget(cx, ids!(bottom_chats)), chats);
        style_nav_tab(cx, &mut self.ui.widget(cx, ids!(bottom_identity)), identity);
        style_nav_tab(cx, &mut self.ui.widget(cx, ids!(bottom_more)), more);
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
            short_key(&pk)
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

        let want_page = entry.page.clone();
        if want_page.is_empty() {
            if !self.page.is_empty() {
                self.force_close_sheet(cx);
            }
        } else if want_page == "lang" {
            self.open_languages(cx);
        } else if want_page != self.page {
            self.open_page(cx, &want_page);
        }

        self.history_suppress = false;
        self.apply_shell_layout(cx);
    }

    /// Close sheet UI without the forced-backup re-prompt (history restore path).
    fn force_close_sheet(&mut self, cx: &mut Cx) {
        self.page.clear();
        script_eval!(cx, {
            ui.sheet_view.set_visible(false)
            ui.identity_view.set_visible(false)
            ui.call_view.set_visible(false)
            ui.hub_view.set_visible(true)
        });
        self.set_screen(cx, "hub");
        self.refresh_roster(cx);
    }

    /// One step of natural back (mouse side button, Android back/gesture, Escape, toolbar Back).
    ///
    /// Unwinds **visible** nest depth first (call → sheet → lock → room → tab), then browser
    /// history. History-only first was a no-op on many nested screens when the stack entry
    /// matched the current UI or a sheet was never recorded. Every gesture shares this path.
    ///
    /// Returns true when the gesture was consumed. `allow_quit` exits at hub/identity root
    /// (mouse / Android back); Escape leaves the app open.
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
        // First-time backup is required — back cannot leave the sheet empty-handed.
        if self.needs_backup && !self.page.is_empty() {
            self.close_sheet(cx);
            return true;
        }
        // Any sheet (Invite, Language, Password, Contacts, …) — one level, every entry point.
        if !self.page.is_empty() {
            if self.hub_history.can_go_back() {
                let before = self.hub_snapshot();
                if let Some(prev) = self.hub_history.go_back() {
                    self.apply_hub_history(cx, prev);
                    if self.hub_snapshot() != before {
                        return true;
                    }
                }
            }
            // History missing, stuck, or no-op — still leave the sheet.
            self.force_close_sheet(cx);
            self.hub_history.replace_top(self.hub_snapshot());
            return true;
        }
        if self.ui_locked && self.session.is_some() {
            self.dismiss_ui_lock(cx);
            return true;
        }
        // Hub chrome: walk history, skipping no-op restores (duplicate stack entries).
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
        if self.session.is_some() && !self.ui_locked {
            if !self.shell_split && self.narrow_show_room {
                self.leave_chat_room(cx);
                self.hub_history.replace_top(self.hub_snapshot());
                return true;
            }
            if self.hub_tab != 0 {
                // Record is suppressed so we only replace_top (one structural step).
                self.history_suppress = true;
                self.set_hub_tab(cx, 0);
                self.history_suppress = false;
                self.hub_history.replace_top(self.hub_snapshot());
                return true;
            }
            if self.shell_split && !self.open_peer.is_empty() {
                self.clear_open_chat(cx);
                self.hub_history.replace_top(self.hub_snapshot());
                return true;
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

    fn clear_open_chat(&mut self, cx: &mut Cx) {
        self.open_peer.clear();
        self.narrow_show_room = false;
        host::set_open_room(None);
        script_eval!(cx, {
            mod.state.selected_pk = ""
            mod.state.selected_title = "Select a chat"
            mod.state.transcript = ""
            ui.chat_title_label.set_text("Select a chat")
            ui.chat_log.render()
        });
        self.apply_shell_layout(cx);
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

        self.ui.widget(cx, ids!(nav_rail)).set_visible(cx, show_rail);
        self.ui.widget(cx, ids!(rail_divider)).set_visible(cx, show_rail);
        self.ui.widget(cx, ids!(bottom_nav)).set_visible(cx, show_bottom);
        self.ui.widget(cx, ids!(roster_col)).set_visible(cx, show_roster);
        self.ui.widget(cx, ids!(chat_col)).set_visible(cx, show_chat);
        self.ui.widget(cx, ids!(chat_back_button)).set_visible(cx, show_chat && !split);

        if show_rail {
            // Never use a skinny icon-only rail: text labels ("Identity") must fit with
            // padding. Wide windows get a roomier rail; otherwise keep a readable minimum.
            let rail = if window_w >= 1100.0 { 220.0 } else { 176.0 };
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
            let roster = if split && show_chat {
                (window_w * 0.34).clamp(280.0, 400.0)
            } else {
                0.0
            };
            let width = if roster > 0.0 {
                Size::Fixed(roster)
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
            let pane = if self.hub_tab == 1 {
                ids!(identity_pane)
            } else {
                ids!(more_pane)
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
        cx.redraw_all();
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
        self.ensure_env();
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
                let status = match &net {
                    Ok(_) => String::new(),
                    Err(e) => format!("Network: {e}"),
                };
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
        for row in roster
            .into_iter()
            .filter(|row| !row.is_blocked && roster_matches(row, &self.search))
        {
            let title = row_label(&row);
            let pk = row.public_key_hex.clone();
            let line = roster_line(&row);
            script_eval!(cx, {
                mod.state.roster.push({
                    title: #(title)
                    line: #(line)
                    pk: #(pk)
                })
                true
            });
        }
        script_eval!(cx, {
            ui.roster_list.render()
            ui.chat_log.render()
            ui.chat_title_label.set_text("Select a chat")
        });
        self.push_identity_pane(cx);
        self.apply_language(cx);
        self.hub_tab = 0;
        self.narrow_show_room = false;
        self.hub_history.reset(self.hub_snapshot());
        self.set_screen(cx, "hub");
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

    fn show_sheet(&mut self, cx: &mut Cx, page: &str, title: &str, body: String, action: &str) {
        if !self.history_suppress {
            // Pin hub/room under this sheet before page changes.
            self.hub_history.replace_top(self.hub_snapshot());
        }
        self.page = page.to_string();
        let title = title.to_string();
        let action = action.to_string();
        let show_invite = page == "invite";
        let show_paste = matches!(page, "join" | "key" | "password" | "invite");
        let show_pk = matches!(page, "add" | "blocked" | "password");
        let show_alias = matches!(page, "add" | "about" | "password");
        let paste_secret = matches!(page, "key" | "password");
        let pk_secret = page == "password";
        let alias_secret = page == "password";
        script_eval!(cx, {
            mod.state.sheet_title = #(title)
            mod.state.sheet_body = #(body)
            mod.state.languages = []
            ui.sheet_action.set_text(#(action))
            ui.status_row.set_visible(false)
            ui.invite_frame.set_visible(#(show_invite))
            ui.paste_row.set_visible(#(show_paste))
            ui.pk_row.set_visible(#(show_pk))
            ui.alias_row.set_visible(#(show_alias))
            ui.hub_view.set_visible(false)
            ui.sheet_view.set_visible(true)
            ui.sheet_copy.render()
        });
        self.ui.widget(cx, ids!(status_row)).set_visible(cx, false);
        self.ui.widget(cx, ids!(invite_frame)).set_visible(cx, show_invite);
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
                let links = host::invite_links(&session.public_key_hex, None).unwrap_or_default();
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
                    body
                } else {
                    format!("{body}\n\n{target}")
                };
                self.show_sheet(cx, page, &i18n::phrase(&lang, "contacts"), body, &i18n::phrase(&lang, "remove"));
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
                    format!("{body}\n\n{}", i18n::phrase(&lang, "blocked_empty"))
                };
                self.show_sheet(cx, page, &i18n::phrase(&lang, "blocked"), body, &i18n::phrase(&lang, "toggle"));
            }
            "delivery" => {
                self.show_sheet(cx, page, &i18n::phrase(&lang, "delivery"), host::delivery_summary(), &i18n::phrase(&lang, "refresh"));
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
        let lang = self.lang.clone();
        let json = host::export_keystore(&self.namespace)?;
        let Some(path) = rfd::FileDialog::new()
            .set_file_name("ghal-bol-keystore.json")
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

    fn finish_backup(&mut self, cx: &mut Cx, msg: String) {
        if !self.needs_backup {
            script_eval!(cx, {
                mod.state.sheet_body = #(msg)
                ui.sheet_copy.render()
            });
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
            mod.state.sheet_body = #(msg)
            ui.sheet_copy.render()
        });
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
        if !alias.is_empty() || self.page == "about" || self.page == "password" {
            self.add_alias = alias;
        }
        let page = self.page.clone();
        let ns = self.namespace.clone();
        let result = match page.as_str() {
            "join" => host::accept_invite(&ns, &self.paste).map(|pk| format!("Added {}", short_key(&pk))),
            "add" => host::add_contact(&ns, &self.add_pk, Some(self.add_alias.trim())).map(|()| "Contact added".into()),
            "contacts" => {
                let pk = self.open_peer.trim().to_string();
                host::remove_contact(&ns, &pk).map(|()| "Removed".into())
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
            "delivery" | "log" => Ok(String::new()),
            "invite" => {
                let link = self.paste.trim();
                if link.is_empty() {
                    Err("Invitation link is empty.".into())
                } else {
                    copy_to_clipboard(link).map(|()| "Invitation copied".into())
                }
            }
            "key" => host::reveal_secret(&ns, &self.paste),
            "backup" => self.save_backup(),
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
                } else if page == "key" || page == "password" || page == "invite" {
                    script_eval!(cx, {
                        mod.state.sheet_body = #(msg)
                        ui.sheet_copy.render()
                    });
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
                    mod.state.sheet_body = #(e)
                    ui.sheet_copy.render()
                });
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
            Ok(None) => "Recording… tap again to send".to_string(),
            Ok(Some((duration_ms, opus))) => match host::send_voice_note(&peer, duration_ms, opus) {
                Ok(()) => "Voice note sent".into(),
                Err(e) => e,
            },
            Err(e) => e,
        };
        let sent = msg == "Voice note sent";
        script_eval!(cx, {
            mod.state.call_line = #(msg)
            ui.chat_log.render()
        });
        if sent {
            self.refresh_transcript(cx);
        }
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
        self.ui.widget(cx, ids!(invite_frame)).set_visible(cx, false);
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
        } else {
            host::open_local_file(&path).map(|()| "Opened".into())
        };
        let note = match &msg {
            Ok(text) if action == "fetch" || action == "voice" => text.clone(),
            Err(e) => e.clone(),
            Ok(_) => String::new(),
        };
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
        let logout = i18n::text(lang, i18n::Key::Logout).to_string();
        let older = i18n::text(lang, i18n::Key::Older).to_string();
        let voice = i18n::text(lang, i18n::Key::VoiceCall).to_string();
        let video = i18n::text(lang, i18n::Key::VideoCall).to_string();
        let accept = i18n::text(lang, i18n::Key::Accept).to_string();
        let mute = i18n::text(lang, i18n::Key::Mute).to_string();
        let speaker = i18n::phrase(lang, "speaker");
        let end = i18n::text(lang, i18n::Key::EndCall).to_string();
        let block = i18n::text(lang, i18n::Key::Block).to_string();
        let add = i18n::phrase(lang, "add_short");
        let record = i18n::text(lang, i18n::Key::VoiceNote).to_string();
        let file = i18n::text(lang, i18n::Key::ChooseFile).to_string();
        let send = i18n::text(lang, i18n::Key::Send).to_string();
        let scan = i18n::text(lang, i18n::Key::ScanQr).to_string();
        let scan_file = i18n::text(lang, i18n::Key::QrPicture).to_string();
        script_eval!(cx, {
            ui.unlock_button.set_text(#(unlock))
            ui.new_chat_button.set_text(#(new_chat))
            ui.invite_button.set_text(#(invitation))
            ui.join_button.set_text(#(join))
            ui.scan_button.set_text(#(scan))
            ui.scan_file_button.set_text(#(scan_file))
            ui.contacts_button.set_text(#(contacts))
            ui.blocked_button.set_text(#(blocked))
            ui.delivery_button.set_text(#(delivery))
            ui.log_button.set_text(#(log))
            ui.key_button.set_text(#(key))
            ui.backup_button.set_text(#(backup))
            ui.password_button.set_text(#(password))
            ui.about_button.set_text(#(about))
            ui.language_button.set_text(#(language))
            ui.lock_button.set_text(#(lock))
            ui.logout_button.set_text(#(logout))
            ui.older_button.set_text(#(older))
            ui.call_button.set_text(#(voice))
            ui.video_button.set_text(#(video))
            ui.accept_call_button.set_text(#(accept))
            ui.mute_button.set_text(#(mute))
            ui.speaker_button.set_text(#(speaker))
            ui.end_call_button.set_text(#(end))
            ui.add_button.set_text(#(add))
            ui.block_button.set_text(#(block))
            ui.record_button.set_text(#(record))
            ui.choose_file_button.set_text(#(file))
            ui.send_button.set_text(#(send))
        });
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
            self.apply_shell_layout(cx);
            if peer_changed {
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

    fn refresh_transcript(&mut self, cx: &mut Cx) {
        let Some(session) = self.session.clone() else {
            return;
        };
        if self.open_peer.is_empty() {
            return;
        }
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
        script_eval!(cx, {
            mod.state.transcript = #(empty)
            mod.state.lines = []
            true
        });
        for line in page.lines {
            let bubble = bubble_of(&line);
            let body = bubble.body;
            let tick = bubble.tick;
            let read = if bubble.read { "1" } else { "0" };
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
            let action = if line.kind == "voice" {
                "voice"
            } else if line.kind == "attachment_offer" && line.local_path.is_empty() && !line.message_id.is_empty() {
                "fetch"
            } else if !open.is_empty() {
                "file"
            } else {
                ""
            };
            script_eval!(cx, {
                mod.state.lines.push({
                    body: #(body)
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
        for row in roster
            .into_iter()
            .filter(|row| !row.is_blocked && roster_matches(row, &self.search))
        {
            let title = row_label(&row);
            let pk = row.public_key_hex.clone();
            let line = roster_line(&row);
            script_eval!(cx, {
                mod.state.roster.push({
                    title: #(title)
                    line: #(line)
                    pk: #(pk)
                })
                true
            });
        }
        script_eval!(cx, { ui.roster_list.render() });
    }

    fn push_identity_pane(&self, cx: &mut Cx) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let blurb = "QR and invitation links encode your identity public key — not IP addresses. After someone adds you, the app finds you via coordination, same Wi‑Fi, or when you connect to them.".to_string();
        let detail = format!(
            "Namespace: {}\nIdentity (share this):\n{}",
            session.app_namespace, session.public_key_hex
        );
        let self_line = format!("You  {}", short_key(&session.public_key_hex));
        script_eval!(cx, {
            mod.state.identity_blurb = #(blurb)
            mod.state.identity_detail = #(detail)
            mod.state.self_line = #(self_line)
        });
    }

    fn leave_chat_room(&mut self, cx: &mut Cx) {
        self.narrow_show_room = false;
        host::set_open_room(None);
        self.apply_shell_layout(cx);
    }

    fn apply_shell_widths(&mut self, cx: &mut Cx, window_w: f64) {
        self.window_w = window_w;
        self.apply_shell_layout(cx);
    }

    fn submit_send(&mut self, cx: &mut Cx) {
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
                self.refresh_transcript(cx);
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
            if self.page.is_empty() {
                self.refresh_roster(cx);
                self.refresh_transcript(cx);
            }
        }
        self.refresh_video(cx);
        self.take_qr(cx);
    }

    fn refresh_video(&mut self, cx: &mut Cx) {
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

    fn take_qr(&mut self, cx: &mut Cx) {
        let Some(result) = host::take_qr_scan() else {
            return;
        };
        let ns = self.namespace.clone();
        match result.and_then(|text| host::accept_invite(&ns, &text)) {
            Ok(pk) => {
                let line = format!("Joined {pk}");
                script_eval!(cx, {
                    mod.state.call_line = #(line)
                    ui.chat_log.render()
                });
                self.refresh_roster(cx);
            }
            Err(e) => {
                script_eval!(cx, {
                    mod.state.call_line = #(e)
                    ui.chat_log.render()
                });
            }
        }
    }

    fn refresh_call_line(&mut self, cx: &mut Cx) {
        let line = host::call_banner();
        host::note_call_banner(&line);
        let calling = !line.is_empty();
        script_eval!(cx, {
            mod.state.call_line = #(line)
            ui.chat_log.render()
        });
        if calling == self.call_open {
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
        } else if self.session.is_some() && !self.ui_locked {
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
        if self.ui.button(cx, ids!(algo_ed)).clicked(actions) {
            self.algorithm = "ed25519".into();
            self.mark_algorithm(cx);
        }
        if self.ui.button(cx, ids!(algo_p256)).clicked(actions) {
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
            let msg = self.import_backup();
            match msg {
                Ok(session) => {
                    self.password.clear();
                    let pk = session.public_key_hex.clone();
                    let ns = session.app_namespace.clone();
                    self.session = Some(session);
                    let status = match host::start_network(&ns) {
                        Ok(_) => "Backup imported".into(),
                        Err(e) => format!("Network: {e}"),
                    };
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
        if self.ui.button(cx, ids!(new_chat_button)).clicked(actions) {
            self.set_hub_tab(cx, 0);
            self.open_page(cx, "add");
        }
        if self.ui.button(cx, ids!(tab_chats)).clicked(actions)
            || self.ui.button(cx, ids!(bottom_chats)).clicked(actions)
        {
            self.set_hub_tab(cx, 0);
        }
        if self.ui.button(cx, ids!(tab_identity)).clicked(actions)
            || self.ui.button(cx, ids!(bottom_identity)).clicked(actions)
        {
            self.push_identity_pane(cx);
            self.set_hub_tab(cx, 1);
        }
        if self.ui.button(cx, ids!(tab_more)).clicked(actions)
            || self.ui.button(cx, ids!(bottom_more)).clicked(actions)
        {
            self.set_hub_tab(cx, 2);
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
                let msg = match host::invite_links(&session.public_key_hex, None) {
                    Ok((https, _)) => match copy_to_clipboard(&https) {
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
                let msg = match copy_to_clipboard(&session.public_key_hex) {
                    Ok(()) => "Public key copied".to_string(),
                    Err(e) => e,
                };
                script_eval!(cx, { mod.state.identity_blurb = #(msg) });
            }
        }
        if self.ui.button(cx, ids!(join_button)).clicked(actions) {
            self.open_page(cx, "join");
        }
        if self.ui.button(cx, ids!(scan_button)).clicked(actions) {
            let msg = match host::start_qr_scan() {
                Ok(()) => "Point the camera at an invitation QR".to_string(),
                Err(e) => e,
            };
            script_eval!(cx, {
                mod.state.call_line = #(msg)
                ui.chat_log.render()
            });
        }
        if self.ui.button(cx, ids!(scan_file_button)).clicked(actions) {
            let msg = if let Some(path) = rfd::FileDialog::new()
                .add_filter("Pictures", &["png", "jpg", "jpeg"])
                .pick_file()
            {
                let ns = self.namespace.clone();
                match host::decode_qr_file(&path.display().to_string())
                    .and_then(|text| host::accept_invite(&ns, &text))
                {
                    Ok(pk) => format!("Joined {pk}"),
                    Err(e) => e,
                }
            } else {
                "No picture chosen".into()
            };
            script_eval!(cx, {
                mod.state.call_line = #(msg)
                ui.chat_log.render()
            });
            self.refresh_roster(cx);
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
            let msg = if peer.is_empty() {
                "Select a chat first".into()
            } else {
                match host::start_video_call(&peer) {
                    Ok(_) => "Video call".into(),
                    Err(e) => e,
                }
            };
            script_eval!(cx, {
                mod.state.call_line = #(msg)
                ui.chat_log.render()
            });
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
        if self.ui.button(cx, ids!(call_button)).clicked(actions) {
            let peer = self.open_peer.clone();
            let msg = if peer.is_empty() {
                "Select a chat first".into()
            } else {
                match host::start_voice_call(&peer) {
                    Ok(_) => "Calling…".into(),
                    Err(e) => e,
                }
            };
            script_eval!(cx, {
                mod.state.call_line = #(msg)
                ui.chat_log.render()
            });
        }
        if self.ui.button(cx, ids!(accept_call_button)).clicked(actions) {
            let msg = match host::accept_incoming_call() {
                Ok(()) => "Call accepted".into(),
                Err(e) => e,
            };
            script_eval!(cx, {
                mod.state.call_line = #(msg)
                ui.chat_log.render()
            });
        }
        if self.ui.button(cx, ids!(end_call_button)).clicked(actions) {
            let msg = match host::end_call() {
                Ok(()) => String::new(),
                Err(e) => e,
            };
            script_eval!(cx, {
                mod.state.call_line = #(msg)
                ui.chat_log.render()
            });
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
        if self.ui.button(cx, ids!(add_button)).clicked(actions) {
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
            self.refresh_roster(cx);
        }
        if self.ui.button(cx, ids!(block_button)).clicked(actions) {
            let peer = self.open_peer.clone();
            if !peer.is_empty() {
                let _ = host::set_blocked(&self.namespace, &peer, true);
                self.open_peer.clear();
                host::set_open_room(None);
                self.refresh_roster(cx);
            }
        }
        if self.ui.button(cx, ids!(choose_file_button)).clicked(actions) {
            let peer = self.open_peer.clone();
            let msg = if peer.is_empty() {
                "Select a chat first".into()
            } else if let Some(path) = rfd::FileDialog::new().pick_file() {
                self.file_path = path.display().to_string();
                match host::send_attachment(&peer, &self.file_path) {
                    Ok(()) => "File queued".into(),
                    Err(e) => e,
                }
            } else {
                String::new()
            };
            let queued = msg == "File queued";
            if !msg.is_empty() {
                script_eval!(cx, {
                    mod.state.call_line = #(msg)
                    ui.chat_log.render()
                });
            }
            if queued {
                self.refresh_transcript(cx);
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
        let send = self.ui.button(cx, ids!(send_button)).clicked(actions)
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
        if !vm.is_reload() {
            let ns = host::default_app_namespace().to_string();
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
                    transcript: ""
                    lines: []
                    selected_pk: ""
                    selected_title: "Select a chat"
                    call_line: ""
                    open_gen: 0
                    play_gen: 0
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
            self.namespace = host::default_app_namespace().to_string();
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
                let _ = cx.request_permission(Permission::Camera);
                let _ = cx.request_permission(Permission::AudioInput);
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
        if let Event::Signal = event {
            self.drain_events(cx);
        }
    }
}

fn row_label(row: &RosterEntry) -> String {
    let unknown = if row.is_known { "" } else { "Unknown · " };
    format!("{unknown}{}", row.title)
}

fn roster_line(row: &RosterEntry) -> String {
    let title = row_label(row);
    let unread = if row.unread > 0 {
        format!("  ({})", row.unread)
    } else {
        String::new()
    };
    let status = if row.availability.is_empty() {
        String::new()
    } else {
        format!(" · {}", row.availability)
    };
    let preview = if row.preview.is_empty() {
        "Tap to open chat".to_string()
    } else {
        row.preview.clone()
    };
    format!("{title}{status}{unread}\n{preview}")
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
    tick: String,
    read: bool,
}

fn bubble_of(line: &ChatLine) -> Bubble {
    let body = if line.kind == "voice" {
        let secs = (line.duration_ms / 1000).max(1);
        format!("Voice note  {secs}s")
    } else if line.kind == "attachment_offer" {
        let name = if line.text.is_empty() {
            "File".to_string()
        } else {
            line.text.clone()
        };
        if line.local_path.is_empty() {
            format!("{name}  ·  tap to download")
        } else {
            name
        }
    } else if line.kind == "text" || line.kind.is_empty() {
        line.text.clone()
    } else if line.text.is_empty() {
        format!("[{}]", line.kind)
    } else {
        line.text.clone()
    };
    if !line.outgoing {
        return Bubble {
            body,
            tick: String::new(),
            read: false,
        };
    }
    let read = line.delivery == "read";
    let tick = match line.delivery.as_str() {
        "pending" => "○",
        "sent" => "✓",
        "delivered" | "read" => "✓✓",
        other if !other.is_empty() => other,
        _ => "",
    };
    Bubble {
        body,
        tick: tick.to_string(),
        read,
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

fn copy_to_clipboard(text: &str) -> Result<(), String> {
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
        return Err("Could not copy. Install wl-copy or xclip.".into());
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = text;
        Err("Copy is not available on this platform yet.".into())
    }
}

fn style_nav_tab(cx: &mut Cx, button: &mut WidgetRef, active: bool) {
    if active {
        script_apply_eval!(cx, button, {
            draw_bg.color: #008069
            draw_bg.color_hover: #017561
            draw_bg.color_down: #006655
            draw_bg.color_focus: #008069
            draw_bg.border_size: 1.0
            draw_bg.border_radius: 10.0
            draw_bg.border_color: #006655
            draw_bg.border_color_hover: #006655
            draw_bg.border_color_down: #006655
            draw_bg.border_color_focus: #006655
            draw_text.color: #FFFFFF
            draw_text.color_hover: #FFFFFF
            draw_text.color_down: #FFFFFF
            draw_text.color_focus: #FFFFFF
            draw_text.max_lines: 2
            draw_text.text_overflow: Ellipsis
            padding: Inset{left: 12. right: 12. top: 12. bottom: 12.}
            align: Center
            label_walk: Walk{width: Fill, height: Fit}
            label_align: Center
        });
    } else {
        script_apply_eval!(cx, button, {
            draw_bg.color: #FFFFFF
            draw_bg.color_hover: #E9EDEF
            draw_bg.color_down: #D1D7DB
            draw_bg.color_focus: #FFFFFF
            draw_bg.border_size: 1.0
            draw_bg.border_radius: 10.0
            draw_bg.border_color: #D1D7DB
            draw_bg.border_color_hover: #D1D7DB
            draw_bg.border_color_down: #D1D7DB
            draw_bg.border_color_focus: #008069
            draw_text.color: #111B21
            draw_text.color_hover: #111B21
            draw_text.color_down: #111B21
            draw_text.color_focus: #111B21
            draw_text.max_lines: 2
            draw_text.text_overflow: Ellipsis
            padding: Inset{left: 12. right: 12. top: 12. bottom: 12.}
            align: Center
            label_walk: Walk{width: Fill, height: Fit}
            label_align: Center
        });
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
                std::thread::sleep(std::time::Duration::from_millis(400));
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

/// Same keys as the debug env file, when it is present next to the repo.
fn load_dotenv_if_present() {
    let candidates = [
        "env/.env.development",
        "env/.env.production",
    ];
    for path in candidates {
        let Ok(raw) = std::fs::read_to_string(path) else {
            continue;
        };
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
            if key.is_empty() || std::env::var(key).is_ok() {
                continue;
            }
            let mut val = val.trim().to_string();
            if (val.starts_with('"') && val.ends_with('"') && val.len() >= 2)
                || (val.starts_with('\'') && val.ends_with('\'') && val.len() >= 2)
            {
                val = val[1..val.len() - 1].to_string();
            }
            // SAFETY: process env is set once at startup before network reads it.
            unsafe { std::env::set_var(key, val) };
        }
        break;
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
}

#[cfg(test)]
mod bubble_tests {
    use super::bubble_of;
    use ghal_bol_core::host::ChatLine;

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
        }
    }

    #[test]
    fn read_tick_is_blue_and_inbound_has_none() {
        let pending = bubble_of(&line(true, "pending", "hi"));
        assert_eq!(pending.tick, "○");
        assert!(!pending.read);
        assert_eq!(pending.body, "hi");

        let sent = bubble_of(&line(true, "sent", "hi"));
        assert_eq!(sent.tick, "✓");
        assert!(!sent.read);

        let delivered = bubble_of(&line(true, "delivered", "hi"));
        assert_eq!(delivered.tick, "✓✓");
        assert!(!delivered.read);

        let read = bubble_of(&line(true, "read", "hi"));
        assert_eq!(read.tick, "✓✓");
        assert!(read.read);
        assert!(!read.body.contains('✓'));

        let inbound = bubble_of(&line(false, "read", "hi"));
        assert!(inbound.tick.is_empty());
        assert!(!inbound.read);
    }
}
