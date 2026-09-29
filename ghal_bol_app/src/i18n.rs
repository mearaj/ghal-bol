//! UI language catalog.
//!
//! Broad worldwide picker (India Eighth Schedule + state languages, South /
//! East / Southeast Asia, Middle East, Europe, Africa, Americas) with native
//! names. Chrome strings exist for a starter set; every other language still
//! selects correctly and falls back to English until its table is filled in.
//! Glyph coverage comes from the Noto `WorldFonts` FontFamily in `main.rs`.

#[derive(Clone, Copy, Debug)]
pub struct LanguageInfo {
    pub code: &'static str,
    pub name_en: &'static str,
    pub native_name: &'static str,
    pub region: &'static str,
}

#[derive(Clone, Copy)]
#[repr(usize)]
pub enum Key {
    Unlock,
    NewChat,
    Invitation,
    Join,
    Contacts,
    Blocked,
    Delivery,
    AppLog,
    PrivateKey,
    Backup,
    PasswordMenu,
    About,
    Language,
    Lock,
    Logout,
    Older,
    VoiceCall,
    VideoCall,
    Accept,
    Mute,
    EndCall,
    Block,
    VoiceNote,
    ChooseFile,
    Send,
    ScanQr,
    QrPicture,
}

pub fn system_code() -> String {
    let raw = std::env::var("LANG").unwrap_or_default();
    resolve_code(raw.split(['.', '@']).next().unwrap_or("en")).into()
}

/// Map OS / preference tags onto a catalogue code (`zh_TW` → `zh-Hant`, etc.).
pub fn resolve_code(raw: &str) -> &'static str {
    let tag = raw
        .split(['.', '@'])
        .next()
        .unwrap_or("en")
        .trim()
        .replace('_', "-");
    if tag.is_empty() {
        return "en";
    }
    let lower = tag.to_ascii_lowercase();
    if let Some(info) = language_by_code(&lower) {
        return info.code;
    }
    // Preserve script/region casing used in the catalogue (zh-Hant, pt-BR).
    if let Some(info) = language_by_code(&tag) {
        return info.code;
    }
    match lower.as_str() {
        "zh-tw" | "zh-hk" | "zh-mo" | "zh-hant" => return "zh-Hant",
        "zh-cn" | "zh-sg" | "zh-hans" => return "zh",
        "pt-br" => return "pt-BR",
        "es-mx" | "es-419" => return "es-MX",
        _ => {}
    }
    let primary = lower.split('-').next().unwrap_or("en");
    language_by_code(primary).map(|l| l.code).unwrap_or("en")
}

pub fn language_by_code(code: &str) -> Option<&'static LanguageInfo> {
    LANGUAGES.iter().find(|l| l.code.eq_ignore_ascii_case(code))
}

/// UI-string lookup key: regional variants share a parent table when present.
fn ui_lang(lang: &str) -> &str {
    let lang = if lang.is_empty() { "en" } else { lang };
    match lang {
        "zh-Hant" | "yue" => "zh",
        "pt-BR" => "pt",
        "es-MX" => "es",
        "awa" | "mag" | "raj" => "hi",
        "gom" => "kok",
        "tcy" => "kn",
        other => other,
    }
}

pub fn text(lang: &str, key: Key) -> &'static str {
    let lang = ui_lang(lang);
    if matches!(key, Key::ScanQr | Key::QrPicture) {
        let id = if matches!(key, Key::ScanQr) { "scan_qr" } else { "qr_picture" };
        if let Some(s) = extra(lang, id).or_else(|| extra("en", id)) {
            return s;
        }
    }
    catalog(lang)
        .get(key as usize)
        .copied()
        .unwrap_or(EN[key as usize])
}

fn catalog(lang: &str) -> &'static [&'static str] {
    match ui_lang(lang) {
        "es" => ES,
        "fr" => FR,
        "pt" => PT,
        "de" => DE,
        "it" => IT,
        "nl" => NL,
        "pl" => PL,
        "uk" => UK,
        "ru" => RU,
        "ar" => AR,
        "fa" => FA,
        "tr" => TR,
        "zh" => ZH,
        "ja" => JA,
        "ko" => KO,
        "vi" => VI,
        "th" => TH,
        "id" => ID,
        "sw" => SW,
        "hi" => HI,
        "bn" => BN,
        "as" => AS,
        "brx" => BRX,
        "doi" => DOI,
        "gu" => GU,
        "kn" => KN,
        "ks" => KS,
        "kok" => KOK,
        "ml" => ML,
        "mni" => MNI,
        "mr" => MR,
        "mai" => MAI,
        "ne" => NE,
        "or" => OR,
        "pa" => PA,
        "sa" => SA,
        "sat" => SAT,
        "sd" => SD,
        "ta" => TA,
        "te" => TE,
        "ur" => UR,
        "bho" => BHO,
        "hne" => HNE,
        "lus" => LUS,
        "kha" => KHA,
        "grt" => GRT,
        "trp" => TRP,
        "lep" => LEP,
        _ => EN,
    }
}

