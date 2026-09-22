//! The four languages Agentty is in, for a plugin's own strings.
//!
//! Agentty's panel, its menus and its messages are in English, Korean, Japanese and Chinese, and
//! a plugin that is only in one of them is the odd one out in the window. A plugin gives each
//! string four ways and asks for the one the user reads:
//!
//! ```ignore
//! use agentty_plugin::text::t;
//!
//! let lang = host.language();
//! host.set_panel(ui::column(vec![
//!     ui::button("send", t(lang, ["Send", "보내기", "送信", "发送"])),
//! ]));
//! ```
//!
//! The language arrives with `initialize` and again whenever the user changes it, so a panel
//! drawn after that is in the right one. Prompts a plugin sends to an agent are a different
//! thing: those stay in English and tell the agent which language to answer in.

/// A language Agentty is in. Anything else reads English.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Lang {
    #[default]
    En,
    Ko,
    Ja,
    Zh,
}

impl Lang {
    /// From the code Agentty sends (`en`, `ko`, `ja`, `zh`, and the longer forms of them).
    pub fn of(code: &str) -> Lang {
        match code.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase().as_str() {
            "ko" => Lang::Ko,
            "ja" => Lang::Ja,
            "zh" => Lang::Zh,
            _ => Lang::En,
        }
    }

    /// Where this language sits in `[en, ko, ja, zh]`.
    pub fn index(self) -> usize {
        match self {
            Lang::En => 0,
            Lang::Ko => 1,
            Lang::Ja => 2,
            Lang::Zh => 3,
        }
    }

    pub fn code(self) -> &'static str {
        ["en", "ko", "ja", "zh"][self.index()]
    }
}

/// One string in four languages, in the order `[English, 한국어, 日本語, 中文]`.
///
/// A language left empty falls back to English rather than showing nothing, so a plugin can add
/// a language at a time.
pub fn t(lang: Lang, all: [&'static str; 4]) -> &'static str {
    let chosen = all[lang.index()];
    if chosen.is_empty() {
        all[0]
    } else {
        chosen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_language_agentty_sends_is_the_one_that_is_used() {
        assert_eq!(Lang::of("ko"), Lang::Ko);
        assert_eq!(Lang::of("ja"), Lang::Ja);
        assert_eq!(Lang::of("zh"), Lang::Zh);
        assert_eq!(Lang::of("en"), Lang::En);
        // The longer forms, and anything Agentty does not have.
        assert_eq!(Lang::of("zh-Hans"), Lang::Zh);
        assert_eq!(Lang::of("ko_KR"), Lang::Ko);
        assert_eq!(Lang::of("de"), Lang::En);
        assert_eq!(Lang::of(""), Lang::En);
        assert_eq!(Lang::of("KO"), Lang::Ko);
    }

    #[test]
    fn a_string_is_picked_by_language_and_falls_back_to_english() {
        let send = ["Send", "보내기", "送信", "发送"];
        assert_eq!(t(Lang::En, send), "Send");
        assert_eq!(t(Lang::Ko, send), "보내기");
        assert_eq!(t(Lang::Zh, send), "发送");
        // A language not written yet reads English rather than nothing.
        assert_eq!(t(Lang::Ja, ["Send", "보내기", "", "发送"]), "Send");
    }

    #[test]
    fn every_language_has_a_code_and_a_place() {
        for lang in [Lang::En, Lang::Ko, Lang::Ja, Lang::Zh] {
            assert_eq!(Lang::of(lang.code()), lang);
            assert!(lang.index() < 4);
        }
    }
}
