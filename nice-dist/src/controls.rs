//! Saved circuit selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Variant {
    #[default]
    Bs808 = 0,
    Bs9 = 1,
    Sh1 = 2,
    Oh1 = 3,
    Cat = 4,
    Clone = 5,
}
impl Variant {
    pub const ALL: [Self; 6] = [
        Self::Bs808,
        Self::Bs9,
        Self::Oh1,
        Self::Sh1,
        Self::Cat,
        Self::Clone,
    ];
    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Bs9,
            2 => Self::Sh1,
            3 => Self::Oh1,
            4 => Self::Cat,
            5 => Self::Clone,
            _ => Self::Bs808,
        }
    }
    pub fn index(self) -> i32 {
        self as i32
    }
    pub fn from_number(number: u32) -> Self {
        match number {
            2 => Self::Bs9,
            3 => Self::Sh1,
            4 => Self::Oh1,
            5 => Self::Cat,
            6 => Self::Clone,
            _ => Self::Bs808,
        }
    }
    pub fn number(self) -> u32 {
        self.index() as u32 + 1
    }
    pub fn inspiration(self) -> &'static str {
        match self {
            Self::Bs808 => "Ibanez TS808",
            Self::Bs9 => "Ibanez TS9",
            Self::Sh1 => "BOSS SD-1",
            Self::Oh1 => "BOSS OD-1",
            Self::Cat => "Pro Co RAT",
            Self::Clone => "Klon Centaur",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Bs808 => "BS808",
            Self::Bs9 => "BS9",
            Self::Sh1 => "SH-1",
            Self::Oh1 => "OH-1",
            Self::Cat => "CAT",
            Self::Clone => "Nice Clone",
        }
    }
}

impl Variant {
    /// Accept historical host text without changing stored numeric model values.
    pub fn parse_label(text: &str) -> Option<Self> {
        let text = text.trim();
        Self::ALL
            .into_iter()
            .find(|mode| mode.label().eq_ignore_ascii_case(text))
            .or_else(|| {
                [
                    ("TS808", Self::Bs808),
                    ("TS9", Self::Bs9),
                    ("SD-1", Self::Sh1),
                    ("OD-1", Self::Oh1),
                    ("RAT", Self::Cat),
                ]
                .into_iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(text))
                .map(|(_, mode)| mode)
            })
    }
}
#[cfg(test)]
mod naming_tests {
    use super::*;
    #[test]
    fn renamed_models_keep_numeric_ids_and_accept_old_host_text() {
        for (old, label, index) in [
            ("TS808", "BS808", 0),
            ("TS9", "BS9", 1),
            ("SD-1", "SH-1", 2),
            ("OD-1", "OH-1", 3),
            ("RAT", "CAT", 4),
            ("Nice Clone", "Nice Clone", 5),
        ] {
            let model = Variant::from_index(index);
            assert_eq!(model.label(), label);
            assert_eq!(Variant::parse_label(old), Some(model));
            assert_eq!(Variant::parse_label(label), Some(model));
            assert_eq!(Variant::from_number(index as u32 + 1), model);
        }
        assert_eq!(Variant::parse_label("unknown"), None);
    }
}