const EN: &[&str] = &[
    "Unlock", "New chat", "My invitation", "Join", "Contacts", "Blocked", "Delivery", "App log",
    "Private key", "Backup", "Password", "About", "Language", "Lock", "Log out", "Older messages",
    "Voice call", "Video call", "Accept", "Mute", "End call", "Block", "Voice note", "Choose file", "Send",
    "Scan QR", "QR picture",
];
const ES: &[&str] = &[
    "Desbloquear", "Chat nuevo", "Mi invitación", "Unirse", "Contactos", "Bloqueados", "Entrega", "Registro",
    "Clave privada", "Copia", "Contraseña", "Acerca de", "Idioma", "Bloquear", "Salir", "Mensajes anteriores",
    "Llamada de voz", "Videollamada", "Aceptar", "Silencio", "Colgar", "Bloquear", "Nota de voz", "Elegir archivo", "Enviar",
];
const FR: &[&str] = &[
    "Déverrouiller", "Nouvelle discussion", "Mon invitation", "Rejoindre", "Contacts", "Bloqués", "Remise", "Journal",
    "Clé privée", "Sauvegarde", "Mot de passe", "À propos", "Langue", "Verrouiller", "Déconnexion", "Messages plus anciens",
    "Appel vocal", "Appel vidéo", "Accepter", "Muet", "Raccrocher", "Bloquer", "Message vocal", "Choisir un fichier", "Envoyer",
];
const PT: &[&str] = &[
    "Desbloquear", "Nova conversa", "Meu convite", "Entrar", "Contatos", "Bloqueados", "Entrega", "Registo",
    "Chave privada", "Cópia", "Palavra-passe", "Sobre", "Idioma", "Bloquear", "Sair", "Mensagens antigas",
    "Chamada de voz", "Videochamada", "Aceitar", "Silenciar", "Desligar", "Bloquear", "Mensagem de voz", "Escolher ficheiro", "Enviar",
];
const DE: &[&str] = &[
    "Entsperren", "Neuer Chat", "Meine Einladung", "Beitreten", "Kontakte", "Blockiert", "Zustellung", "Protokoll",
    "Privater Schlüssel", "Sicherung", "Passwort", "Über", "Sprache", "Sperren", "Abmelden", "Ältere Nachrichten",
    "Sprachanruf", "Videoanruf", "Annehmen", "Stumm", "Auflegen", "Blockieren", "Sprachnachricht", "Datei wählen", "Senden",
];
const IT: &[&str] = &[
    "Sblocca", "Nuova chat", "Il mio invito", "Entra", "Contatti", "Bloccati", "Consegna", "Registro",
    "Chiave privata", "Backup", "Password", "Informazioni", "Lingua", "Blocca", "Esci", "Messaggi precedenti",
    "Chiamata vocale", "Videochiamata", "Accetta", "Muto", "Chiudi", "Blocca", "Nota vocale", "Scegli file", "Invia",
];
const NL: &[&str] = &[
    "Ontgrendelen", "Nieuwe chat", "Mijn uitnodiging", "Deelnemen", "Contacten", "Geblokkeerd", "Aflevering", "Logboek",
    "Privésleutel", "Back-up", "Wachtwoord", "Over", "Taal", "Vergrendelen", "Afmelden", "Oudere berichten",
    "Spraakoproep", "Videogesprek", "Accepteren", "Dempen", "Ophangen", "Blokkeren", "Spraakbericht", "Bestand kiezen", "Versturen",
];
const PL: &[&str] = &[
    "Odblokuj", "Nowy czat", "Moje zaproszenie", "Dołącz", "Kontakty", "Zablokowani", "Dostarczenie", "Dziennik",
    "Klucz prywatny", "Kopia", "Hasło", "O aplikacji", "Język", "Zablokuj", "Wyloguj", "Starsze wiadomości",
    "Rozmowa głosowa", "Rozmowa wideo", "Odbierz", "Wycisz", "Zakończ", "Zablokuj", "Wiadomość głosowa", "Wybierz plik", "Wyślij",
];
const UK: &[&str] = &[
    "Розблокувати", "Новий чат", "Моє запрошення", "Приєднатися", "Контакти", "Заблоковані", "Доставлення", "Журнал",
    "Приватний ключ", "Резервна копія", "Пароль", "Про застосунок", "Мова", "Заблокувати", "Вийти", "Старіші повідомлення",
    "Голосовий дзвінок", "Відеодзвінок", "Прийняти", "Вимкнути звук", "Завершити", "Заблокувати", "Голосове повідомлення", "Обрати файл", "Надіслати",
];
const RU: &[&str] = &[
    "Разблокировать", "Новый чат", "Моё приглашение", "Присоединиться", "Контакты", "Заблокированные", "Доставка", "Журнал",
    "Закрытый ключ", "Резервная копия", "Пароль", "О приложении", "Язык", "Заблокировать", "Выйти", "Старые сообщения",
    "Голосовой звонок", "Видеозвонок", "Принять", "Без звука", "Завершить", "Заблокировать", "Голосовое сообщение", "Выбрать файл", "Отправить",
];
const AR: &[&str] = &[
    "فتح القفل", "محادثة جديدة", "دعوتي", "انضمام", "جهات الاتصال", "المحظورون", "التسليم", "السجل",
    "المفتاح الخاص", "نسخة احتياطية", "كلمة المرور", "حول", "اللغة", "قفل", "خروج", "رسائل أقدم",
    "مكالمة صوتية", "مكالمة فيديو", "قبول", "كتم", "إنهاء", "حظر", "رسالة صوتية", "اختيار ملف", "إرسال",
];
const FA: &[&str] = &[
    "باز کردن قفل", "گفتگوی جدید", "دعوت من", "پیوستن", "مخاطبان", "مسدودشده", "تحویل", "گزارش",
    "کلید خصوصی", "پشتیبان", "گذرواژه", "درباره", "زبان", "قفل", "خروج", "پیام‌های قدیمی‌تر",
    "تماس صوتی", "تماس تصویری", "پذیرفتن", "بی‌صدا", "پایان", "مسدود", "پیام صوتی", "انتخاب پرونده", "ارسال",
];
const TR: &[&str] = &[
    "Kilidi aç", "Yeni sohbet", "Davetim", "Katıl", "Kişiler", "Engellenenler", "Teslim", "Günlük",
    "Özel anahtar", "Yedek", "Parola", "Hakkında", "Dil", "Kilitle", "Çıkış", "Eski mesajlar",
    "Sesli arama", "Görüntülü arama", "Kabul et", "Sessiz", "Bitir", "Engelle", "Sesli mesaj", "Dosya seç", "Gönder",
];
const ZH: &[&str] = &[
    "解锁", "新聊天", "我的邀请", "加入", "联系人", "已屏蔽", "投递", "日志",
    "私钥", "备份", "密码", "关于", "语言", "锁定", "退出", "更早的消息",
    "语音通话", "视频通话", "接听", "静音", "挂断", "屏蔽", "语音消息", "选择文件", "发送",
];
const JA: &[&str] = &[
    "ロック解除", "新しいチャット", "自分の招待", "参加", "連絡先", "ブロック中", "配信", "ログ",
    "秘密鍵", "バックアップ", "パスワード", "情報", "言語", "ロック", "ログアウト", "以前のメッセージ",
    "音声通話", "ビデオ通話", "応答", "ミュート", "終了", "ブロック", "ボイスメッセージ", "ファイルを選択", "送信",
];
const KO: &[&str] = &[
    "잠금 해제", "새 대화", "내 초대", "참여", "연락처", "차단됨", "전달", "로그",
    "개인 키", "백업", "비밀번호", "정보", "언어", "잠금", "로그아웃", "이전 메시지",
    "음성 통화", "영상 통화", "수락", "음소거", "종료", "차단", "음성 메시지", "파일 선택", "보내기",
];
const VI: &[&str] = &[
    "Mở khóa", "Cuộc trò chuyện mới", "Lời mời của tôi", "Tham gia", "Danh bạ", "Đã chặn", "Chuyển phát", "Nhật ký",
    "Khóa riêng", "Bản sao lưu", "Mật khẩu", "Giới thiệu", "Ngôn ngữ", "Khóa", "Đăng xuất", "Tin nhắn cũ hơn",
    "Cuộc gọi thoại", "Cuộc gọi video", "Chấp nhận", "Tắt tiếng", "Kết thúc", "Chặn", "Tin nhắn thoại", "Chọn tệp", "Gửi",
];
const TH: &[&str] = &[
    "ปลดล็อก", "แชทใหม่", "คำเชิญของฉัน", "เข้าร่วม", "รายชื่อ", "ที่บล็อก", "การส่ง", "บันทึก",
    "กุญแจส่วนตัว", "สำรอง", "รหัสผ่าน", "เกี่ยวกับ", "ภาษา", "ล็อก", "ออก", "ข้อความเก่ากว่า",
    "โทรเสียง", "โทรวิดีโอ", "รับ", "ปิดเสียง", "วางสาย", "บล็อก", "ข้อความเสียง", "เลือกไฟล์", "ส่ง",
];
const ID: &[&str] = &[
    "Buka kunci", "Obrolan baru", "Undangan saya", "Gabung", "Kontak", "Diblokir", "Pengiriman", "Catatan",
    "Kunci pribadi", "Cadangan", "Kata sandi", "Tentang", "Bahasa", "Kunci", "Keluar", "Pesan lebih lama",
    "Panggilan suara", "Panggilan video", "Terima", "Bisukan", "Akhiri", "Blokir", "Pesan suara", "Pilih berkas", "Kirim",
];
const SW: &[&str] = &[
    "Fungua", "Mazungumzo mapya", "Mwaliko wangu", "Jiunge", "Anwani", "Waliozuiwa", "Uwasilishaji", "Kumbukumbu",
    "Ufunguo wa siri", "Nakala", "Nenosiri", "Kuhusu", "Lugha", "Funga", "Toka", "Ujumbe wa zamani",
    "Simu ya sauti", "Simu ya video", "Kubali", "Nyamaza", "Maliza", "Zui", "Ujumbe wa sauti", "Chagua faili", "Tuma",
];
const HI: &[&str] = &[
    "अनलॉक", "नई बातचीत", "मेरा निमंत्रण", "जुड़ें", "संपर्क", "अवरुद्ध", "डिलीवरी", "लॉग",
    "निजी कुंजी", "बैकअप", "पासवर्ड", "परिचय", "भाषा", "लॉक", "लॉग आउट", "पुराने संदेश",
    "वॉइस कॉल", "वीडियो कॉल", "स्वीकार", "म्यूट", "कॉल समाप्त", "ब्लॉक", "वॉइस नोट", "फ़ाइल चुनें", "भेजें",
];
const BN: &[&str] = &[
    "আনলক", "নতুন চ্যাট", "আমার আমন্ত্রণ", "যোগ দিন", "পরিচিতি", "অবরুদ্ধ", "ডেলিভারি", "লগ",
    "ব্যক্তিগত চাবি", "ব্যাকআপ", "পাসওয়ার্ড", "সম্পর্কে", "ভাষা", "লক", "লগ আউট", "পুরোনো বার্তা",
    "ভয়েস কল", "ভিডিও কল", "গ্রহণ", "নিঃশব্দ", "কল শেষ", "ব্লক", "ভয়েস নোট", "ফাইল বেছে নিন", "পাঠান",
];
const AS: &[&str] = &[
    "আনলক", "নতুন চেট", "মোৰ আমন্ত্ৰণ", "যোগদান", "সম্পৰ্ক", "অৱৰুদ্ধ", "ডেলিভাৰী", "লগ",
    "ব্যক্তিগত চাবি", "বেকআপ", "পাছৱৰ্ড", "বিষয়ে", "ভাষা", "লক", "লগ আউট", "পুৰণি বাৰ্তা",
    "ভয়েচ কল", "ভিডিঅ' কল", "গ্ৰহণ", "নিমাত", "কল শেষ", "ব্লক", "ভয়েচ নোট", "ফাইল বাছক", "পঠাওক",
];
const BRX: &[&str] = &[
    "खुलि", "गोदान थुनलाइ", "आंनि सावरायनाय", "ज्वं", "सम्पर्क", "ब्लक", "फहंनाय", "लग",
    "प्राइभेट कि", "बेकआप", "पासवर्ड", "सोमोन्दै", "राव", "लक", "लग आउट", "गोजाम थुनलाइ",
    "भइस कल", "भिडिअ कल", "नाजाव", "म्युट", "कल जोबनाय", "ब्लक", "भइस नट", "फाइल सायख", "दाखाय",
];
const DOI: &[&str] = &[
    "अनलॉक", "नवी गल्ल", "मेरा नेउता", "जुड़ो", "संपर्क", "ब्लॉक", "डिलीवरी", "लॉग",
    "प्राइवेट कुंजी", "बैकअप", "पासवर्ड", "बारे", "भाषा", "लॉक", "लॉग आउट", "पुराने संदेश",
    "वॉइस काल", "वीडियो काल", "मंजूर", "म्यूट", "काल बंद", "ब्लॉक", "वॉइस नोट", "फाइल चुनो", "भेजो",
];
const GU: &[&str] = &[
    "અનલૉક", "નવી વાતચીત", "મારું આમંત્રણ", "જોડાઓ", "સંપર્કો", "અવરોધિત", "ડિલિવરી", "લૉગ",
    "ખાનગી ચાવી", "બૅકઅપ", "પાસવર્ડ", "વિશે", "ભાષા", "લૉક", "લૉગ આઉટ", "જૂના સંદેશા",
    "વૉઇસ કૉલ", "વિડિયો કૉલ", "સ્વીકારો", "મ્યૂટ", "કૉલ બંધ", "બ્લૉક", "વૉઇસ નોટ", "ફાઇલ પસંદ કરો", "મોકલો",
];
const KN: &[&str] = &[
    "ಅನ್‌ಲಾಕ್", "ಹೊಸ ಚಾಟ್", "ನನ್ನ ಆಮಂತ್ರಣ", "ಸೇರಿ", "ಸಂಪರ್ಕಗಳು", "ನಿರ್ಬಂಧಿತ", "ತಲುಪಿಸುವಿಕೆ", "ಲಾಗ್",
    "ಖಾಸಗಿ ಕೀ", "ಬ್ಯಾಕಪ್", "ಪಾಸ್‌ವರ್ಡ್", "ಬಗ್ಗೆ", "ಭಾಷೆ", "ಲಾಕ್", "ಲಾಗ್ ಔಟ್", "ಹಳೆಯ ಸಂದೇಶಗಳು",
    "ಧ್ವನಿ ಕರೆ", "ವೀಡಿಯೊ ಕರೆ", "ಸ್ವೀಕರಿಸಿ", "ಮ್ಯೂಟ್", "ಕರೆ ಮುಗಿಸಿ", "ನಿರ್ಬಂಧಿಸಿ", "ಧ್ವನಿ ಟಿಪ್ಪಣಿ", "ಕಡತ ಆಯ್ಕೆ", "ಕಳುಹಿಸಿ",
];
const KS: &[&str] = &[
    "اَن لاک", "نٔو گَپ", "مےٚن دَعوت", "شٲمِل", "رابطہٕ", "بُلاک", "ڈِلیوری", "لاگ",
    "پرائیویٹ کی", "بیک اَپ", "پاسوَرڈ", "مُتعَلِق", "زَبان", "لاک", "لاگ آؤٹ", "پرٛون پٲغام",
    "آواز کال", "ویڈیو کال", "قَبول", "میوٹ", "کال خَتم", "بُلاک", "آواز نوٹ", "فایل چُنِو", "سَوزِو",
];
const KOK: &[&str] = &[
    "अनलॉक", "नवी गप्प", "म्हजें आमंत्रण", "जोडचे", "संपर्क", "ब्लॉक", "डिलिव्हरी", "लॉग",
    "खाजगी चावी", "बॅकअप", "पासवर्ड", "विशीं", "भास", "लॉक", "लॉग आउट", "आदले संदेश",
    "व्हॉइस कॉल", "व्हिडिओ कॉल", "मान्य", "म्यूट", "कॉल बंद", "ब्लॉक", "व्हॉइस नोट", "फायल वेंच", "धाड",
];
const ML: &[&str] = &[
    "അൺലോക്ക്", "പുതിയ ചാറ്റ്", "എന്റെ ക്ഷണം", "ചേരുക", "കോൺടാക്റ്റുകൾ", "തടഞ്ഞവ", "ഡെലിവറി", "ലോഗ്",
    "സ്വകാര്യ കീ", "ബാക്കപ്പ്", "പാസ്‌വേഡ്", "കുറിച്ച്", "ഭാഷ", "ലോക്ക്", "ലോഗ് ഔട്ട്", "പഴയ സന്ദേശങ്ങൾ",
    "വോയ്‌സ് കോൾ", "വീഡിയോ കോൾ", "സ്വീകരിക്കുക", "മ്യൂട്ട്", "കോൾ അവസാനിപ്പിക്കുക", "ബ്ലോക്ക്", "വോയ്‌സ് നോട്ട്", "ഫയൽ തിരഞ്ഞെടുക്കുക", "അയയ്ക്കുക",
];
const MNI: &[&str] = &[
    "অনলক", "অনৌবা চ্যাট", "ঐগী ইনভিটেশন", "শোকহনবা", "কন্টেক্ট", "ব্লক", "দেলিভারি", "লগ",
    "প্রাইভেট কি", "বেকআপ", "পাসৱার্দ", "মরমদা", "লোন", "লক", "লগ আউট", "অরিবা মেসেজ",
    "ভয়েস কল", "ভিদিও কল", "য়াহনবা", "মিউট", "কল লোইশিনবা", "ব্লক", "ভয়েস নোট", "ফাইল খনবা", "থার্কপা",
];
const MR: &[&str] = &[
    "अनलॉक", "नवीन गप्पा", "माझे आमंत्रण", "सामील व्हा", "संपर्क", "अवरोधित", "डिलिव्हरी", "लॉग",
    "खाजगी कळ", "बॅकअप", "पासवर्ड", "माहिती", "भाषा", "लॉक", "लॉग आउट", "जुने संदेश",
    "व्हॉइस कॉल", "व्हिडिओ कॉल", "स्वीकारा", "म्यूट", "कॉल बंद", "ब्लॉक", "व्हॉइस नोट", "फाइल निवडा", "पाठवा",
];
const MAI: &[&str] = &[
    "अनलॉक", "नव गप", "हमर निमंत्रण", "जोड़ू", "संपर्क", "ब्लॉक", "डिलीवरी", "लॉग",
    "निजी चाबी", "बैकअप", "पासवर्ड", "बारे", "भाषा", "लॉक", "लॉग आउट", "पुरान संदेश",
    "आवाज काल", "वीडियो काल", "स्वीकार", "म्यूट", "काल बंद", "ब्लॉक", "आवाज नोट", "फाइल चुनू", "पठाउ",
];
const NE: &[&str] = &[
    "अनलक", "नयाँ कुराकानी", "मेरो निमन्त्रणा", "सामेल", "सम्पर्क", "ब्लक", "डेलिभरी", "लग",
    "निजी साँचो", "ब्याकअप", "पासवर्ड", "बारे", "भाषा", "लक", "लग आउट", "पुराना सन्देश",
    "भ्वाइस कल", "भिडियो कल", "स्वीकार", "म्युट", "कल अन्त्य", "ब्लक", "भ्वाइस नोट", "फाइल छान्नुहोस्", "पठाउनुहोस्",
];
const OR: &[&str] = &[
    "ଅନଲକ୍", "ନୂଆ ଚାଟ୍", "ମୋ ନିମନ୍ତ୍ରଣ", "ଯୋଗ ଦିଅନ୍ତୁ", "ସମ୍ପର୍କ", "ଅବରୋଧିତ", "ଡେଲିଭରି", "ଲଗ୍",
    "ବ୍ୟକ୍ତିଗତ ଚାବି", "ବ୍ୟାକଅପ୍", "ପାସୱାର୍ଡ", "ବିଷୟରେ", "ଭାଷା", "ଲକ୍", "ଲଗ୍ ଆଉଟ୍", "ପୁରୁଣା ସନ୍ଦେଶ",
    "ଭଏସ୍ କଲ୍", "ଭିଡିଓ କଲ୍", "ଗ୍ରହଣ", "ମ୍ୟୁଟ୍", "କଲ୍ ଶେଷ", "ବ୍ଲକ୍", "ଭଏସ୍ ନୋଟ୍", "ଫାଇଲ୍ ବାଛନ୍ତୁ", "ପଠାନ୍ତୁ",
];
const PA: &[&str] = &[
    "ਅਨਲਾਕ", "ਨਵੀਂ ਗੱਲਬਾਤ", "ਮੇਰਾ ਸੱਦਾ", "ਜੁੜੋ", "ਸੰਪਰਕ", "ਬਲਾਕ", "ਡਿਲਿਵਰੀ", "ਲਾਗ",
    "ਨਿੱਜੀ ਕੁੰਜੀ", "ਬੈਕਅੱਪ", "ਪਾਸਵਰਡ", "ਬਾਰੇ", "ਭਾਸ਼ਾ", "ਲਾਕ", "ਲਾਗ ਆਉਟ", "ਪੁਰਾਣੇ ਸੁਨੇਹੇ",
    "ਵੌਇਸ ਕਾਲ", "ਵੀਡੀਓ ਕਾਲ", "ਸਵੀਕਾਰ", "ਮਿਊਟ", "ਕਾਲ ਖਤਮ", "ਬਲਾਕ", "ਵੌਇਸ ਨੋਟ", "ਫਾਈਲ ਚੁਣੋ", "ਭੇਜੋ",
];
const SA: &[&str] = &[
    "उद्घाटय", "नवीनसंलापः", "मम आमन्त्रणम्", "संगच्छ", "सम्पर्काः", "अवरुद्धाः", "प्रेषणम्", "वृत्तान्तः",
    "गुप्तकुञ्जिका", "प्रतिकृतिः", "गुप्तशब्दः", "विषये", "भाषा", "पिधत्स्व", "निर्गच्छ", "प्राचीनानि सन्देशानि",
    "ध्वनिकॉल", "दृश्यकॉल", "स्वीकुरु", "मूकम्", "समापय", "वारय", "ध्वनिसन्देशः", "सञ्चिकां चिनु", "प्रेषय",
];
const SAT: &[&str] = &[
    "खोल", "नावा कथा", "इञाक् आमंत्रण", "जोड़ोक", "संपर्क", "ब्लॉक", "पाहुँच", "लॉग",
    "निजी चाबी", "बैकअप", "पासवर्ड", "बारे", "भाषा", "बंद", "बाहर", "मारां संदेश",
    "आवाज काल", "वीडियो काल", "माना", "चुप", "काल चबा", "रोक", "आवाज नोट", "फाइल बाछा", "कुल",
];
const SD: &[&str] = &[
    "اڻ لاڪ", "نئين ڳالهه", "منهنجي دعوت", "شامل ٿيو", "رابطا", "بلاڪ", "پهچ", "لاگ",
    "نجي چاٻي", "بيڪ اپ", "پاسورڊ", "بابت", "ٻولي", "لاڪ", "لاگ آئوٽ", "پراڻا پيغام",
    "آواز ڪال", "وڊيو ڪال", "قبول", "خاموش", "ڪال ختم", "بلاڪ", "آواز نوٽ", "فائل چونڊيو", "موڪليو",
];
const TA: &[&str] = &[
    "திற", "புதிய அரட்டை", "என் அழைப்பு", "சேர்", "தொடர்புகள்", "தடுக்கப்பட்டவை", "வழங்கல்", "பதிவு",
    "தனிப்பட்ட திறவுகோல்", "காப்பு", "கடவுச்சொல்", "பற்றி", "மொழி", "பூட்டு", "வெளியேறு", "பழைய செய்திகள்",
    "குரல் அழைப்பு", "காணொலி அழைப்பு", "ஏற்றுக்கொள்", "ஒலிநிறுத்து", "அழைப்பை முடி", "தடு", "குரல் குறிப்பு", "கோப்பைத் தேர்", "அனுப்பு",
];
const TE: &[&str] = &[
    "అన్‌లాక్", "కొత్త చాట్", "నా ఆహ్వానం", "చేరండి", "పరిచయాలు", "నిరోధించినవి", "డెలివరీ", "లాగ్",
    "ప్రైవేట్ కీ", "బ్యాకప్", "పాస్‌వర్డ్", "గురించి", "భాష", "లాక్", "లాగ్ అవుట్", "పాత సందేశాలు",
    "వాయిస్ కాల్", "వీడియో కాల్", "అంగీకరించు", "మ్యూట్", "కాల్ ముగించు", "నిరోధించు", "వాయిస్ నోట్", "ఫైల్ ఎంచుకో", "పంపు",
];
const UR: &[&str] = &[
    "ان لاک", "نئی گفتگو", "میری دعوت", "شامل ہوں", "رابطے", "مسدود", "ترسیل", "لاگ",
    "نجی کلید", "بیک اپ", "پاس ورڈ", "تعارف", "زبان", "لاک", "لاگ آؤٹ", "پرانی پیغامات",
    "وائس کال", "ویڈیو کال", "قبول", "خاموش", "کال ختم", "بلاک", "وائس نوٹ", "فائل منتخب کریں", "بھیجیں",
];
const BHO: &[&str] = &[
    "अनलॉक", "नव बात", "हमार निमंत्रण", "जुड़ीं", "संपर्क", "ब्लॉक", "डिलीवरी", "लॉग",
    "निजी चाभी", "बैकअप", "पासवर्ड", "बारे", "भाषा", "लॉक", "लॉग आउट", "पुरान संदेश",
    "आवाज काल", "वीडियो काल", "मानीं", "म्यूट", "काल बंद", "ब्लॉक", "आवाज नोट", "फाइल चुनीं", "भेजीं",
];
const HNE: &[&str] = &[
    "अनलॉक", "नवा बात", "मोर नेवता", "जुड़ो", "संपर्क", "ब्लॉक", "डिलीवरी", "लॉग",
    "निजी चाबी", "बैकअप", "पासवर्ड", "बारे", "भाषा", "लॉक", "लॉग आउट", "पुराना संदेश",
    "आवाज काल", "वीडियो काल", "मान", "म्यूट", "काल बंद", "ब्लॉक", "आवाज नोट", "फाइल चुन", "भेज",
];
const LUS: &[&str] = &[
    "Hawng", "Thuchah thar", "Ka sawmna", "Tel rawh", "Contact", "Kharkhup", "Thlen", "Log",
    "Private key", "Backup", "Password", "Chungchang", "Ṭawng", "Khar", "Chhuak", "Thuchah hlui",
    "Aw call", "Video call", "Pawm", "Mute", "Call tit", "Kharkhup", "Aw message", "File thlang", "Thawn",
];
const KHA: &[&str] = &[
    "Plie", "Jingiathuhkhana thymmai", "Ka jingkhot", "Iasoh", "Kontak", "Lah pynkhang", "Pynphai", "Log",
    "Private key", "Backup", "Password", "Shaphang", "Ktien", "Khang", "Mih", "Jingthoh baroh",
    "Call aw", "Call video", "Pdiang", "Mute", "Kut call", "Pynkhang", "Jingthoh aw", "Jied file", "Phah",
];
const GRT: &[&str] = &[
    "Oka", "Gital chat", "Angni invitation", "Bakrimbo", "Contact", "Block", "Poraiani", "Log",
    "Private key", "Backup", "Password", "Gimin", "Ku·sik", "Khak", "Ong·kat", "Da·al message",
    "Voice call", "Video call", "Ra·chak", "Mute", "Call bon", "Block", "Voice note", "File se", "Watat",
];
const TRP: &[&str] = &[
    "Unlock", "Chat koktwma", "Ani invitation", "Baksa", "Contact", "Block", "Delivery", "Log",
    "Private key", "Backup", "Password", "Bising", "Kok", "Lock", "Logout", "Message oswi",
    "Voice call", "Video call", "Nai", "Mute", "Call thangkha", "Block", "Voice note", "File sal", "Thang",
];
const LEP: &[&str] = &[
    "Unlock", "Chat thom", "Go invitation", "Nyot", "Contact", "Block", "Delivery", "Log",
    "Private key", "Backup", "Password", "Tham", "Róng", "Lock", "Logout", "Message nyim",
    "Voice call", "Video call", "Nyen", "Mute", "Call chót", "Block", "Voice note", "File lyo", "Lyo",
];

