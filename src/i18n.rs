use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Zh,
}

impl Lang {
    pub fn parse(code: &str) -> Option<Self> {
        let normalized = code.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "en" | "en-us" | "en-gb" => Some(Self::En),
            "zh" | "zh-cn" | "zh-hans" | "zh-hans-cn" => Some(Self::Zh),
            _ => None,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Zh => "zh-CN",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::Zh => "简体中文",
        }
    }

    pub fn t(self, en: &'static str, zh: &'static str) -> &'static str {
        match self {
            Self::En => en,
            Self::Zh => zh,
        }
    }
}

impl fmt::Display for Lang {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}