/// Worldwide language catalogue (picker). UI chrome expands over time; missing
/// copy falls back to English while the native name still renders via WorldFonts.
pub const LANGUAGES: &[LanguageInfo] = &[
    LanguageInfo { code: "en", name_en: "English", native_name: "English", region: "World" },
    // India: Eighth Schedule + widely used
    LanguageInfo { code: "hi", name_en: "Hindi", native_name: "हिन्दी", region: "India" },
    LanguageInfo { code: "bn", name_en: "Bengali", native_name: "বাংলা", region: "India" },
    LanguageInfo { code: "te", name_en: "Telugu", native_name: "తెలుగు", region: "India" },
    LanguageInfo { code: "mr", name_en: "Marathi", native_name: "मराठी", region: "India" },
    LanguageInfo { code: "ta", name_en: "Tamil", native_name: "தமிழ்", region: "India" },
    LanguageInfo { code: "ur", name_en: "Urdu", native_name: "اردو", region: "India" },
    LanguageInfo { code: "gu", name_en: "Gujarati", native_name: "ગુજરાતી", region: "India" },
    LanguageInfo { code: "kn", name_en: "Kannada", native_name: "ಕನ್ನಡ", region: "India" },
    LanguageInfo { code: "ml", name_en: "Malayalam", native_name: "മലയാളം", region: "India" },
    LanguageInfo { code: "or", name_en: "Odia", native_name: "ଓଡ଼ିଆ", region: "India" },
    LanguageInfo { code: "pa", name_en: "Punjabi", native_name: "ਪੰਜਾਬੀ", region: "India" },
    LanguageInfo { code: "as", name_en: "Assamese", native_name: "অসমীয়া", region: "India" },
    LanguageInfo { code: "mai", name_en: "Maithili", native_name: "मैथिली", region: "India" },
    LanguageInfo { code: "sa", name_en: "Sanskrit", native_name: "संस्कृतम्", region: "India" },
    LanguageInfo { code: "ks", name_en: "Kashmiri", native_name: "كٲشُر", region: "India" },
    LanguageInfo { code: "ne", name_en: "Nepali", native_name: "नेपाली", region: "India" },
    LanguageInfo { code: "sd", name_en: "Sindhi", native_name: "سنڌي", region: "India" },
    LanguageInfo { code: "kok", name_en: "Konkani", native_name: "कोंकणी", region: "India" },
    LanguageInfo { code: "doi", name_en: "Dogri", native_name: "डोगरी", region: "India" },
    LanguageInfo { code: "mni", name_en: "Manipuri", native_name: "ꯃꯤꯇꯩꯂꯣꯟ", region: "India" },
    LanguageInfo { code: "sat", name_en: "Santali", native_name: "ᱥᱟᱱᱛᱟᱲᱤ", region: "India" },
    LanguageInfo { code: "brx", name_en: "Bodo", native_name: "बर'", region: "India" },
    // India: additional state / widely spoken
    LanguageInfo { code: "bho", name_en: "Bhojpuri", native_name: "भोजपुरी", region: "India" },
    LanguageInfo { code: "awa", name_en: "Awadhi", native_name: "अवधी", region: "India" },
    LanguageInfo { code: "mag", name_en: "Magahi", native_name: "मगही", region: "India" },
    LanguageInfo { code: "hne", name_en: "Chhattisgarhi", native_name: "छत्तीसगढ़ी", region: "India" },
    LanguageInfo { code: "raj", name_en: "Rajasthani", native_name: "राजस्थानी", region: "India" },
    LanguageInfo { code: "tcy", name_en: "Tulu", native_name: "ತುಳು", region: "India" },
    LanguageInfo { code: "trp", name_en: "Kokborok", native_name: "ককবরক", region: "India" },
    LanguageInfo { code: "lus", name_en: "Mizo", native_name: "Mizo ṭawng", region: "India" },
    LanguageInfo { code: "kha", name_en: "Khasi", native_name: "Khasi", region: "India" },
    LanguageInfo { code: "grt", name_en: "Garo", native_name: "A·chik", region: "India" },
    LanguageInfo { code: "lep", name_en: "Lepcha", native_name: "Lepcha", region: "India" },
    LanguageInfo { code: "gom", name_en: "Goan Konkani", native_name: "गोंयची कोंकणी", region: "India" },
    LanguageInfo { code: "bo", name_en: "Tibetan", native_name: "བོད་སྐད་", region: "India" },
    // South Asia neighbours
    LanguageInfo { code: "si", name_en: "Sinhala", native_name: "සිංහල", region: "South Asia" },
    LanguageInfo { code: "dv", name_en: "Dhivehi", native_name: "ދިވެހި", region: "South Asia" },
    LanguageInfo { code: "ps", name_en: "Pashto", native_name: "پښتو", region: "South Asia" },
    LanguageInfo { code: "fa", name_en: "Persian", native_name: "فارسی", region: "Middle East" },
    // East / Southeast Asia
    LanguageInfo { code: "zh", name_en: "Chinese (Simplified)", native_name: "中文", region: "East Asia" },
    LanguageInfo { code: "zh-Hant", name_en: "Chinese (Traditional)", native_name: "繁體中文", region: "East Asia" },
    LanguageInfo { code: "yue", name_en: "Cantonese", native_name: "粵語", region: "East Asia" },
    LanguageInfo { code: "ja", name_en: "Japanese", native_name: "日本語", region: "East Asia" },
    LanguageInfo { code: "ko", name_en: "Korean", native_name: "한국어", region: "East Asia" },
    LanguageInfo { code: "vi", name_en: "Vietnamese", native_name: "Tiếng Việt", region: "Southeast Asia" },
    LanguageInfo { code: "th", name_en: "Thai", native_name: "ไทย", region: "Southeast Asia" },
    LanguageInfo { code: "id", name_en: "Indonesian", native_name: "Bahasa Indonesia", region: "Southeast Asia" },
    LanguageInfo { code: "ms", name_en: "Malay", native_name: "Bahasa Melayu", region: "Southeast Asia" },
    LanguageInfo { code: "fil", name_en: "Filipino", native_name: "Filipino", region: "Southeast Asia" },
    LanguageInfo { code: "my", name_en: "Burmese", native_name: "မြန်မာ", region: "Southeast Asia" },
    LanguageInfo { code: "km", name_en: "Khmer", native_name: "ខ្មែរ", region: "Southeast Asia" },
    LanguageInfo { code: "lo", name_en: "Lao", native_name: "ລາວ", region: "Southeast Asia" },
    // Middle East / Central Asia
    LanguageInfo { code: "ar", name_en: "Arabic", native_name: "العربية", region: "Middle East" },
    LanguageInfo { code: "he", name_en: "Hebrew", native_name: "עברית", region: "Middle East" },
    LanguageInfo { code: "tr", name_en: "Turkish", native_name: "Türkçe", region: "Middle East" },
    LanguageInfo { code: "az", name_en: "Azerbaijani", native_name: "Azərbaycan", region: "Central Asia" },
    LanguageInfo { code: "kk", name_en: "Kazakh", native_name: "Қазақша", region: "Central Asia" },
    LanguageInfo { code: "uz", name_en: "Uzbek", native_name: "Oʻzbekcha", region: "Central Asia" },
    LanguageInfo { code: "ky", name_en: "Kyrgyz", native_name: "Кыргызча", region: "Central Asia" },
    LanguageInfo { code: "tg", name_en: "Tajik", native_name: "Тоҷикӣ", region: "Central Asia" },
    LanguageInfo { code: "tk", name_en: "Turkmen", native_name: "Türkmençe", region: "Central Asia" },
    LanguageInfo { code: "mn", name_en: "Mongolian", native_name: "Монгол", region: "East Asia" },
    // Europe
    LanguageInfo { code: "es", name_en: "Spanish", native_name: "Español", region: "Europe" },
    LanguageInfo { code: "fr", name_en: "French", native_name: "Français", region: "Europe" },
    LanguageInfo { code: "pt", name_en: "Portuguese", native_name: "Português", region: "Europe" },
    LanguageInfo { code: "de", name_en: "German", native_name: "Deutsch", region: "Europe" },
    LanguageInfo { code: "it", name_en: "Italian", native_name: "Italiano", region: "Europe" },
    LanguageInfo { code: "ru", name_en: "Russian", native_name: "Русский", region: "Europe" },
    LanguageInfo { code: "uk", name_en: "Ukrainian", native_name: "Українська", region: "Europe" },
    LanguageInfo { code: "pl", name_en: "Polish", native_name: "Polski", region: "Europe" },
    LanguageInfo { code: "nl", name_en: "Dutch", native_name: "Nederlands", region: "Europe" },
    LanguageInfo { code: "ro", name_en: "Romanian", native_name: "Română", region: "Europe" },
    LanguageInfo { code: "hu", name_en: "Hungarian", native_name: "Magyar", region: "Europe" },
    LanguageInfo { code: "cs", name_en: "Czech", native_name: "Čeština", region: "Europe" },
    LanguageInfo { code: "sk", name_en: "Slovak", native_name: "Slovenčina", region: "Europe" },
    LanguageInfo { code: "bg", name_en: "Bulgarian", native_name: "Български", region: "Europe" },
    LanguageInfo { code: "sr", name_en: "Serbian", native_name: "Српски", region: "Europe" },
    LanguageInfo { code: "hr", name_en: "Croatian", native_name: "Hrvatski", region: "Europe" },
    LanguageInfo { code: "bs", name_en: "Bosnian", native_name: "Bosanski", region: "Europe" },
    LanguageInfo { code: "sl", name_en: "Slovenian", native_name: "Slovenščina", region: "Europe" },
    LanguageInfo { code: "el", name_en: "Greek", native_name: "Ελληνικά", region: "Europe" },
    LanguageInfo { code: "sv", name_en: "Swedish", native_name: "Svenska", region: "Europe" },
    LanguageInfo { code: "no", name_en: "Norwegian", native_name: "Norsk", region: "Europe" },
    LanguageInfo { code: "da", name_en: "Danish", native_name: "Dansk", region: "Europe" },
    LanguageInfo { code: "fi", name_en: "Finnish", native_name: "Suomi", region: "Europe" },
    LanguageInfo { code: "is", name_en: "Icelandic", native_name: "Íslenska", region: "Europe" },
    LanguageInfo { code: "ga", name_en: "Irish", native_name: "Gaeilge", region: "Europe" },
    LanguageInfo { code: "cy", name_en: "Welsh", native_name: "Cymraeg", region: "Europe" },
    LanguageInfo { code: "ca", name_en: "Catalan", native_name: "Català", region: "Europe" },
    LanguageInfo { code: "eu", name_en: "Basque", native_name: "Euskara", region: "Europe" },
    LanguageInfo { code: "gl", name_en: "Galician", native_name: "Galego", region: "Europe" },
    LanguageInfo { code: "lt", name_en: "Lithuanian", native_name: "Lietuvių", region: "Europe" },
    LanguageInfo { code: "lv", name_en: "Latvian", native_name: "Latviešu", region: "Europe" },
    LanguageInfo { code: "et", name_en: "Estonian", native_name: "Eesti", region: "Europe" },
    LanguageInfo { code: "mt", name_en: "Maltese", native_name: "Malti", region: "Europe" },
    LanguageInfo { code: "sq", name_en: "Albanian", native_name: "Shqip", region: "Europe" },
    LanguageInfo { code: "mk", name_en: "Macedonian", native_name: "Македонски", region: "Europe" },
    LanguageInfo { code: "be", name_en: "Belarusian", native_name: "Беларуская", region: "Europe" },
    LanguageInfo { code: "ka", name_en: "Georgian", native_name: "ქართული", region: "Europe" },
    LanguageInfo { code: "hy", name_en: "Armenian", native_name: "Հայերեն", region: "Europe" },
    // Africa
    LanguageInfo { code: "sw", name_en: "Swahili", native_name: "Kiswahili", region: "Africa" },
    LanguageInfo { code: "am", name_en: "Amharic", native_name: "አማርኛ", region: "Africa" },
    LanguageInfo { code: "ha", name_en: "Hausa", native_name: "Hausa", region: "Africa" },
    LanguageInfo { code: "yo", name_en: "Yoruba", native_name: "Yorùbá", region: "Africa" },
    LanguageInfo { code: "ig", name_en: "Igbo", native_name: "Igbo", region: "Africa" },
    LanguageInfo { code: "zu", name_en: "Zulu", native_name: "isiZulu", region: "Africa" },
    LanguageInfo { code: "xh", name_en: "Xhosa", native_name: "isiXhosa", region: "Africa" },
    LanguageInfo { code: "af", name_en: "Afrikaans", native_name: "Afrikaans", region: "Africa" },
    LanguageInfo { code: "so", name_en: "Somali", native_name: "Soomaali", region: "Africa" },
    LanguageInfo { code: "rw", name_en: "Kinyarwanda", native_name: "Ikinyarwanda", region: "Africa" },
    LanguageInfo { code: "mg", name_en: "Malagasy", native_name: "Malagasy", region: "Africa" },
    // Americas
    LanguageInfo { code: "pt-BR", name_en: "Portuguese (Brazil)", native_name: "Português (Brasil)", region: "Americas" },
    LanguageInfo { code: "es-MX", name_en: "Spanish (Mexico)", native_name: "Español (México)", region: "Americas" },
    LanguageInfo { code: "ht", name_en: "Haitian Creole", native_name: "Kreyòl ayisyen", region: "Americas" },
    LanguageInfo { code: "qu", name_en: "Quechua", native_name: "Runa Simi", region: "Americas" },
    LanguageInfo { code: "gn", name_en: "Guarani", native_name: "Avañe'ẽ", region: "Americas" },
];

/// Longer sheet copy. Missing translations use English.
pub fn phrase(lang: &str, id: &str) -> String {
    let lang = ui_lang(lang);
    extra(lang, id)
        .or_else(|| extra("en", id))
        .unwrap_or(id)
        .to_string()
}

fn extra(lang: &str, id: &str) -> Option<&'static str> {
    if let Some(row) = crate::phrases::row(lang) {
        if let Some(v) = row.iter().find(|(k, _)| *k == id).map(|(_, v)| *v) {
            return Some(v);
        }
    }
    let row: &[(&str, &str)] = match lang {
        "es" => &[
            ("remove", "Quitar"),
            ("share", "Compartir invitación"),
            ("paste", "Pegar invitación"),
            ("paste_body", "Pega un enlace ghalbol:// o https://ghalbol.com/connect/."),
            ("add", "Añadir por clave pública"),
            ("add_body", "El invitado guarda la clave del anfitrión. El anfitrión no necesita la tuya antes."),
            ("contacts", "Contactos"),
            ("none", "Aún no hay contactos"),
            ("blocked", "Bloqueados"),
            ("delivery", "Entrega"),
            ("log", "Registro"),
            ("key", "Clave privada"),
            ("key_body", "Escribe la contraseña de la app y luego muestra la clave. Hazlo solo en un lugar privado."),
            ("backup", "Exportar copia"),
            ("backup_body", "Guarda el almacén cifrado. Hace falta la contraseña de la app para abrirlo en otro dispositivo."),
            ("password", "Cambiar contraseña"),
            ("password_body", "La contraseña actual va en el enlace. La nueva, en el nombre. Las copias antiguas siguen con la contraseña antigua."),
            ("about", "Acerca de Ghal Bol"),
            ("save_backup", "Guarda tu copia"),
            ("save_backup_body", "Guarda la copia cifrada antes de chatear. Esta identidad no se recupera sin ella y tu contraseña."),
            ("back", "Atrás"),
            ("join_go", "Unirse"),
            ("add_go", "Añadir contacto"),
            ("show_key", "Mostrar clave"),
            ("save", "Guardar copia"),
            ("change", "Cambiar contraseña"),
            ("save_status", "Guardar estado"),
            ("speaker", "Altavoz"),
            ("speaker_off", "Altavoz apagado"),
            ("scan_qr", "Escanear QR"),
            ("qr_picture", "Imagen QR"),
        ],
        "hi" => &[
            ("remove", "हटाएँ"),
            ("share", "निमंत्रण बाँटें"),
            ("paste", "निमंत्रण चिपकाएँ"),
            ("paste_body", "ghalbol:// या https://ghalbol.com/connect/ लिंक चिपकाएँ।"),
            ("add", "सार्वजनिक कुंजी से जोड़ें"),
            ("add_body", "अतिथि मेज़बान की सार्वजनिक कुंजी रखता है। मेज़बान को पहले आपकी कुंजी की ज़रूरत नहीं।"),
            ("contacts", "संपर्क"),
            ("none", "अभी कोई संपर्क नहीं"),
            ("blocked", "अवरुद्ध"),
            ("delivery", "वितरण"),
            ("log", "ऐप लॉग"),
            ("key", "निजी कुंजी"),
            ("key_body", "ऐप पासवर्ड लिखें, फिर कुंजी दिखाएँ। यह केवल निजी जगह पर करें।"),
            ("backup", "बैकअप निर्यात"),
            ("backup_body", "एन्क्रिप्टेड कीस्टोर सहेजता है। दूसरे उपकरण पर खोलने के लिए ऐप पासवर्ड चाहिए।"),
            ("password", "पासवर्ड बदलें"),
            ("password_body", "वर्तमान पासवर्ड निमंत्रण में लिखें। नया नाम में। पुरानी प्रतियाँ पुराना पासवर्ड रखती हैं।"),
            ("about", "गाल बोल के बारे में"),
            ("save_backup", "बैकअप सहेजें"),
            ("save_backup_body", "चैट से पहले एन्क्रिप्टेड बैकअप सहेजें। बिना इसके और पासवर्ड के यह पहचान वापस नहीं आती।"),
            ("back", "वापस"),
            ("join_go", "जुड़ें"),
            ("add_go", "संपर्क जोड़ें"),
            ("show_key", "कुंजी दिखाएँ"),
            ("save", "बैकअप सहेजें"),
            ("change", "पासवर्ड बदलें"),
            ("save_status", "स्थिति सहेजें"),
            ("speaker", "स्पीकर"),
            ("speaker_off", "स्पीकर बंद"),
            ("scan_qr", "क्यूआर स्कैन"),
            ("qr_picture", "क्यूआर चित्र"),
        ],
        "fr" => &[
            ("remove", "Retirer"),
            ("share", "Partager l’invitation"),
            ("paste", "Coller l’invitation"),
            ("paste_body", "Collez un lien ghalbol:// ou https://ghalbol.com/connect/."),
            ("add", "Ajouter par clé publique"),
            ("add_body", "L’invité enregistre la clé publique de l’hôte. L’hôte n’a pas besoin de la vôtre avant."),
            ("contacts", "Contacts"),
            ("none", "Aucun contact"),
            ("blocked", "Bloqués"),
            ("blocked_empty", "Aucun contact bloqué. Collez une clé publique pour bloquer quelqu’un déjà dans la liste."),
            ("delivery", "Stockage de remise"),
            ("log", "Journal"),
            ("key", "Clé privée"),
            ("key_body", "Saisissez le mot de passe de l’app, puis affichez la clé. Faites-le seulement dans un endroit privé."),
            ("backup", "Exporter la sauvegarde"),
            ("backup_body", "Enregistre le coffre chiffré. Le mot de passe de l’app reste nécessaire sur un autre appareil."),
            ("password", "Changer le mot de passe"),
            ("password_body", "Mot de passe actuel dans le lien. Nouveau mot de passe dans le nom. Les anciennes sauvegardes gardent l’ancien mot de passe."),
            ("about", "À propos de Ghal Bol"),
            ("about_body", "Messagerie chiffrée de bout en bout. Chaque appareil garde sa propre conversation."),
            ("no_status", "Aucun statut"),
            ("save_backup", "Enregistrez la sauvegarde"),
            ("save_backup_body", "Enregistrez la sauvegarde chiffrée avant de discuter. Cette identité ne se récupère pas sans elle et votre mot de passe."),
            ("back", "Retour"),
            ("join_go", "Rejoindre"),
            ("add_go", "Ajouter le contact"),
            ("add_short", "Ajouter"),
            ("toggle", "Bloquer ou débloquer"),
            ("refresh", "Actualiser"),
            ("show_key", "Afficher la clé"),
            ("save", "Enregistrer la sauvegarde"),
            ("change", "Changer le mot de passe"),
            ("save_status", "Enregistrer le statut"),
            ("must_save", "Enregistrez la sauvegarde avant de continuer."),
            ("enter_password", "Saisissez d’abord le mot de passe de cette sauvegarde."),
            ("choose_file", "Choisissez un fichier de sauvegarde."),
            ("added", "Contact ajouté"),
            ("saved_prefix", "Enregistré"),
            ("speaker", "Haut-parleur"),
            ("speaker_off", "Haut-parleur coupé"),
            ("scan_qr", "Scanner le QR"),
            ("qr_picture", "Image QR"),
        ],
        "de" => &[
            ("remove", "Entfernen"),
            ("share", "Einladung teilen"),
            ("paste", "Einladung einfügen"),
            ("paste_body", "Fügen Sie einen ghalbol://- oder https://ghalbol.com/connect/-Link ein."),
            ("add", "Mit öffentlichem Schlüssel hinzufügen"),
            ("add_body", "Der Gast speichert den öffentlichen Schlüssel des Gastgebers. Der Gastgeber braucht Ihren nicht zuerst."),
            ("contacts", "Kontakte"),
            ("none", "Noch keine Kontakte"),
            ("blocked", "Blockiert"),
            ("blocked_empty", "Keine blockierten Kontakte. Fügen Sie einen öffentlichen Schlüssel ein, um jemanden aus der Liste zu blockieren."),
            ("delivery", "Zustellspeicher"),
            ("log", "App-Protokoll"),
            ("key", "Privater Schlüssel"),
            ("key_body", "Geben Sie das App-Passwort ein und zeigen Sie dann den Schlüssel. Nur an einem privaten Ort."),
            ("backup", "Sicherung exportieren"),
            ("backup_body", "Speichert den verschlüsselten Schlüsselspeicher. Zum Öffnen auf einem anderen Gerät brauchen Sie das App-Passwort."),
            ("password", "Passwort ändern"),
            ("password_body", "Aktuelles Passwort ins Einladungsfeld. Neues Passwort ins Namensfeld. Ältere Sicherungen behalten das alte Passwort."),
            ("about", "Über Ghal Bol"),
            ("about_body", "Ende-zu-Ende-verschlüsselter Messenger. Jedes Gerät behält sein eigenes Protokoll."),
            ("no_status", "Kein Status"),
            ("save_backup", "Sicherung speichern"),
            ("save_backup_body", "Speichern Sie die verschlüsselte Sicherung vor dem Chat. Diese Identität ist ohne sie und das Passwort nicht wiederherstellbar."),
            ("back", "Zurück"),
            ("join_go", "Beitreten"),
            ("add_go", "Kontakt hinzufügen"),
            ("add_short", "Hinzufügen"),
            ("toggle", "Blockierung umschalten"),
            ("refresh", "Aktualisieren"),
            ("show_key", "Schlüssel zeigen"),
            ("save", "Sicherung speichern"),
            ("change", "Passwort ändern"),
            ("save_status", "Status speichern"),
            ("must_save", "Speichern Sie die Sicherung, bevor Sie fortfahren."),
            ("enter_password", "Geben Sie zuerst das Passwort dieser Sicherung ein."),
            ("choose_file", "Wählen Sie eine Sicherungsdatei."),
            ("added", "Kontakt hinzugefügt"),
            ("saved_prefix", "Gespeichert"),
            ("speaker", "Lautsprecher"),
            ("speaker_off", "Lautsprecher aus"),
            ("scan_qr", "QR scannen"),
            ("qr_picture", "QR-Bild"),
        ],
        "zh" => &[
            ("remove", "移除"),
            ("share", "分享邀请"),
            ("paste", "粘贴邀请"),
            ("paste_body", "粘贴 ghalbol:// 或 https://ghalbol.com/connect/ 链接。"),
            ("add", "用公钥添加"),
            ("add_body", "客人保存主人的公钥。主人不需要先拥有你的公钥。"),
            ("contacts", "联系人"),
            ("none", "还没有联系人"),
            ("blocked", "已屏蔽"),
            ("blocked_empty", "没有已屏蔽的联系人。粘贴公钥可屏蔽名单中的人。"),
            ("delivery", "投递存储"),
            ("log", "应用日志"),
            ("key", "私钥"),
            ("key_body", "输入应用密码后再显示密钥。请只在私下进行。"),
            ("backup", "导出备份"),
            ("backup_body", "保存加密的密钥库。在另一台设备上打开仍需要应用密码。"),
            ("password", "更改密码"),
            ("password_body", "当前密码填在邀请栏。新密码填在名称栏。旧备份仍使用旧密码。"),
            ("about", "关于 Ghal Bol"),
            ("about_body", "端到端加密的通讯。每台设备只保存自己的聊天记录。"),
            ("no_status", "未设置状态"),
            ("save_backup", "请保存备份"),
            ("save_backup_body", "开始聊天前请保存加密备份。没有备份和应用密码就无法恢复这个身份。"),
            ("back", "返回"),
            ("join_go", "加入"),
            ("add_go", "添加联系人"),
            ("add_short", "添加"),
            ("toggle", "切换屏蔽"),
            ("refresh", "刷新"),
            ("show_key", "显示密钥"),
            ("save", "保存备份"),
            ("change", "更改密码"),
            ("save_status", "保存状态"),
            ("must_save", "请先保存备份再继续。"),
            ("enter_password", "请先输入这份备份的应用密码。"),
            ("choose_file", "请选择密钥库备份文件。"),
            ("added", "已添加联系人"),
            ("saved_prefix", "已保存"),
            ("speaker", "扬声器"),
            ("speaker_off", "扬声器已关"),
            ("scan_qr", "扫描二维码"),
            ("qr_picture", "二维码图片"),
        ],
        "ar" => &[
            ("remove", "إزالة"),
            ("share", "مشاركة الدعوة"),
            ("paste", "لصق الدعوة"),
            ("paste_body", "الصق رابط ghalbol:// أو https://ghalbol.com/connect/."),
            ("add", "إضافة بالمفتاح العام"),
            ("add_body", "الضيف يحفظ المفتاح العام للمضيف. المضيف لا يحتاج مفتاحك أولاً."),
            ("contacts", "جهات الاتصال"),
            ("none", "لا جهات اتصال بعد"),
            ("blocked", "المحظورون"),
            ("blocked_empty", "لا جهات محظورة. الصق مفتاحًا عامًا لحظر شخص موجود في القائمة."),
            ("delivery", "تخزين التسليم"),
            ("log", "سجل التطبيق"),
            ("key", "المفتاح الخاص"),
            ("key_body", "أدخل كلمة مرور التطبيق ثم أظهر المفتاح. افعل ذلك في مكان خاص فقط."),
            ("backup", "تصدير النسخة"),
            ("backup_body", "يحفظ مخزن المفاتيح المشفّر. ما زلت تحتاج كلمة مرور التطبيق لفتحه على جهاز آخر."),
            ("password", "تغيير كلمة المرور"),
            ("password_body", "كلمة المرور الحالية في حقل الدعوة. الجديدة في حقل الاسم. النسخ القديمة تبقى على كلمة المرور القديمة."),
            ("about", "حول غال بول"),
            ("about_body", "مراسلة مشفّرة طرفًا لطرف. كل جهاز يحتفظ بنسخته من المحادثة."),
            ("no_status", "لا حالة"),
            ("save_backup", "احفظ النسخة"),
            ("save_backup_body", "احفظ النسخة المشفّرة قبل الدردشة. لا يمكن استرجاع هذه الهوية بدونها وبدون كلمة المرور."),
            ("back", "رجوع"),
            ("join_go", "انضمام"),
            ("add_go", "إضافة جهة"),
            ("add_short", "إضافة"),
            ("toggle", "تبديل الحظر"),
            ("refresh", "تحديث"),
            ("show_key", "إظهار المفتاح"),
            ("save", "حفظ النسخة"),
            ("change", "تغيير كلمة المرور"),
            ("save_status", "حفظ الحالة"),
            ("must_save", "احفظ النسخة قبل المتابعة."),
            ("enter_password", "أدخل أولاً كلمة مرور هذه النسخة."),
            ("choose_file", "اختر ملف نسخة مخزن المفاتيح."),
            ("added", "تمت إضافة الجهة"),
            ("saved_prefix", "تم الحفظ"),
            ("speaker", "مكبّر الصوت"),
            ("speaker_off", "مكبّر الصوت متوقف"),
            ("scan_qr", "مسح رمز QR"),
            ("qr_picture", "صورة QR"),
        ],
        _ => &[
            ("remove", "Remove"),
            ("share", "Share invitation"),
            ("paste", "Paste invitation link"),
            ("paste_body", "Paste a ghalbol:// or https://ghalbol.com/connect/ link."),
            ("add", "Add contact by public key"),
            ("add_body", "The guest stores the host public key. The host does not need yours first."),
            ("contacts", "Contacts"),
            ("none", "No contacts yet"),
            ("blocked", "Blocked contacts"),
            ("blocked_empty", "No blocked contacts. Paste a public key below to block someone already in your list."),
            ("delivery", "Delivery storage"),
            ("log", "App log"),
            ("key", "Private key"),
            ("key_body", "Enter your app password below, then show the key. Do this only in a private place."),
            ("backup", "Export backup"),
            ("backup_body", "Saves the encrypted keystore. You still need your app password to unlock it on another device."),
            ("password", "Change password"),
            ("password_body", "Type the current app password in Invitation link. Type the new password in Display name and again in Public key. Older backups still use the old password, so export a new backup after this."),
            ("about", "About Ghal Bol"),
            ("about_body", "End-to-end encrypted messenger. Each device keeps its own transcript."),
            ("no_status", "No status set"),
            ("save_backup", "Save your backup"),
            ("save_backup_body", "Save the encrypted backup before using chat. This identity cannot be recovered without it and your app password."),
            ("back", "Back"),
            ("join_go", "Join"),
            ("add_go", "Add contact"),
            ("add_short", "Add"),
            ("toggle", "Toggle block"),
            ("refresh", "Refresh"),
            ("show_key", "Show key"),
            ("save", "Save backup"),
            ("change", "Change password"),
            ("save_status", "Save status"),
            ("speaker", "Speaker"),
            ("speaker_off", "Speaker off"),
            ("scan_qr", "Scan QR"),
            ("qr_picture", "QR picture"),
            ("must_save", "Save the backup before continuing."),
            ("enter_password", "Enter the app password for this backup first."),
            ("choose_file", "Choose a keystore backup file."),
            ("added", "Contact added"),
            ("saved_prefix", "Saved"),
        ],
    };
    row.iter().find(|(k, _)| *k == id).map(|(_, v)| *v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_matches_the_key_count() {
        let n = Key::Send as usize + 1;
        assert_eq!(EN.len(), n + 2);
        for lang in LANGUAGES {
            let rows = catalog(lang.code);
            assert!(
                rows.len() == n || rows.len() == EN.len(),
                "{} has {} strings",
                lang.code,
                rows.len()
            );
        }
    }

    /// Languages that ship a full chrome + sheet table (not English fallback).
    const TRANSLATED: &[&str] = &[
        "es", "fr", "pt", "de", "it", "nl", "pl", "uk", "ru", "ar", "fa", "tr", "zh", "ja", "ko",
        "vi", "th", "id", "sw", "hi", "bn", "as", "brx", "doi", "gu", "kn", "ks", "kok", "ml",
        "mni", "mr", "mai", "ne", "or", "pa", "sa", "sat", "sd", "ta", "te", "ur", "bho", "hne",
        "lus", "kha", "grt", "trp", "lep",
    ];

    #[test]
    fn translated_languages_have_sheet_copy() {
        let ids = [
            "share", "paste_body", "add_body", "save_backup_body", "password_body", "speaker",
            "scan_qr", "qr_picture",
        ];
        for code in TRANSLATED {
            for id in ids {
                let local = phrase(code, id);
                assert_ne!(local, id, "{code} missing {id}");
                assert_ne!(local, phrase("en", id), "{code} still English for {id}");
            }
        }
    }

    #[test]
    fn phrase_falls_back_to_english() {
        assert_eq!(phrase("hi", "share"), "निमंत्रण बाँटें");
        assert_eq!(phrase("zz", "share"), "Share invitation");
        assert_eq!(phrase("am", "share"), "Share invitation");
        assert_eq!(phrase("zh-Hant", "share"), phrase("zh", "share"));
        assert_eq!(phrase("", "missing-id"), "missing-id");
    }

    #[test]
    fn resolve_code_maps_common_tags() {
        assert_eq!(resolve_code("en_US.UTF-8"), "en");
        assert_eq!(resolve_code("hi_IN"), "hi");
        assert_eq!(resolve_code("zh_TW"), "zh-Hant");
        assert_eq!(resolve_code("pt_BR"), "pt-BR");
        assert_eq!(resolve_code("xx_YY"), "en");
    }

    #[test]
    fn catalogue_has_unique_codes() {
        let mut seen = std::collections::HashSet::new();
        for lang in LANGUAGES {
            assert!(seen.insert(lang.code), "duplicate {}", lang.code);
            assert!(!lang.native_name.is_empty());
            assert!(!lang.name_en.is_empty());
        }
        assert!(LANGUAGES.len() > 80);
    }

    #[test]
    fn system_code_is_a_known_language_or_english() {
        let code = system_code();
        assert!(language_by_code(&code).is_some() || code == "en");
    }
}
